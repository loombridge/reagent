//! `match_sound`: patches Reagent rendered itself are matched back, a search
//! repeats whatever the thread count, and the commands run a search off the
//! calling thread and set what it found.

use std::fs;
use std::sync::atomic::AtomicBool;

use lspk_plugin::Plugin;
use serde_json::{Value, json};

use lspk_test::render::{Instance, Output, Script, Settings};
use reagent_core::Reagent;
use reagent_core::matching::{self, Args, Match, Space};
use reagent_core::patches::Patch;
use reagent_engine::Registry;
use reagent_engine::params::{FILTER, OSC, channel, env, env_id, filter, id, osc};

const RATE: f64 = 48_000.0;
const KEY: u8 = 48;
/// The reference's length and how long it holds the key, in seconds: the
/// search holds for two thirds of the reference when it isn't told.
const LENGTH: f64 = 1.0;
const HOLD: f64 = LENGTH * 2.0 / 3.0;

fn render(patch: &Patch) -> Output {
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patch.state(&Registry::new()))
        .expect("the patch loads");
    instance.activate(Settings::new(RATE, 512)).expect("active");
    let frames = |seconds: f64| (seconds * RATE).round() as usize;
    instance
        .render(&Script::new(frames(LENGTH)).note(0, frames(HOLD) as u32, KEY, 0.8))
        .output
}

/// A point of the search space, from `seed` (xorshift).
fn point(seed: u64, n: usize) -> Vec<f64> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        })
        .collect()
}

fn value(patch: &Patch, param: lspk_params::ParamId) -> f64 {
    patch
        .params
        .iter()
        .find(|(id, _)| *id == param)
        .expect("the patch sets it")
        .1
}

/// A patch from the search space that one oscillator, one voice and the
/// filter make, with no modulation: the kind whose parameters a recording
/// of one note pins down.
fn simple(seed: u64) -> Patch {
    let space = Space::new();
    let mut patch = space.decode(&point(seed, space.len()));
    for (param, to) in [
        (id(OSC, 1, channel::ON), 0.0),
        (id(OSC, 0, osc::UNISON), 1.0),
        (id(FILTER, 0, filter::ON), 1.0),
        (id(OSC, 0, channel::OUTPUT), 0.0),
    ] {
        patch.params.retain(|(id, _)| *id != param);
        patch.params.push((param, to));
    }
    patch.routes.clear();
    patch
}

fn search(reference: &Output, evaluations: usize, threads: Option<usize>) -> Match {
    matching::search(
        reference,
        &Args {
            evaluations: Some(evaluations),
            key: Some(KEY),
            threads,
            ..Args::default()
        },
        &mut |_| {},
        &AtomicBool::new(false),
    )
    .expect("a match")
}

/// A short reference and a small budget, for the tests a debug build runs.
fn small(patch: &Patch) -> Output {
    let frames = |seconds: f64| (seconds * RATE).round() as usize;
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patch.state(&Registry::new()))
        .expect("the patch loads");
    instance.activate(Settings::new(RATE, 512)).expect("active");
    instance
        .render(&Script::new(frames(0.4)).note(0, frames(0.27) as u32, KEY, 0.8))
        .output
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "renders thousands of notes: run it with `cargo nextest run --release`"
)]
fn the_search_recovers_the_parameters_of_patches_reagent_rendered() {
    for seed in 1..=6 {
        let target = simple(seed);
        let found = search(&render(&target), 600, None);
        // The tolerance: the match renders to within 40 % of the Init
        // patch's distance from the reference, plays the same octave and
        // has the release the recording shows to within 15 %.
        assert!(
            found.distance <= 0.4 * found.baseline,
            "seed {seed}: distance {} of {}",
            found.distance,
            found.baseline
        );
        let octave = id(OSC, 0, osc::OCTAVE);
        assert_eq!(
            value(&found.patch, octave),
            value(&target, octave),
            "seed {seed}"
        );
        let release = env_id(0, env::RELEASE);
        let ratio = value(&found.patch, release) / value(&target, release);
        assert!(
            (0.85..=1.15).contains(&ratio),
            "seed {seed}: release off by {ratio}"
        );
    }
}

#[test]
fn a_search_repeats_whatever_the_thread_count_and_beats_the_init_patch() {
    let reference = small(&simple(2));
    let one = search(&reference, 24, Some(1));
    let three = search(&reference, 24, Some(3));
    assert_eq!(one.patch.params, three.patch.params);
    assert_eq!(
        format!("{:?}", one.patch.routes),
        format!("{:?}", three.patch.routes)
    );
    assert_eq!(one.distance, three.distance);
    assert!(
        one.distance < one.baseline,
        "{} of {}",
        one.distance,
        one.baseline
    );
    assert_eq!(one.evaluations, 24);
    assert_eq!((one.key, one.cancelled), (KEY, false));
}

#[test]
fn a_silent_reference_is_refused() {
    let silent = Output::new(RATE, vec![vec![0.0; 4_800]; 2]);
    let error = matching::search(
        &silent,
        &Args::default(),
        &mut |_| {},
        &AtomicBool::new(false),
    )
    .expect_err("silence has nothing to match");
    assert!(error.contains("silent"), "{error}");
}

/// A WAV file in this process's temporary folder holding `output`.
fn wav(name: &str, output: &Output) -> String {
    let dir = std::env::temp_dir().join(format!("reagent-match-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("a temporary folder");
    let path = dir.join(name);
    fs::write(&path, lspk_test::agent::encode_wav(output).expect("a WAV")).expect("a file");
    path.to_string_lossy().into_owned()
}

fn command(instance: &mut Instance<Reagent>, id: &str, args: Value) -> Result<Value, String> {
    instance
        .plugin_mut()
        .run_command(id, &args)
        .map_err(|e| e.message().to_owned())
}

/// Polls the search's status until it ends, and returns it.
fn finished(instance: &mut Instance<Reagent>) -> Value {
    loop {
        let status = command(instance, matching::MATCH_STATUS, json!({})).expect("a status");
        if status["state"] != "running" {
            return status;
        }
        std::thread::yield_now();
    }
}

#[test]
fn the_commands_search_off_the_calling_thread_and_set_what_they_found() {
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    let reference = wav("commands.wav", &small(&simple(2)));
    // Nothing to read or set before a search.
    for id in [
        matching::MATCH_STATUS,
        matching::MATCH_APPLY,
        matching::MATCH_CANCEL,
    ] {
        assert!(command(&mut instance, id, json!({})).is_err(), "{id}");
    }
    let missing = command(
        &mut instance,
        matching::MATCH_SOUND,
        json!({ "reference": "/nonexistent/reference.wav" }),
    );
    assert!(missing.expect_err("no such file").contains("reference.wav"));

    // A silent recording ends the search as failed, with the reason.
    let silent = wav("silent.wav", &Output::new(RATE, vec![vec![0.0; 4_800]; 2]));
    command(
        &mut instance,
        matching::MATCH_SOUND,
        json!({ "reference": silent }),
    )
    .expect("started");
    let failed = finished(&mut instance);
    assert_eq!(failed["state"], "failed", "{failed}");
    assert!(
        failed["error"]
            .as_str()
            .is_some_and(|e| e.contains("silent")),
        "{failed}"
    );
    assert!(command(&mut instance, matching::MATCH_APPLY, json!({})).is_err());

    // It returns with the search running, so the caller isn't held.
    let args = json!({ "reference": reference, "evaluations": 40, "key": KEY });
    let started = command(&mut instance, matching::MATCH_SOUND, args.clone()).expect("started");
    assert_eq!(started["budget"], 40);
    assert!(started["result"].is_null());
    if started["state"] == "running" {
        let again = command(&mut instance, matching::MATCH_SOUND, args.clone());
        assert!(again.expect_err("one at a time").contains("running"));
    }
    let status = finished(&mut instance);
    assert_eq!(status["state"], "done", "{status}");
    assert_eq!(status["evaluations"], 40);
    let report = &status["result"]["report"];
    assert!(
        report["distance"].as_f64() < report["baseline"].as_f64(),
        "{report}"
    );
    assert_eq!(status["best_distance"], report["distance"]);
    assert!((0.0..=1.0).contains(&report["closeness"].as_f64().expect("a closeness")));

    // Applying sets the found patch: each parameter it names, as it names it.
    command(&mut instance, matching::MATCH_APPLY, json!({})).expect("applied");
    let cutoff = id(FILTER, 0, filter::CUTOFF);
    let found = status["result"]["patch"]["params"]
        .as_array()
        .expect("parameters")
        .iter()
        .find(|p| p["id"] == cutoff.0)
        .expect("the cutoff");
    assert_eq!(found["name"], "Filter 1 Cutoff");
    let set = instance.host().param(cutoff).expect("a value");
    assert!((set - found["value"].as_f64().expect("a value")).abs() < 1e-6);
}

#[test]
fn a_cancelled_search_keeps_its_best_patch() {
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    let reference = wav("cancel.wav", &small(&simple(5)));
    let args = json!({ "reference": reference, "evaluations": 1_000_000, "key": KEY });
    command(&mut instance, matching::MATCH_SOUND, args).expect("started");
    command(&mut instance, matching::MATCH_CANCEL, json!({})).expect("cancelled");
    let status = finished(&mut instance);
    assert_eq!(status["state"], "cancelled", "{status}");
    assert!(status["evaluations"].as_u64() < Some(1_000));
    assert!(status["result"]["patch"]["params"].is_array());
    command(&mut instance, matching::MATCH_APPLY, json!({})).expect("a partial patch applies");
}
