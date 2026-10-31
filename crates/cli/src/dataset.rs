//! `reagent-cli dataset`: thousands of random, playable patches rendered
//! and measured, for training and for sound matching.
//!
//! A dataset is a folder:
//!
//! - `dataset.json`: the format version, the seed and count, the sample
//!   rate, the note every patch plays and what the features are.
//! - `patches.jsonl`: one JSON object per line, in index order: `index`,
//!   `attempts` (how many random patches the sampler drew before one was
//!   playable), `params` (each parameter the patch sets, as `[id, value]`
//!   with the value in real units; every other parameter is at its default),
//!   `routes` (each modulation route: `source` key, `destination` parameter
//!   ID, `amount`, `polarity`) and `features`.
//! - `audio/<index>.wav`: the render, a 32-bit float WAV, when `--audio` is
//!   given.
//!
//! Everything in the folder is a function of the seed and the count: patch
//! `i` comes from the seed and `i` alone, so a dataset repeats to the byte
//! whatever the thread count, and any patch can be drawn again without the
//! rest.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use clap::Parser;
use lspk_mod::{Polarity, Target};
use lspk_params::ParamId;
use lspk_test::Script;
use lspk_test::measure;
use lspk_test::render::{Instance, Output, Settings};
use reagent_core::Reagent;
use reagent_core::patches::Patch;
use reagent_engine::Registry;
use reagent_engine::params::{
    self, FILTER, LFO, NOISE, OSC, SUB, channel, env, filter, id, lfo, noise, osc, sub,
};
use serde_json::{Value, json};

use crate::Outcome;

/// The dataset format's version.
const FORMAT: u32 = 1;
/// The sample rate every patch renders at, in hertz.
const RATE: f64 = 48_000.0;
/// Frames per block.
const BLOCK: usize = 512;
/// The note every patch plays: MIDI key 48 at velocity 0.8, held for one
/// second, in a render of one and a half.
const KEY: u8 = 48;
const VELOCITY: f32 = 0.8;
const HOLD_SECONDS: f64 = 1.0;
const RENDER_SECONDS: f64 = 1.5;
/// Patches the sampler may draw for one index before it gives up.
const MAX_ATTEMPTS: u32 = 64;

/// How to make a dataset.
#[derive(Debug, Parser)]
pub struct DatasetArgs {
    /// The folder to write; it is made if it isn't there.
    #[arg(long)]
    pub out: PathBuf,
    /// How many patches.
    #[arg(long)]
    pub count: usize,
    /// The seed every patch is drawn from.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
    /// Also write each render as `audio/<index>.wav`.
    #[arg(long)]
    pub audio: bool,
    /// How many patches render at once; default one per core.
    #[arg(long)]
    pub threads: Option<usize>,
}

/// SplitMix64: a small generator whose output is fixed by its seed.
struct Rng(u64);

impl Rng {
    /// The generator for patch `index` of `seed`.
    fn for_patch(seed: u64, index: usize) -> Self {
        let mut rng = Self(seed);
        rng.0 = rng
            .next()
            .wrapping_add((index as u64).wrapping_mul(0xA24B_AED4_963E_E407));
        rng
    }

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

    fn uniform(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.unit()
    }

    /// Even on a logarithmic scale, for times and frequencies.
    fn log(&mut self, low: f64, high: f64) -> f64 {
        low * (high / low).powf(self.unit())
    }

    /// A whole number from `low` to `high`, both included.
    fn int(&mut self, low: i64, high: i64) -> f64 {
        (low + (self.next() % (high - low + 1) as u64) as i64) as f64
    }

    fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
}

/// A random patch a player could use: oscillator A always sounds, through
/// Filter 1 or straight to Main; the other sources, Filter 1, a modulation
/// envelope and an LFO come and go; every level, time and cutoff stays in
/// the range patches are played in.
fn sample(rng: &mut Rng) -> Patch {
    let osc = |s: u8, p| id(OSC, s, p);
    let mut params: Vec<(ParamId, f64)> = Vec::new();
    let mut routes: Vec<(&'static str, Target, f32, Polarity)> = Vec::new();
    let filtered = rng.chance(0.75);
    let to_filter = if filtered { 0.0 } else { 1.0 };

    params.push((osc(0, osc::OCTAVE), rng.int(-2, 1)));
    params.push((osc(0, osc::FRAME), rng.uniform(0.0, 100.0)));
    params.push((osc(0, osc::UNISON), rng.int(1, 8)));
    params.push((osc(0, osc::DETUNE), rng.uniform(0.0, 0.5)));
    params.push((osc(0, osc::DETUNE_LAW), rng.int(0, 4)));
    params.push((osc(0, channel::LEVEL), rng.uniform(50.0, 90.0)));
    params.push((osc(0, channel::OUTPUT), to_filter));
    if rng.chance(0.5) {
        params.push((osc(1, channel::ON), 1.0));
        params.push((osc(1, channel::OUTPUT), to_filter));
        params.push((osc(1, osc::OCTAVE), rng.int(-2, 1)));
        params.push((osc(1, osc::FINE), rng.uniform(-30.0, 30.0)));
        params.push((osc(1, osc::FRAME), rng.uniform(0.0, 100.0)));
        params.push((osc(1, osc::UNISON), rng.int(1, 4)));
        params.push((osc(1, channel::LEVEL), rng.uniform(30.0, 80.0)));
    }
    if rng.chance(0.25) {
        params.push((id(SUB, 0, channel::ON), 1.0));
        params.push((id(SUB, 0, channel::OUTPUT), to_filter));
        params.push((id(SUB, 0, sub::OCTAVE), rng.int(-2, 0)));
        params.push((id(SUB, 0, sub::SHAPE), rng.int(0, 5)));
        params.push((id(SUB, 0, channel::LEVEL), rng.uniform(30.0, 70.0)));
    }
    if rng.chance(0.15) {
        params.push((id(NOISE, 0, channel::ON), 1.0));
        params.push((id(NOISE, 0, channel::OUTPUT), to_filter));
        params.push((id(NOISE, 0, noise::SOURCE), rng.int(0, 3)));
        params.push((id(NOISE, 0, channel::LEVEL), rng.uniform(5.0, 30.0)));
    }

    let cutoff = Target::Param(id(FILTER, 0, filter::CUTOFF));
    if filtered {
        params.push((id(FILTER, 0, filter::ON), 1.0));
        // The first 32 types: the ladders, state-variable, peak, notch, dual
        // and morph filters, which pass a tone through.
        params.push((id(FILTER, 0, filter::TYPE), rng.int(0, 31)));
        params.push((id(FILTER, 0, filter::CUTOFF), rng.log(150.0, 16_000.0)));
        params.push((id(FILTER, 0, filter::RESONANCE), rng.uniform(0.0, 60.0)));
        params.push((id(FILTER, 0, filter::DRIVE), rng.uniform(0.0, 40.0)));
    }

    params.push((params::env_id(0, env::ATTACK), rng.log(1.0, 500.0)));
    params.push((params::env_id(0, env::DECAY), rng.log(20.0, 2_000.0)));
    params.push((params::env_id(0, env::SUSTAIN), rng.uniform(0.2, 1.0)));
    params.push((params::env_id(0, env::RELEASE), rng.log(20.0, 1_500.0)));

    let frame = Target::Param(osc(0, osc::FRAME));
    if filtered && rng.chance(0.5) {
        params.push((params::env_id(1, env::DECAY), rng.log(50.0, 2_000.0)));
        params.push((params::env_id(1, env::SUSTAIN), rng.uniform(0.0, 0.6)));
        routes.push((
            "env2",
            cutoff,
            rng.uniform(0.1, 0.7) as f32,
            Polarity::Unipolar,
        ));
    }
    if rng.chance(0.4) {
        params.push((id(LFO, 0, lfo::SYNC), 0.0));
        params.push((id(LFO, 0, lfo::RATE), rng.log(0.2, 8.0)));
        let target = if filtered { cutoff } else { frame };
        routes.push((
            "lfo1",
            target,
            rng.uniform(0.1, 0.5) as f32,
            Polarity::Bipolar,
        ));
    }
    Patch {
        name: "dataset",
        params,
        routes,
    }
}

/// The phrase every patch plays.
fn script() -> Script {
    let frames = |seconds: f64| (seconds * RATE).round() as usize;
    Script::new(frames(RENDER_SECONDS)).note(0, frames(HOLD_SECONDS) as u32, KEY, VELOCITY)
}

/// `output` as the features a dataset stores. `None` if it isn't playable:
/// not finite, quieter than −50 dBFS RMS, or over 0 dBFS.
fn features(output: &Output) -> Option<Value> {
    let mono = output.mono();
    let (peak, rms) = (
        measure::db(measure::peak(&mono)),
        measure::db(measure::rms(&mono)),
    );
    if !output.is_finite() || rms < -50.0 || peak > 0.0 {
        return None;
    }
    let envelope = measure::envelope(&mono, RATE);
    Some(json!({
        "peak_db": peak,
        "rms_db": rms,
        "centroid_hz": measure::centroid(&mono, RATE),
        "pitch_hz": measure::pitch(&mono, RATE),
        "onset_s": envelope.map(|e| e.onset),
        "attack_end_s": envelope.map(|e| e.peak),
        "decay_end_s": envelope.map(|e| e.end),
    }))
}

/// Patch `index` of `seed`: draws until a patch renders playable, and
/// returns its record and its render.
fn patch(seed: u64, index: usize, registry: &Registry) -> Result<(Value, Output), String> {
    let mut rng = Rng::for_patch(seed, index);
    for attempt in 1..=MAX_ATTEMPTS {
        let patch = sample(&mut rng);
        let mut instance = Instance::<Reagent>::new()?;
        instance.load_state(&patch.state(registry))?;
        instance.activate(Settings::new(RATE, BLOCK))?;
        let output = instance.render(&script()).output;
        let Some(features) = features(&output) else {
            continue;
        };
        let record = json!({
            "index": index,
            "attempts": attempt,
            "params": patch.params.iter().map(|(id, value)| json!([id.0, value])).collect::<Vec<_>>(),
            "routes": patch.routes.iter().map(|(source, target, amount, polarity)| {
                let Target::Param(destination) = target else { unreachable!("the sampler routes to parameters") };
                json!({
                    "source": source,
                    "destination": destination.0,
                    "amount": amount,
                    "polarity": match polarity { Polarity::Unipolar => "unipolar", Polarity::Bipolar => "bipolar" },
                })
            }).collect::<Vec<_>>(),
            "features": features,
        });
        return Ok((record, output));
    }
    Err(format!(
        "patch {index}: no playable patch in {MAX_ATTEMPTS} draws"
    ))
}

/// The manifest `dataset.json` holds.
fn manifest(args: &DatasetArgs) -> Value {
    json!({
        "format": FORMAT,
        "seed": args.seed,
        "count": args.count,
        "audio": args.audio,
        "sample_rate": RATE,
        "note": {
            "key": KEY,
            "velocity": VELOCITY,
            "hold_s": HOLD_SECONDS,
            "render_s": RENDER_SECONDS,
        },
        "features": {
            "peak_db": "the largest sample, dBFS",
            "rms_db": "the RMS of the mono mix, dBFS",
            "centroid_hz": "the power-weighted mean frequency, Hz; null for silence",
            "pitch_hz": "the fundamental, Hz; null if not clearly periodic",
            "onset_s": "when the level first comes within 40 dB of its loudest",
            "attack_end_s": "when it first comes within 1 dB of its loudest",
            "decay_end_s": "when it has fallen for good to 60 dB below its loudest",
        },
    })
}

/// Renders the dataset `args` describes and prints its throughput.
pub fn run(args: &DatasetArgs) -> Outcome {
    let started = Instant::now();
    let audio = args.out.join("audio");
    let make =
        |path: &Path| fs::create_dir_all(path).map_err(|e| format!("{}: {e}", path.display()));
    make(&args.out)?;
    if args.audio {
        make(&audio)?;
    }
    let threads = args
        .threads
        .or_else(|| std::thread::available_parallelism().ok().map(Into::into))
        .unwrap_or(1)
        .max(1);
    let registry = Registry::new();
    let next = AtomicUsize::new(0);
    // Each worker takes the next index; the lines are sorted by index below,
    // so which worker drew which patch can't show in the files.
    let mut lines: Vec<(usize, String)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| -> Result<Vec<(usize, String)>, String> {
                    let mut lines = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        if index >= args.count {
                            return Ok(lines);
                        }
                        let (record, output) = patch(args.seed, index, &registry)?;
                        if args.audio {
                            crate::render::write(&output, &audio.join(format!("{index:08}.wav")))?;
                        }
                        lines.push((index, record.to_string()));
                    }
                })
            })
            .collect();
        let mut all = Vec::new();
        for worker in workers {
            all.extend(
                worker
                    .join()
                    .map_err(|_| "a render thread panicked".to_owned())??,
            );
        }
        Ok::<_, String>(all)
    })?;
    lines.sort_by_key(|(index, _)| *index);
    let text: String = lines.iter().map(|(_, line)| format!("{line}\n")).collect();
    let write = |name: &str, text: String| {
        let path = args.out.join(name);
        fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    };
    write("patches.jsonl", text)?;
    write("dataset.json", format!("{:#}\n", manifest(args)))?;
    let seconds = started.elapsed().as_secs_f64();
    println!(
        "wrote {} patches to {} in {seconds:.1} s on {threads} threads: {:.0} patches a minute",
        args.count,
        args.out.display(),
        args.count as f64 * 60.0 / seconds,
    );
    Ok(ExitCode::SUCCESS)
}
