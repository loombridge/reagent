//! `match_sound` (R9.3): from a reference recording, a search over Reagent's
//! parameters for a patch that sounds like it.
//!
//! The reference is one held note. [`search`] reads its perception features
//! (`lspk_test::features`, R9.2), then looks for the parameters whose own
//! render of the same note has features nearest to them, by the kit's
//! [`distance`](features::distance). The search is gradient-free, because
//! the engine isn't differentiable and some of the parameters (filter type,
//! unison count) are choices:
//!
//! 1. a random batch over the whole [`Space`], to find where to look;
//! 2. an elitist evolution strategy from the best few of those: each batch
//!    of children is a parent plus Gaussian noise on some of the dimensions,
//!    and the noise shrinks batch by batch, so the search narrows.
//!
//! Every draw comes from the seed and the evaluation count alone, so a
//! search repeats exactly whatever the thread count; the candidates of a
//! batch render on all cores.
//!
//! What it can find is the [`Space`]: the two oscillators, Filter 1, Env 1,
//! Env 2 and LFO 1 with their routes, the ranges the dataset sampler
//! (R9.1) draws patches from. The sub, the noise, the other filter and the
//! effects are R9.8's.
//!
//! The commands run a search on a thread of their own, so nothing waits on
//! it: [`MATCH_SOUND`] starts one and returns at once, [`MATCH_STATUS`]
//! reads its progress and, once it has finished, its patch and report,
//! [`MATCH_APPLY`] sets the patch and [`MATCH_CANCEL`] stops the search with
//! the best patch so far.

use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use lspk_commands::{
    CommandSpec, Context, NoArgs, Permission, RegisterError, Registry as Commands,
};
use lspk_mod::{Polarity, Target};
use lspk_params::ParamId;
use lspk_plugin::PluginError;
use lspk_test::Script;
use lspk_test::features::{self, Features};
use lspk_test::render::{Instance, Output, Settings};
use reagent_engine::Registry;
use reagent_engine::params::{FILTER, LFO, OSC, channel, env, env_id, filter, id, lfo, osc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Reagent;
use crate::patches::Patch;

/// `reagent.match_sound`: start a search for a patch that sounds like a
/// recording.
pub const MATCH_SOUND: &str = "reagent.match_sound";
/// `reagent.match_status`: a search's progress, and its result once it ends.
pub const MATCH_STATUS: &str = "reagent.match_status";
/// `reagent.match_apply`: set the patch a search found.
pub const MATCH_APPLY: &str = "reagent.match_apply";
/// `reagent.match_cancel`: stop the search, keeping the best patch so far.
pub const MATCH_CANCEL: &str = "reagent.match_cancel";

/// How many candidates a search renders when it isn't told, the Init patch
/// it is read against included.
pub const DEFAULT_EVALUATIONS: usize = 600;
/// The longest stretch of the reference that's matched, in seconds.
const MAX_SECONDS: f64 = 4.0;
/// The note's velocity.
const VELOCITY: f32 = 0.8;
/// Frames per block.
const BLOCK: usize = 512;
/// Candidates in a batch of the evolution strategy.
const BATCH: usize = 16;
/// The part of the budget the random batch takes: one over this.
const EXPLORE_SHARE: usize = 6;
/// Chains of the evolution strategy, each from one of the best random points.
const CHAINS: usize = 3;
/// The noise's size, in the unit range, at a chain's first and last batch.
const SIGMA: (f64, f64) = (0.5, 0.01);
/// The chance that a child's dimension is moved.
const MOVE: f64 = 0.3;
/// What a candidate that renders silence counts as: further than any sound
/// the distance ranks.
const SILENT: f64 = 100.0;

/// How a dimension maps the unit range onto its parameter.
#[derive(Clone, Copy)]
enum Scale {
    /// Evenly from low to high.
    Linear,
    /// Evenly in the logarithm, for times and frequencies.
    Log,
    /// The whole numbers from low to high, each an equal share.
    Whole,
}

/// A parameter the search sets, from `low` to `high` in real units.
struct Dim {
    id: ParamId,
    low: f64,
    high: f64,
    scale: Scale,
}

impl Dim {
    fn real(&self, unit: f64) -> f64 {
        match self.scale {
            Scale::Linear => self.low + (self.high - self.low) * unit,
            Scale::Log => self.low * (self.high / self.low).powf(unit),
            Scale::Whole => {
                (self.low + (unit * (self.high - self.low + 1.0)).floor()).min(self.high)
            }
        }
    }
}

/// A modulation route whose amount the search sets; under [`Self::MIN`] it
/// isn't made.
struct RouteDim {
    source: &'static str,
    destination: ParamId,
    high: f32,
    polarity: Polarity,
}

impl RouteDim {
    const MIN: f32 = 0.02;
}

/// The patches the search can make: a point of the unit cube, one dimension
/// for each parameter and route in it, decoded into a [`Patch`].
pub struct Space {
    dims: Vec<Dim>,
    routes: Vec<RouteDim>,
    /// Where the switch of Filter 1 is in `dims`: a patch that isn't
    /// filtered sends its oscillators to Main.
    filter_on: usize,
}

impl Default for Space {
    fn default() -> Self {
        Self::new()
    }
}

impl Space {
    /// The search space: see the [module](self).
    #[must_use]
    pub fn new() -> Self {
        let dim = |id, low, high, scale| Dim {
            id,
            low,
            high,
            scale,
        };
        let osc = |s: u8, p| id(OSC, s, p);
        let mut dims = vec![
            dim(osc(0, osc::OCTAVE), -2.0, 1.0, Scale::Whole),
            dim(osc(0, osc::FRAME), 0.0, 100.0, Scale::Linear),
            dim(osc(0, osc::UNISON), 1.0, 8.0, Scale::Whole),
            dim(osc(0, osc::DETUNE), 0.0, 0.5, Scale::Linear),
            dim(osc(0, osc::DETUNE_LAW), 0.0, 4.0, Scale::Whole),
            dim(osc(0, channel::LEVEL), 50.0, 90.0, Scale::Linear),
            dim(osc(1, channel::ON), 0.0, 1.0, Scale::Whole),
            dim(osc(1, osc::OCTAVE), -2.0, 1.0, Scale::Whole),
            dim(osc(1, osc::FINE), -30.0, 30.0, Scale::Linear),
            dim(osc(1, osc::FRAME), 0.0, 100.0, Scale::Linear),
            dim(osc(1, osc::UNISON), 1.0, 4.0, Scale::Whole),
            dim(osc(1, channel::LEVEL), 30.0, 80.0, Scale::Linear),
        ];
        let filter_on = dims.len();
        dims.extend([
            dim(id(FILTER, 0, filter::ON), 0.0, 1.0, Scale::Whole),
            // The first 32 types: the ladders, state-variable, peak, notch,
            // dual and morph filters, which pass a tone through.
            dim(id(FILTER, 0, filter::TYPE), 0.0, 31.0, Scale::Whole),
            dim(id(FILTER, 0, filter::CUTOFF), 150.0, 16_000.0, Scale::Log),
            dim(id(FILTER, 0, filter::RESONANCE), 0.0, 60.0, Scale::Linear),
            dim(id(FILTER, 0, filter::DRIVE), 0.0, 40.0, Scale::Linear),
            dim(env_id(0, env::ATTACK), 1.0, 500.0, Scale::Log),
            dim(env_id(0, env::DECAY), 20.0, 2_000.0, Scale::Log),
            dim(env_id(0, env::SUSTAIN), 0.2, 1.0, Scale::Linear),
            dim(env_id(0, env::RELEASE), 20.0, 1_500.0, Scale::Log),
            dim(env_id(1, env::DECAY), 50.0, 2_000.0, Scale::Log),
            dim(env_id(1, env::SUSTAIN), 0.0, 0.6, Scale::Linear),
            dim(id(LFO, 0, lfo::RATE), 0.2, 8.0, Scale::Log),
        ]);
        let cutoff = id(FILTER, 0, filter::CUTOFF);
        Self {
            dims,
            routes: vec![
                RouteDim {
                    source: "env2",
                    destination: cutoff,
                    high: 0.7,
                    polarity: Polarity::Unipolar,
                },
                RouteDim {
                    source: "lfo1",
                    destination: cutoff,
                    high: 0.5,
                    polarity: Polarity::Bipolar,
                },
            ],
            filter_on,
        }
    }

    /// How many dimensions a point has.
    #[must_use]
    pub fn len(&self) -> usize {
        self.dims.len() + self.routes.len()
    }

    /// Never: the space has dimensions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The patch at `point`, each coordinate in 0 to 1.
    #[must_use]
    pub fn decode(&self, point: &[f64]) -> Patch {
        let (values, amounts) = point.split_at(self.dims.len());
        let mut params: Vec<(ParamId, f64)> = self
            .dims
            .iter()
            .zip(values)
            .map(|(dim, unit)| (dim.id, dim.real(*unit)))
            .collect();
        let filtered = params[self.filter_on].1 >= 0.5;
        let to = if filtered { 0.0 } else { 1.0 };
        params.push((id(OSC, 0, channel::OUTPUT), to));
        params.push((id(OSC, 1, channel::OUTPUT), to));
        // The LFO runs free, at the rate the point gives.
        params.push((id(LFO, 0, lfo::SYNC), 0.0));
        let routes = self
            .routes
            .iter()
            .zip(amounts)
            .map(|(route, unit)| (route, route.high * *unit as f32))
            .filter(|(_, amount)| *amount >= RouteDim::MIN)
            .map(|(route, amount)| {
                (
                    route.source,
                    Target::Param(route.destination),
                    amount,
                    route.polarity,
                )
            })
            .collect();
        Patch {
            name: "match",
            params,
            routes,
        }
    }
}

/// SplitMix64: a small generator whose output is fixed by its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Normally distributed, mean 0 and deviation 1 (Box–Muller).
    fn normal(&mut self) -> f64 {
        let (u, v) = (1.0 - self.unit(), self.unit());
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

/// What to search for, and how hard.
#[derive(Clone, Debug, Default)]
pub struct Args {
    /// How many candidates to render, at least 2; [`DEFAULT_EVALUATIONS`] if
    /// `None`.
    pub evaluations: Option<usize>,
    /// The seed every draw comes from.
    pub seed: u64,
    /// The MIDI key the note is played on. If `None`, the key nearest the
    /// reference's pitch, or middle C if it has none.
    pub key: Option<u8>,
    /// How long the key is held, in seconds. If `None`, two thirds of the
    /// reference's length.
    pub hold: Option<f64>,
    /// How many threads render; one per core if `None`.
    pub threads: Option<usize>,
}

impl Args {
    /// How many candidates the search renders: the Init patch it's read
    /// against and at least one more.
    #[must_use]
    pub fn budget(&self) -> usize {
        self.evaluations.unwrap_or(DEFAULT_EVALUATIONS).max(2)
    }
}

/// How far a search is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    /// Candidates rendered so far.
    pub evaluations: usize,
    /// How many it will render.
    pub budget: usize,
    /// The distance of the best so far, once there is one.
    pub best_distance: Option<f64>,
}

/// A finished search.
#[derive(Clone, Debug)]
pub struct Match {
    /// The patch nearest the reference.
    pub patch: Patch,
    /// Its distance from the reference: 0 for the same features.
    pub distance: f64,
    /// The Init patch's distance, to read it against.
    pub baseline: f64,
    /// Candidates rendered.
    pub evaluations: usize,
    /// How long the search took, in seconds.
    pub seconds: f64,
    /// The key the note was played on.
    pub key: u8,
    /// How long it was held, in seconds.
    pub hold: f64,
    /// Whether it was stopped before its budget.
    pub cancelled: bool,
}

/// What renders candidates: the reference's features and the note they play.
struct Judge {
    reference: Features,
    rate: f64,
    key: u8,
    hold: f64,
    seconds: f64,
    registry: Arc<Registry>,
}

impl Judge {
    /// How far `patch` renders from the reference.
    fn distance(&self, patch: &Patch) -> f64 {
        let render = || -> Result<Option<Features>, String> {
            let mut instance = Instance::<Reagent>::new()?;
            instance.load_state(&patch.state(&self.registry))?;
            instance.activate(Settings::new(self.rate, BLOCK))?;
            let frames = |seconds: f64| (seconds * self.rate).round() as usize;
            let script = Script::new(frames(self.seconds)).note(
                0,
                frames(self.hold) as u32,
                self.key,
                VELOCITY,
            );
            let output = instance.render(&script).output;
            Ok(output
                .is_finite()
                .then(|| features::extract(&output))
                .flatten())
        };
        match render() {
            Ok(Some(candidate)) => features::distance(&self.reference, &candidate),
            _ => SILENT,
        }
    }

    /// Each point's distance, rendered on `threads` threads.
    fn distances(&self, space: &Space, points: &[Vec<f64>], threads: usize) -> Vec<f64> {
        let mut distances = vec![0.0; points.len()];
        let chunk = points.len().div_ceil(threads.max(1)).max(1);
        std::thread::scope(|scope| {
            for (points, out) in points.chunks(chunk).zip(distances.chunks_mut(chunk)) {
                scope.spawn(move || {
                    for (point, out) in points.iter().zip(out) {
                        *out = self.distance(&space.decode(point));
                    }
                });
            }
        });
        distances
    }
}

/// The key nearest `hz`.
fn key_of(hz: f64) -> u8 {
    (69.0 + 12.0 * (hz / 440.0).log2())
        .round()
        .clamp(0.0, 127.0) as u8
}

/// Searches for the patch nearest `reference`, a recording of one held note.
/// `progress` hears of each batch of candidates; the search stops, keeping
/// its best, once `cancel` is set.
///
/// # Errors
///
/// The reference is silent.
pub fn search(
    reference: &Output,
    args: &Args,
    progress: &mut dyn FnMut(Progress),
    cancel: &AtomicBool,
) -> Result<Match, String> {
    let started = Instant::now();
    // Only the stretch the candidates are rendered for is read.
    let kept = (MAX_SECONDS * reference.sample_rate) as usize;
    let reference = Output::new(
        reference.sample_rate,
        reference
            .channels
            .iter()
            .map(|channel| channel[..channel.len().min(kept)].to_vec())
            .collect(),
    );
    let features = features::extract(&reference).ok_or("the reference is silent")?;
    let seconds = (reference.channels.first().map_or(0, Vec::len) as f64 / reference.sample_rate)
        .clamp(0.25, MAX_SECONDS);
    let key = args
        .key
        .unwrap_or_else(|| features.pitch_hz.map_or(60, key_of));
    let hold = args
        .hold
        .unwrap_or(seconds * 2.0 / 3.0)
        .clamp(0.01, seconds);
    let judge = Judge {
        reference: features,
        rate: reference.sample_rate,
        key,
        hold,
        seconds,
        registry: Registry::new(),
    };
    let threads = args
        .threads
        .or_else(|| std::thread::available_parallelism().ok().map(Into::into))
        .unwrap_or(1);
    let budget = args.budget();
    let space = Space::new();
    let mut rng = Rng(args.seed);

    let baseline = judge.distance(&crate::patches::init());
    let mut done = 1;
    let mut best = f64::INFINITY;
    let mut render = |points: &[Vec<f64>], done: &mut usize| {
        let distances = judge.distances(&space, points, threads);
        *done += points.len();
        best = distances.iter().copied().fold(best, f64::min);
        progress(Progress {
            evaluations: *done,
            budget,
            best_distance: Some(best),
        });
        distances
    };

    // Stage 1: random points over the whole space, a batch at a time; the
    // best few are where the chains start.
    let explore = (budget / EXPLORE_SHARE).clamp(1, budget.saturating_sub(done).max(1));
    let mut starts: Vec<(f64, Vec<f64>)> = Vec::new();
    while starts.len() < explore {
        let points: Vec<Vec<f64>> = (0..BATCH.min(explore - starts.len()))
            .map(|_| (0..space.len()).map(|_| rng.unit()).collect())
            .collect();
        let distances = render(&points, &mut done);
        starts.extend(distances.into_iter().zip(points));
        if cancel.load(Ordering::Relaxed) {
            break;
        }
    }
    starts.sort_by(|a, b| a.0.total_cmp(&b.0));
    starts.truncate(CHAINS);

    // Stage 2: each start's chain, a parent and its children, the noise
    // shrinking along the chain. Chains are apart, so one that has settled
    // in a poor basin doesn't take the others with it.
    let chain_batches = budget.saturating_sub(done).div_ceil(BATCH * starts.len());
    let mut finalists = Vec::new();
    for (mut parent_distance, mut parent) in starts {
        for batch in 0..chain_batches {
            if cancel.load(Ordering::Relaxed) || done >= budget {
                break;
            }
            let size = BATCH.min(budget - done);
            let sigma = SIGMA.0 * (SIGMA.1 / SIGMA.0).powf(batch as f64 / chain_batches as f64);
            let children: Vec<Vec<f64>> = (0..size)
                .map(|_| {
                    let forced = (rng.next() % space.len() as u64) as usize;
                    parent
                        .iter()
                        .enumerate()
                        .map(|(i, x)| {
                            if i == forced || rng.unit() < MOVE {
                                (x + sigma * rng.normal()).clamp(0.0, 1.0 - f64::EPSILON)
                            } else {
                                *x
                            }
                        })
                        .collect()
                })
                .collect();
            let distances = render(&children, &mut done);
            for (distance, child) in distances.into_iter().zip(children) {
                if distance < parent_distance {
                    (parent_distance, parent) = (distance, child);
                }
            }
        }
        finalists.push((parent_distance, parent));
    }

    let (distance, point) = finalists
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .expect("at least one chain");
    Ok(Match {
        patch: space.decode(&point),
        distance,
        baseline,
        evaluations: done,
        seconds: started.elapsed().as_secs_f64(),
        key,
        hold,
        cancelled: done < budget,
    })
}

/// A search running on its thread, or finished on it. Dropping it stops the
/// search.
pub struct Job {
    shared: Arc<Shared>,
    started: Instant,
}

struct Shared {
    cancel: AtomicBool,
    state: Mutex<State>,
}

struct State {
    progress: Progress,
    /// Set when the thread ends.
    outcome: Option<Result<Match, String>>,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }
}

/// The arguments of [`MATCH_SOUND`].
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MatchSoundArgs {
    /// The recording to match, a WAV file: one note held, as a patch plays
    /// one, its release included.
    reference: String,
    /// How many candidate patches to render, at least 2. More finds a
    /// nearer patch and takes longer; the default is 600.
    #[serde(default)]
    evaluations: Option<usize>,
    /// The seed the search draws from; the same seed, reference and budget
    /// find the same patch. Left out: 0.
    #[serde(default)]
    seed: Option<u64>,
    /// The MIDI key the note is played on, from 0 to 127. Left out: the key
    /// nearest the reference's pitch.
    #[serde(default)]
    key: Option<u8>,
    /// How long the key is held, in seconds. Left out: two thirds of the
    /// reference's length.
    #[serde(default)]
    hold: Option<f64>,
}

/// Registers the matching commands.
///
/// # Errors
///
/// A command's ID is already taken.
pub fn register(registry: &mut Commands) -> Result<(), RegisterError> {
    for spec in [
        CommandSpec::new(MATCH_SOUND, "Match Sound")
            .description(
                "Starts a search for a patch that sounds like a recording, and returns at once \
                 with the search's status. The search renders candidate patches on a thread of \
                 its own, offline, and keeps the one whose perception features (spectrum over \
                 time, envelope, pitch, modulation, stereo width) are nearest the recording's. \
                 It can set oscillators A and B, Filter 1, Env 1 and 2, LFO 1 and the routes \
                 from Env 2 and LFO 1 to the cutoff. Read its progress and result with \
                 `reagent.match_status`, and set the patch it found with `reagent.match_apply`. \
                 It changes nothing in the patch itself, and one search runs at a time.",
            )
            .permission(Permission::External)
            .context(Context::Global)
            .args::<MatchSoundArgs>()
            .result::<Value>(),
        CommandSpec::new(MATCH_STATUS, "Match Status")
            .description(
                "How the last search is going: its state (`running`, `done`, `cancelled` or \
                 `failed`), the candidates rendered of its budget, the distance of the best \
                 (0 is the same features) and the seconds so far. Once it has ended, also its \
                 `result`: the patch (each parameter's ID, name and value, and the modulation \
                 routes) and a report with its distance, the Init patch's distance to read it \
                 against, `closeness` (1 minus the first over the second, from 0 to 1) and \
                 the key and hold the note was played with.",
            )
            .permission(Permission::Read)
            .context(Context::Global)
            .args::<NoArgs>()
            .result::<Value>(),
        CommandSpec::new(MATCH_APPLY, "Apply Matched Patch")
            .description(
                "Sets the patch the last search found, as `reagent.init_patch` followed by its \
                 parameters and routes: every parameter it doesn't name is at its default and \
                 the modulation matrix is replaced. Fails if no search has found one yet. The \
                 host records each parameter change like a person's.",
            )
            .permission(Permission::Edit)
            .context(Context::Global)
            .args::<NoArgs>()
            .result::<()>(),
        CommandSpec::new(MATCH_CANCEL, "Cancel Match")
            .description(
                "Stops the running search at the end of its batch; its status then reads \
                 `cancelled` and its result holds the best patch found so far.",
            )
            .permission(Permission::Edit)
            .context(Context::Global)
            .args::<NoArgs>()
            .result::<()>(),
    ] {
        registry.register(spec)?;
    }
    Ok(())
}

/// Runs a matching command on `plugin`.
pub(crate) fn run(plugin: &mut Reagent, id: &str, args: &Value) -> Result<Value, PluginError> {
    match id {
        MATCH_SOUND => {
            let args: MatchSoundArgs = crate::arguments(args)?;
            if plugin
                .matching
                .as_ref()
                .is_some_and(|job| job.shared.state().outcome.is_none())
            {
                return Err(PluginError::new(
                    "a search is running; cancel it with reagent.match_cancel first",
                ));
            }
            let bytes = fs::read(&args.reference)
                .map_err(|e| PluginError::new(format!("{}: {e}", args.reference)))?;
            let reference = lspk_test::agent::decode_wav(&bytes)
                .map_err(|e| PluginError::new(format!("{}: {e}", args.reference)))?;
            let search_args = Args {
                evaluations: args.evaluations,
                seed: args.seed.unwrap_or(0),
                key: args.key,
                hold: args.hold,
                // Half the cores, so a host's audio threads keep theirs.
                threads: std::thread::available_parallelism()
                    .ok()
                    .map(|cores| (usize::from(cores) / 2).max(1)),
            };
            let shared = Arc::new(Shared {
                cancel: AtomicBool::new(false),
                state: Mutex::new(State {
                    progress: Progress {
                        evaluations: 0,
                        budget: search_args.budget(),
                        best_distance: None,
                    },
                    outcome: None,
                }),
            });
            let worker = shared.clone();
            std::thread::Builder::new()
                .name("reagent-match".to_owned())
                .spawn(move || {
                    let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        search(
                            &reference,
                            &search_args,
                            &mut |progress| worker.state().progress = progress,
                            &worker.cancel,
                        )
                    }))
                    .unwrap_or_else(|_| Err("the search failed unexpectedly".to_owned()));
                    worker.state().outcome = Some(found);
                })
                .map_err(|e| PluginError::new(e.to_string()))?;
            let job = plugin.matching.insert(Job {
                shared,
                started: Instant::now(),
            });
            Ok(status(job, &plugin.registry))
        }
        MATCH_STATUS => plugin
            .matching
            .as_ref()
            .map(|job| status(job, &plugin.registry))
            .ok_or_else(|| PluginError::new("no search has been started")),
        MATCH_CANCEL => {
            let job = plugin
                .matching
                .as_ref()
                .ok_or_else(|| PluginError::new("no search has been started"))?;
            job.shared.cancel.store(true, Ordering::Relaxed);
            Ok(Value::Null)
        }
        MATCH_APPLY => {
            let patch = plugin
                .matching
                .as_ref()
                .and_then(|job| match &job.shared.state().outcome {
                    Some(Ok(found)) => Some(found.patch.clone()),
                    _ => None,
                })
                .ok_or_else(|| PluginError::new("no search has found a patch yet"))?;
            plugin.init_patch()?;
            for (param, value) in &patch.params {
                plugin
                    .host
                    .edit(*param, *value)
                    .map_err(|e| PluginError::new(e.to_string()))?;
            }
            plugin.matrix = patch.matrix(&plugin.registry);
            plugin.send_matrix();
            plugin.recompile();
            plugin.host.mark_dirty();
            Ok(Value::Null)
        }
        _ => Err(PluginError::new(format!("no command `{id}`"))),
    }
}

/// A search's status, as [`MATCH_STATUS`] reads it.
fn status(job: &Job, registry: &Registry) -> Value {
    let state = job.shared.state();
    let name = |param: ParamId| {
        registry
            .space
            .view(param, &|_| None)
            .map_or_else(|| param.0.to_string(), |view| view.name)
    };
    let (label, error, found) = match &state.outcome {
        None => ("running", None, None),
        Some(Err(message)) => ("failed", Some(message), None),
        Some(Ok(found)) => (
            if found.cancelled { "cancelled" } else { "done" },
            None,
            Some(found),
        ),
    };
    json!({
        "state": label,
        "error": error,
        "evaluations": state.progress.evaluations,
        "budget": state.progress.budget,
        "best_distance": state.progress.best_distance,
        "elapsed_s": found.map_or_else(|| job.started.elapsed().as_secs_f64(), |m| m.seconds),
        "result": found.map(|found| json!({
            "patch": {
                "params": found.patch.params.iter().map(|(param, value)| {
                    json!({ "id": param.0, "name": name(*param), "value": value })
                }).collect::<Vec<_>>(),
                "routes": found.patch.routes.iter().map(|(source, target, amount, polarity)| {
                    let Target::Param(destination) = target else { unreachable!("the space routes to parameters") };
                    json!({
                        "source": source,
                        "destination": name(*destination),
                        "amount": amount,
                        "polarity": match polarity { Polarity::Unipolar => "unipolar", Polarity::Bipolar => "bipolar" },
                    })
                }).collect::<Vec<_>>(),
            },
            "report": {
                "distance": found.distance,
                "baseline": found.baseline,
                "closeness": if found.baseline > 0.0 { (1.0 - found.distance / found.baseline).clamp(0.0, 1.0) } else { 1.0 },
                "evaluations": found.evaluations,
                "seconds": found.seconds,
                "key": found.key,
                "hold": found.hold,
            },
        })),
    })
}
