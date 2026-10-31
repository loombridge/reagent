//! `reagent-cli text-to-patch`: a description becomes a patch by the loop an
//! agent runs through the agent interface: start from a recipe, render a
//! note, read the perception features, adjust one control, render again.
//!
//! The agent here is a scripted one: a small vocabulary stands in for a
//! model's reading of the words. An instrument word (`bass`, `lead`, `pad`,
//! `pluck`) picks a recipe, the same ones the guide gives a model under
//! "Sound-design recipes"; a quality word (`dark`, `bright`, `wide`, `slow`,
//! `snappy`) sets a goal for one reading of the features and the control
//! that moves it. The loop bisects that control, on a log scale, until the
//! reading is inside the goal or it runs out of steps.
//!
//! With no description, the verb runs the evaluation set (`eval.json`, beside
//! this file): written descriptions, each with the judge that decides whether
//! the result suits it. The judges read the features and don't know the
//! recipes' goals, so a model's patches are judged the same way.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use lspk_params::{ParamId, db_to_gain};
use lspk_test::Script;
use lspk_test::features::{self, BANDS, FRAMES, HIGH_HZ, LOW_HZ};
use lspk_test::render::{Instance, Settings};
use reagent_core::Reagent;
use reagent_core::patches::Patch;
use reagent_engine::Registry;
use reagent_engine::params::{FILTER, OSC, SUB, channel, env, filter, global, id, osc};
use serde_json::{Value, json};

use crate::Outcome;

/// The sample rate every render uses, in hertz.
const RATE: f64 = 48_000.0;
/// Frames per block.
const BLOCK: usize = 512;
const VELOCITY: f32 = 0.8;
/// How long after the note ends the render goes on, for the release.
const TAIL_SECONDS: f64 = 1.5;
/// The most renders the loop spends on one goal.
const STEPS: usize = 10;
/// How far below its loudest cell the spectrum reaches, in dB.
const SPECTRUM_FLOOR_DB: f64 = 80.0;
/// The goal every patch has, whatever its words.
const LEVEL: &str = "level";
/// The envelope reading `attack_s` counts the sound as at full level from
/// the first moment within this many dB of its loudest.
const FULL_DB: f64 = -3.0;

/// The evaluation set.
const EVAL: &str = include_str!("eval.json");

/// How to run the loop.
#[derive(Debug, Parser)]
pub struct TextToPatchArgs {
    /// The sound to describe, in words. Without one, runs the evaluation set.
    pub description: Option<String>,
    /// Write the result as JSON to this file instead of standard output.
    #[arg(long)]
    pub out: Option<PathBuf>,
}

/// What the loop reads from a render, from the perception features.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Readings {
    /// How far above the note the spectrum's centre sits, in octaves.
    pub(crate) brightness_oct: Option<f64>,
    pub(crate) attack_s: f64,
    pub(crate) width: f64,
    pub(crate) pitch_hz: Option<f64>,
    pub(crate) level_db: f64,
    pub(crate) duration_s: f64,
}

impl Readings {
    pub(crate) fn of(f: &features::Features) -> Self {
        // The mean of the bands' centres on a log scale, each band weighted
        // by its level in dB above the spectrum's floor, so a quiet top
        // octave still counts: a plain power weighting reads a saw and the
        // same saw low-passed alike, since the fundamental holds the power.
        let band_hz = |b: usize| LOW_HZ * (HIGH_HZ / LOW_HZ).powf((b as f64 + 0.5) / BANDS as f64);
        let (mut weight, mut log_hz) = (0.0, 0.0);
        for (i, db) in f.spectrum.iter().enumerate() {
            let above_floor = db + SPECTRUM_FLOOR_DB;
            weight += above_floor;
            log_hz += above_floor * band_hz(i % BANDS).ln();
        }
        let attack_moment = f.envelope.iter().position(|db| *db >= FULL_DB).unwrap_or(0);
        Self {
            brightness_oct: f
                .pitch_hz
                .map(|pitch| (log_hz / weight - pitch.ln()) / std::f64::consts::LN_2),
            attack_s: (attack_moment as f64 + 0.5) / FRAMES as f64 * f.duration,
            width: f.stereo.width,
            pitch_hz: f.pitch_hz,
            level_db: f.level_db,
            duration_s: f.duration,
        }
    }

    fn get(&self, name: &str) -> Option<f64> {
        match name {
            "brightness_oct" => self.brightness_oct,
            "attack_s" => Some(self.attack_s),
            "width" => Some(self.width),
            "pitch_hz" => self.pitch_hz,
            "level_db" => Some(self.level_db),
            "duration_s" => Some(self.duration_s),
            _ => None,
        }
    }
}

/// A recipe: the patch an instrument word starts from, and the note it plays.
struct Recipe {
    word: String,
    key: u8,
    hold_seconds: f64,
    params: Vec<(ParamId, f64)>,
}

fn osc_param(s: u8, p: u16) -> ParamId {
    id(OSC, s, p)
}

fn filter_param(p: u16) -> ParamId {
    id(FILTER, 0, p)
}

fn amp(p: u16) -> ParamId {
    reagent_engine::params::env_id(0, p)
}

/// A filtered patch: the filter type's index (see `FILTER_TYPES`), cutoff in
/// hertz and resonance in per cent, then the amp envelope: attack, decay and
/// release in milliseconds, sustain from 0 to 1.
fn filtered(kind: f64, cutoff: f64, resonance: f64, adsr: [f64; 4]) -> Vec<(ParamId, f64)> {
    vec![
        (osc_param(0, channel::OUTPUT), 0.0),
        (filter_param(filter::ON), 1.0),
        (filter_param(filter::TYPE), kind),
        (filter_param(filter::CUTOFF), cutoff),
        (filter_param(filter::RESONANCE), resonance),
        (amp(env::ATTACK), adsr[0]),
        (amp(env::DECAY), adsr[1]),
        (amp(env::SUSTAIN), adsr[2]),
        (amp(env::RELEASE), adsr[3]),
    ]
}

fn recipe(word: &str) -> Option<Recipe> {
    let (key, hold_seconds, mut params, more): (u8, f64, _, Vec<(ParamId, f64)>) = match word {
        // A saw with a sine sub on its note, through a 24 dB ladder.
        "bass" => (
            36,
            1.0,
            filtered(3.0, 700.0, 20.0, [3.0, 400.0, 0.7, 120.0]),
            vec![
                (id(SUB, 0, channel::ON), 1.0),
                (id(SUB, 0, channel::OUTPUT), 0.0),
                (id(SUB, 0, channel::LEVEL), 70.0),
            ],
        ),
        // Two saws detuned a few cents, three unison copies on the first.
        "lead" => (
            60,
            1.0,
            filtered(5.0, 4_000.0, 15.0, [5.0, 300.0, 0.8, 200.0]),
            vec![
                (osc_param(0, osc::UNISON), 3.0),
                (osc_param(0, osc::DETUNE), 0.1),
                (osc_param(1, channel::ON), 1.0),
                (osc_param(1, channel::OUTPUT), 0.0),
                (osc_param(1, osc::FINE), 7.0),
                (osc_param(1, channel::LEVEL), 50.0),
            ],
        ),
        // Detuned unison saws, an open ladder and a slow envelope.
        "pad" => (
            60,
            2.5,
            filtered(1.0, 2_500.0, 10.0, [600.0, 800.0, 0.8, 1_200.0]),
            vec![
                (osc_param(0, osc::UNISON), 5.0),
                (osc_param(0, osc::DETUNE), 0.15),
                (osc_param(0, channel::LEVEL), 60.0),
            ],
        ),
        // One saw whose level falls to nothing: no sustain.
        "pluck" => (
            60,
            0.5,
            filtered(5.0, 3_000.0, 15.0, [1.0, 250.0, 0.0, 100.0]),
            Vec::new(),
        ),
        _ => return None,
    };
    params.extend(more);
    Some(Recipe {
        word: word.to_owned(),
        key,
        hold_seconds,
        params,
    })
}

/// A quality word's goal: the range one reading should fall in, and the
/// control the loop moves to get it there.
struct Goal {
    word: &'static str,
    reading: &'static str,
    range: (f64, f64),
    /// What the word sets before the loop starts.
    sets: Vec<(ParamId, f64)>,
    knob: ParamId,
    /// The knob's travel, in its own units (a gain's is a linear factor); the loop halves it on a log scale
    /// where it is positive throughout, else on a linear one.
    travel: (f64, f64),
    /// Whether a higher knob gives a higher reading.
    rises: bool,
}

fn goals() -> Vec<Goal> {
    let cutoff = filter_param(filter::CUTOFF);
    vec![
        Goal {
            word: "dark",
            reading: "brightness_oct",
            range: (f64::NEG_INFINITY, 1.2),
            sets: Vec::new(),
            knob: cutoff,
            travel: (60.0, 16_000.0),
            rises: true,
        },
        Goal {
            word: "bright",
            reading: "brightness_oct",
            range: (1.8, f64::INFINITY),
            sets: Vec::new(),
            knob: cutoff,
            travel: (60.0, 16_000.0),
            rises: true,
        },
        Goal {
            word: "wide",
            reading: "width",
            range: (0.3, 1.0),
            // Oscillator A hard left, oscillator B detuned against it, and
            // the loop moves B across.
            sets: vec![
                (osc_param(0, channel::PAN), -50.0),
                (osc_param(1, channel::ON), 1.0),
                (osc_param(1, channel::OUTPUT), 0.0),
                (osc_param(1, osc::FINE), -9.0),
                (osc_param(1, channel::LEVEL), 60.0),
                (osc_param(1, osc::UNISON), 7.0),
                (osc_param(1, osc::DETUNE), 0.2),
            ],
            knob: osc_param(1, channel::PAN),
            travel: (-50.0, 50.0),
            rises: true,
        },
        Goal {
            word: "slow",
            reading: "attack_s",
            range: (0.4, f64::INFINITY),
            sets: Vec::new(),
            knob: amp(env::ATTACK),
            travel: (1.0, 4_000.0),
            rises: true,
        },
        Goal {
            word: "snappy",
            reading: "duration_s",
            range: (0.0, 0.8),
            sets: vec![(amp(env::SUSTAIN), 0.0)],
            knob: amp(env::DECAY),
            travel: (5.0, 2_000.0),
            rises: true,
        },
        // Every patch ends here: a quality word that closes the filter or
        // thins the sound shouldn't leave it near silent.
        Goal {
            word: LEVEL,
            reading: "level_db",
            range: (-30.0, -12.0),
            sets: Vec::new(),
            knob: global::MAIN_LEVEL,
            travel: (0.01, 1.4),
            rises: true,
        },
    ]
}

/// Sets `value` for `param`, replacing any value the patch already gave it.
fn set(params: &mut Vec<(ParamId, f64)>, param: ParamId, value: f64) {
    params.retain(|(p, _)| *p != param);
    params.push((param, value));
}

/// Plays `recipe`'s note through `params` and reads the render.
fn render(
    recipe: &Recipe,
    params: &[(ParamId, f64)],
    registry: &Registry,
) -> Result<Readings, String> {
    let patch = Patch {
        name: "text-to-patch",
        params: params.to_vec(),
        routes: Vec::new(),
    };
    let frames = |seconds: f64| (seconds * RATE).round() as usize;
    let script = Script::new(frames(recipe.hold_seconds + TAIL_SECONDS)).note(
        0,
        frames(recipe.hold_seconds) as u32,
        recipe.key,
        VELOCITY,
    );
    let mut instance = Instance::<Reagent>::new()?;
    instance.load_state(&patch.state(registry))?;
    instance.activate(Settings::new(RATE, BLOCK))?;
    let output = instance.render(&script).output;
    features::extract(&output)
        .map(|f| Readings::of(&f))
        .ok_or_else(|| "the patch is silent".to_owned())
}

/// Builds the patch for `description`, and says how each step went.
fn build(description: &str, registry: &Registry) -> Result<Value, String> {
    let lower = description.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .collect();
    let instrument = words
        .iter()
        .find_map(|w| recipe(w))
        .ok_or("no instrument word: say bass, lead, pad or pluck")?;
    let mut params = instrument.params.clone();
    // Reagent's own default, so the level goal starts from where a new patch is.
    params.push((global::MAIN_LEVEL, db_to_gain(-9.0)));
    let mut steps = Vec::new();
    for goal in goals()
        .iter()
        .filter(|g| g.word == LEVEL || words.contains(&g.word))
    {
        for (param, value) in &goal.sets {
            set(&mut params, *param, *value);
        }
        let (mut low, mut high) = goal.travel;
        let mut renders = 0;
        let mut reading = f64::NAN;
        let value_of = |params: &[(ParamId, f64)]| {
            params
                .iter()
                .find(|(p, _)| *p == goal.knob)
                .map(|(_, v)| *v)
        };
        // Start from the recipe's own value for the control.
        let mut knob = value_of(&params).unwrap_or_else(|| (low + high) / 2.0);
        let mut used = knob;
        let mut reached = false;
        while renders < STEPS {
            set(&mut params, goal.knob, knob);
            let readings = render(&instrument, &params, registry)?;
            renders += 1;
            reading = readings
                .get(goal.reading)
                .ok_or("a goal needs a feature the render doesn't have: is the note pitched?")?;
            used = knob;
            if (goal.range.0..=goal.range.1).contains(&reading) {
                reached = true;
                break;
            }
            // A reading too high wants the knob lower if the knob raises it.
            if (reading > goal.range.1) == goal.rises {
                high = knob;
            } else {
                low = knob;
            }
            knob = if low > 0.0 {
                (low * high).sqrt()
            } else {
                (low + high) / 2.0
            };
        }
        steps.push(json!({
            "word": goal.word,
            "reading": goal.reading,
            "value": reading,
            "knob": used,
            "renders": renders,
            "reached": reached,
        }));
    }
    let readings = render(&instrument, &params, registry)?;
    Ok(json!({
        "description": description,
        "recipe": instrument.word,
        "steps": steps,
        "readings": {
            "brightness_oct": readings.brightness_oct,
            "attack_s": readings.attack_s,
            "width": readings.width,
            "pitch_hz": readings.pitch_hz,
            "level_db": readings.level_db,
            "duration_s": readings.duration_s,
        },
        "params": params.iter().map(|(id, value)| json!([id.0, value])).collect::<Vec<_>>(),
    }))
}

/// Whether `result`'s readings fall inside every range in `judge`.
fn judged(result: &Value, judge: &Value) -> Result<Vec<Value>, String> {
    let ranges = judge.as_object().ok_or("a judge is an object of ranges")?;
    ranges
        .iter()
        .map(|(name, range)| {
            let (low, high) = (
                range[0].as_f64().ok_or("a range is [low, high]")?,
                range[1].as_f64().ok_or("a range is [low, high]")?,
            );
            let value = result["readings"][name].as_f64();
            Ok(json!({
                "reading": name,
                "range": [low, high],
                "value": value,
                "passed": value.is_some_and(|v| (low..=high).contains(&v)),
            }))
        })
        .collect()
}

/// Runs the loop on one description, or the evaluation set.
pub fn run(args: &TextToPatchArgs) -> Outcome {
    let registry = Registry::new();
    let (result, passed) = if let Some(description) = &args.description {
        (build(description, &registry)?, true)
    } else {
        let set: Vec<Value> = serde_json::from_str(EVAL).map_err(|e| format!("eval.json: {e}"))?;
        // One thread a case: the renders are the whole cost, and the results
        // are collected in the set's order.
        let results = std::thread::scope(|scope| {
            let cases: Vec<_> = set
                .iter()
                .map(|case| {
                    scope.spawn(|| -> Result<Value, String> {
                        let description = case["description"]
                            .as_str()
                            .ok_or("a case needs a description")?;
                        let mut result = build(description, &registry)?;
                        let judges = judged(&result, &case["judge"])?;
                        result["passed"] = json!(judges.iter().all(|j| j["passed"] == true));
                        result["judges"] = json!(judges);
                        Ok(result)
                    })
                })
                .collect();
            cases
                .into_iter()
                .map(|c| {
                    c.join()
                        .map_err(|_| "a render thread panicked".to_owned())?
                })
                .collect::<Result<Vec<_>, String>>()
        })?;
        let passed = results.iter().filter(|r| r["passed"] == true).count();
        (
            json!({ "passed": passed, "of": results.len(), "results": results }),
            passed == set.len(),
        )
    };
    let text = format!("{result:#}\n");
    match &args.out {
        Some(path) => fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?,
        None => print!("{text}"),
    }
    Ok(if passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
