//! R1's exit demo, replayed: an agent, headless through `reagent-cli mcp`,
//! built a bass, a pad and a lead from text descriptions. The command
//! transcripts it left in `evidence/r1-exit-demo/` replay through the kit's
//! scripted client from the empty patch to each render, which null-tests
//! against the render kept beside them; the preset each replay saves matches
//! the kept preset; every command a transcript runs is registered; and each
//! kept preset loads as a native device and plays a note.
//!
//! The renders are this crate's golden suite, so `cargo xtask golden
//! --update` remakes them, and the kept presets and analyses with them.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use lspk_test::agent::{Client, Stdio, audio};
use lspk_test::golden::{Suite, UPDATE_ENV};
use lspk_test::render::Output;
use reagent_cli::render::{self, Phrase, RenderArgs};
use serde_json::{Value, json};

lspk_rt::install_trap!();

/// The three patches, in the order the demo built them.
const PATCHES: [&str; 3] = ["bass", "pad", "lead"];

fn evidence() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/r1-exit-demo")
}

/// A file in this process's temporary folder.
fn temporary(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("reagent-exit-demo-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("a temporary folder");
    dir.join(name)
}

/// Whether `cargo xtask golden --update` asked for the kept files to be remade.
fn updating() -> bool {
    std::env::var_os(UPDATE_ENV).is_some()
}

fn transcript(name: &str) -> Value {
    let path = evidence().join(format!("{name}.transcript.json"));
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).expect("a transcript is JSON")
}

/// What a replay left: the patch's saved preset, and the last preview's
/// analysis and audio.
struct Replay {
    preset: String,
    analysis: Value,
    audio: Output,
}

/// Puts `path` where a transcript says `{preset}`.
fn with_preset(value: &mut Value, path: &str) {
    match value {
        Value::String(text) if text == "{preset}" => *text = path.to_owned(),
        Value::Array(items) => items.iter_mut().for_each(|v| with_preset(v, path)),
        Value::Object(map) => map.values_mut().for_each(|v| with_preset(v, path)),
        _ => {}
    }
}

fn start(autonomous: bool) -> Client<Stdio> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_reagent-cli"));
    command.arg("mcp");
    if autonomous {
        command.arg("--autonomous");
    }
    let mut client = Client::new(Stdio::spawn(&mut command).expect("reagent-cli mcp starts"));
    client.initialize("reagent-exit-demo").expect("initialised");
    client
}

/// Plays `name`'s transcript to a new `reagent-cli mcp`, every call as
/// recorded.
fn replay(name: &str) -> Replay {
    let saved = temporary(&format!("{name}.json"));
    let _ = fs::remove_file(&saved);
    let mut client = start(true);
    let mut last = None;
    for step in transcript(name)["steps"].as_array().expect("steps") {
        let tool = step["tool"].as_str().expect("a tool");
        let mut args = step["args"].clone();
        with_preset(&mut args, &saved.to_string_lossy());
        let result = client.call(tool, args).expect("a reply");
        let analysis = result
            .clone()
            .ok()
            .unwrap_or_else(|e| panic!("{name}: {tool} failed: {e}"));
        if tool == "preview" {
            last = Some((analysis, audio(&result).expect("a preview's audio")));
        }
    }
    let (analysis, audio) = last.expect("a transcript hears its patch");
    Replay {
        preset: fs::read_to_string(&saved).expect("the transcript saves the preset"),
        analysis,
        audio,
    }
}

/// Every command ID `transcript` runs that isn't in `registered`.
fn unregistered(transcript: &Value, registered: &BTreeSet<String>) -> Vec<String> {
    let steps = transcript["steps"].as_array().expect("steps");
    let used = steps
        .iter()
        .filter(|step| step["tool"] == "run")
        .flat_map(|step| step["args"]["commands"].as_array().into_iter().flatten())
        .filter_map(|command| command["command"].as_str());
    used.filter(|id| !registered.contains(*id))
        .map(str::to_owned)
        .collect()
}

/// Every command Reagent registers, as an agent lists them.
fn registered() -> BTreeSet<String> {
    let mut client = start(false);
    let mut list = |args| client.call("list_commands", args).expect("a reply").ok();
    let areas = list(json!({})).expect("the areas");
    areas["areas"]
        .as_object()
        .expect("areas")
        .keys()
        .flat_map(|area| {
            let listed = list(json!({ "area": area })).expect("an area's commands");
            listed["commands"]
                .as_array()
                .expect("commands")
                .iter()
                .map(|c| c["id"].as_str().expect("an ID").to_owned())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The numbers the description of each patch promises, from the analysis.
fn matches_its_description(name: &str, analysis: &Value) {
    let centroid = analysis["spectrum"]["centroid_hz"]
        .as_f64()
        .expect("centroid");
    let over_time: Vec<f64> = analysis["spectrum"]["centroid_over_time_hz"]
        .as_array()
        .expect("centroids")
        .iter()
        .filter_map(Value::as_f64)
        .collect();
    let width = analysis["stereo"]["width"].as_f64().expect("width");
    let (low, high) = over_time
        .iter()
        .fold((f64::MAX, 0.0f64), |(l, h), c| (l.min(*c), h.max(*c)));
    match name {
        "bass" => assert!(centroid < 300.0, "a deep bass: centroid {centroid} Hz"),
        "pad" => {
            assert!(width > 0.2, "a wide pad: stereo width {width}");
            assert!(high > low * 1.3, "a pad that breathes: {over_time:?}");
        }
        "lead" => {
            assert!(
                (800.0..3_000.0).contains(&centroid),
                "a bright lead: {centroid} Hz"
            );
            assert!(width > 0.1, "a lead with some width: {width}");
        }
        _ => unreachable!("{name}"),
    }
}

#[test]
fn the_transcripts_replay_from_an_empty_patch_to_their_renders() {
    let mut suite = Suite::new(evidence().join("renders"));
    for name in PATCHES {
        let replayed = replay(name);
        assert!(replayed.audio.is_finite() && !replayed.audio.is_silent());
        matches_its_description(name, &replayed.analysis);

        let preset = evidence().join(format!("{name}.preset.json"));
        let analysis = evidence().join(format!("{name}.analysis.json"));
        if updating() {
            fs::write(&preset, &replayed.preset).expect("the preset is writable");
            let text = serde_json::to_string_pretty(&replayed.analysis).expect("JSON");
            fs::write(&analysis, text + "\n").expect("the analysis is writable");
        }
        // Parsed, so a checkout that turns line endings into CRLF still matches.
        let parse = |text: &str| serde_json::from_str::<Value>(text).expect("a preset is JSON");
        assert_eq!(
            parse(&replayed.preset),
            parse(&fs::read_to_string(&preset).expect("the kept preset")),
            "{name}'s transcript no longer builds its kept preset"
        );
        suite.add(name, replayed.audio);
    }
    suite.assert();
}

#[test]
fn every_command_the_transcripts_run_is_registered() {
    let registered = registered();
    assert!(registered.contains("params.set"), "{registered:?}");
    for name in PATCHES {
        let missing = unregistered(&transcript(name), &registered);
        assert!(
            missing.is_empty(),
            "{name} runs unregistered commands: {missing:?}"
        );
    }
}

#[test]
fn a_command_that_is_not_registered_fails_the_coverage_check() {
    let registered = registered();
    let made_up = json!({ "steps": [
        { "tool": "run", "args": { "commands": [
            { "command": "params.set" }, { "command": "synth.make_louder" } ] } },
        { "tool": "preview", "args": {} },
    ] });
    assert_eq!(unregistered(&made_up, &registered), ["synth.make_louder"]);
}

// This crate uses `reagent-core`, Reagent without its exports and editor,
// as Beatlayer's native devices do, so loading a preset here is loading it
// as a native device: each kept preset plays notes.
#[test]
fn each_kept_preset_loads_as_a_native_device_and_plays_a_note() {
    for name in PATCHES {
        let args = RenderArgs {
            preset: evidence().join(format!("{name}.preset.json")),
            out: temporary(&format!("{name}.wav")),
            phrase: Phrase::Run,
            midi: None,
            rate: 48_000.0,
            block: 512,
        };
        let output = render::render(&args).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            output.is_finite() && !output.is_silent(),
            "{name} is silent"
        );
    }
}
