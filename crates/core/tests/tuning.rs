//! Tuning and preset stepping through the plug-in, rendered and measured:
//! the pitch a note sounds at under a Scala scale, a `.tun` file and an
//! MTS-ESP master (a stand-in), and the presets the controllers and keys
//! step through.

use std::f64::consts::LN_2;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use lspk_plugin::{Event, Note, Plugin};
use lspk_test::measure;
use lspk_test::render::{Instance, Script, Settings};
use reagent_core::{Reagent, preset};
use reagent_engine::params::global;
use reagent_engine::tuning::Master;
use serde_json::json;

const RATE: f64 = 48_000.0;

/// A file in this process's temporary folder, under `test`.
fn file(test: &str, name: &str, text: &str) -> String {
    let dir = std::env::temp_dir().join(format!("reagent-tuning-{}-{test}", std::process::id()));
    fs::create_dir_all(&dir).expect("a temporary folder");
    let path = dir.join(name);
    fs::write(&path, text).expect("a file");
    path.to_string_lossy().into_owned()
}

fn run(instance: &mut Instance<Reagent>, command: &str, args: serde_json::Value) {
    instance
        .plugin_mut()
        .run_command(command, &args)
        .unwrap_or_else(|e| panic!("{command}: {}", e.message()));
}

fn active() -> Instance<Reagent> {
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    // A standing-in master that isn't connected, so a machine with the real
    // library installed doesn't change what a test hears.
    instance
        .plugin_mut()
        .use_master(Arc::new(Standin::offline()));
    instance
}

/// The pitch, in hertz, of key `key` held for a second, or `None` for
/// silence.
fn pitch(instance: &mut Instance<Reagent>, key: u8) -> Option<f64> {
    if !instance.is_active() {
        instance.activate(Settings::new(RATE, 512)).expect("active");
    }
    let frames = RATE as usize;
    // Choked at the end, so no voice rings into the next note measured.
    let out = instance.render(
        &Script::new(frames)
            .note_on(0, key, 1.0)
            .choke(frames as u32 - 1, key),
    );
    let held = out.output.mono()[frames / 4..].to_vec();
    if measure::peak(&held) < 1e-4 {
        return None;
    }
    Some(measure::pitch(&held, RATE).expect("a pitched sound"))
}

/// The hertz of key `key` in equal temperament.
fn equal(key: u8) -> f64 {
    440.0 * ((f64::from(key) - 69.0) / 12.0).exp2()
}

fn near(got: Option<f64>, want: f64) {
    let got = got.expect("a sound");
    let cents = 1200.0 * (got / want).ln() / LN_2;
    assert!(
        cents.abs() < 1.5,
        "{got} Hz where {want} Hz, {cents} cents out"
    );
}

const PENTATONIC: &str = "Pentatonic\n5\n9/8\n5/4\n3/2\n5/3\n2/1\n";
const FIVE_KEYS: &str = "5\n0\n127\n60\n60\n261.625565\n5\n0\n1\n2\n3\n4\n";

// Scala `.scl` and `.kbm`.
#[test]
fn a_scala_scale_and_its_mapping_retune_the_keys() {
    let mut instance = active();
    near(pitch(&mut instance, 62), equal(62));
    run(
        &mut instance,
        "tuning.load",
        json!({
            "path": file("scala", "five.scl", PENTATONIC),
            "keymap": file("scala", "five.kbm", FIVE_KEYS),
        }),
    );
    let tonic = 261.625565;
    for (key, ratio) in [
        (60, 1.0),
        (61, 9.0 / 8.0),
        (64, 5.0 / 3.0),
        (66, 2.25),
        (55, 0.5),
    ] {
        near(pitch(&mut instance, key), tonic * ratio);
    }
}

// Scala `.scl`; a scale with no mapping starts at
// middle C with A4 at 440 Hz.
#[test]
fn a_scale_alone_starts_at_middle_c_and_keeps_a_four_at_440() {
    let mut instance = active();
    // Two degrees to the octave, a fifth then the octave. A4, nine keys above
    // middle C, is the reference and sounds at 440 Hz; the keys above it
    // follow the scale.
    let scl = "Two\n2\n3/2\n2/1\n";
    run(
        &mut instance,
        "tuning.load",
        json!({ "path": file("alone", "two.scl", scl) }),
    );
    near(pitch(&mut instance, 69), 440.0);
    near(pitch(&mut instance, 70), 440.0 * 2f64.powf(498.04 / 1200.0));
    near(pitch(&mut instance, 71), 880.0);
    near(pitch(&mut instance, 72), 880.0 * 2f64.powf(498.04 / 1200.0));
}

// Per-instance `.tun` file.
#[test]
fn a_tun_file_retunes_the_keys_it_names() {
    let mut instance = active();
    // Key 69 a quarter tone sharp, key 70 a quarter tone flat.
    let tun = "[Tuning]\nnote 69=6950\nnote 70=6950\n";
    run(
        &mut instance,
        "tuning.load",
        json!({ "path": file("tun", "q.tun", tun) }),
    );
    near(pitch(&mut instance, 69), equal(69) * 2f64.powf(0.5 / 12.0));
    near(pitch(&mut instance, 70), equal(69) * 2f64.powf(0.5 / 12.0));
    near(pitch(&mut instance, 71), equal(71));
    run(&mut instance, "tuning.clear", json!({}));
    near(pitch(&mut instance, 69), equal(69));
}

// Main Tuning and concert pitch still apply on top.
#[test]
fn reference_pitch_still_applies_to_a_tuned_key() {
    let mut instance = active();
    let tun = "[Tuning]\nnote 69=6950\n";
    run(
        &mut instance,
        "tuning.load",
        json!({ "path": file("reference", "q.tun", tun) }),
    );
    instance.activate(Settings::new(RATE, 512)).expect("active");
    instance
        .host()
        .edit(global::REFERENCE_PITCH, 450.0)
        .expect("an edit");
    near(
        pitch(&mut instance, 69),
        equal(69) * 2f64.powf(0.5 / 12.0) * 450.0 / 440.0,
    );
}

// Keys the mapping leaves out play nothing.
#[test]
fn a_key_the_mapping_leaves_out_plays_nothing() {
    let mut instance = active();
    let scl = file("gap", "twelve.scl", &twelve());
    let kbm = file(
        "gap",
        "gap.kbm",
        "3\n0\n127\n60\n60\n261.625565\n12\n0\nx\n7\n",
    );
    run(
        &mut instance,
        "tuning.load",
        json!({ "path": scl, "keymap": kbm }),
    );
    assert!(pitch(&mut instance, 60).is_some());
    assert!(pitch(&mut instance, 61).is_none());
    assert!(pitch(&mut instance, 62).is_some());
}

fn twelve() -> String {
    let mut text = "Twelve\n12\n".to_owned();
    for n in 1..12 {
        text += &format!("{n}00.0\n");
    }
    text + "2/1\n"
}

/// A stand-in for an MTS-ESP master: every key a number of cents off equal
/// temperament, held as atomics so a test can change it under a note.
struct Standin {
    online: AtomicBool,
    cents: AtomicU64,
    /// A key it asks clients to leave silent, or 255 for none.
    silent: AtomicU64,
}

impl Standin {
    fn offline() -> Self {
        let standin = Self::offset(0.0);
        standin.online.store(false, Ordering::Relaxed);
        standin
    }

    fn offset(cents: f64) -> Self {
        Self {
            online: AtomicBool::new(true),
            cents: AtomicU64::new(cents.to_bits()),
            silent: AtomicU64::new(255),
        }
    }

    fn retune(&self, cents: f64) {
        self.cents.store(cents.to_bits(), Ordering::Relaxed);
    }
}

impl Master for Standin {
    fn online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    fn frequency(&self, key: u8) -> f64 {
        equal(key) * (f64::from_bits(self.cents.load(Ordering::Relaxed)) / 1200.0).exp2()
    }

    fn filtered(&self, key: u8) -> bool {
        u64::from(key) == self.silent.load(Ordering::Relaxed)
    }
}

fn with_master(master: &Arc<Standin>) -> Instance<Reagent> {
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance.plugin_mut().use_master(master.clone());
    instance
}

// MTS-ESP, on by default.
#[test]
fn an_mts_esp_master_retunes_by_default_and_a_file_overrides_it() {
    let master = Arc::new(Standin::offset(100.0));
    let mut instance = with_master(&master);
    near(pitch(&mut instance, 60), equal(61));
    // The client can be turned off.
    run(&mut instance, "tuning.set_mts", json!({ "enabled": false }));
    near(pitch(&mut instance, 60), equal(60));
    run(&mut instance, "tuning.set_mts", json!({ "enabled": true }));
    near(pitch(&mut instance, 60), equal(61));
    // A file takes precedence over the master.
    let tun = "[Tuning]\nnote 60=6000\n";
    run(
        &mut instance,
        "tuning.load",
        json!({ "path": file("override", "plain.tun", tun) }),
    );
    near(pitch(&mut instance, 60), equal(60));
    run(&mut instance, "tuning.clear", json!({}));
    near(pitch(&mut instance, 60), equal(61));
}

// The master's keys that it asks to leave silent.
#[test]
fn a_key_the_master_filters_plays_nothing() {
    let master = Arc::new(Standin::offset(0.0));
    master.silent.store(64, Ordering::Relaxed);
    let mut instance = with_master(&master);
    assert!(pitch(&mut instance, 64).is_none());
    near(pitch(&mut instance, 65), equal(65));
}

/// Blocks of one held note, the master retuned by `to` cents after
/// `retune_at` of them; the pitch of the blocks before and after.
fn held_note_while_the_master_retunes(note_on_only: bool) -> (f64, f64) {
    let master = Arc::new(Standin::offset(0.0));
    let mut instance = with_master(&master);
    run(
        &mut instance,
        "tuning.set_mts",
        json!({ "enabled": true, "note_on_only": note_on_only }),
    );
    instance.activate(Settings::new(RATE, 512)).expect("active");
    let on = Event::NoteOn {
        time: 0,
        note: Note::key(0, 60),
        velocity: 1.0,
    };
    let mut heard = Vec::new();
    for block in 0..96 {
        if block == 48 {
            master.retune(100.0);
        }
        let events = if block == 0 { vec![on] } else { vec![] };
        heard.extend(instance.process(512, &[], &events).outputs[0].clone());
    }
    let measure_at = |from: usize, to: usize| {
        measure::pitch(&heard[from * 512..to * 512], RATE).expect("a pitched sound")
    };
    (measure_at(16, 44), measure_at(64, 96))
}

// MTS-ESP note-on only: a held note's tuning doesn't
// change when the global tuning updates.
#[test]
fn the_note_on_only_option_keeps_a_held_notes_pitch() {
    let (before, after) = held_note_while_the_master_retunes(true);
    near(Some(before), equal(60));
    near(Some(after), equal(60));
    let (before, after) = held_note_while_the_master_retunes(false);
    near(Some(before), equal(60));
    near(Some(after), equal(61));
}

/// Presets `a`, `b` and `c` in a folder of their own, `a` loaded; `tuning`
/// is the main tuning each is saved with.
fn folder_of_presets(test: &str) -> (Instance<Reagent>, Vec<String>) {
    let mut instance = active();
    instance.activate(Settings::new(RATE, 512)).expect("active");
    let mut paths = Vec::new();
    for name in ["a", "b", "c"] {
        let path = file(test, &format!("{name}.json"), "");
        run(&mut instance, preset::SAVE, json!({ "path": path }));
        paths.push(path);
    }
    // Something else beside them, which stepping doesn't touch.
    file(test, "notes.txt", "not a preset");
    run(&mut instance, preset::LOAD, json!({ "path": paths[0] }));
    (instance, paths)
}

/// The preset `preset.step` by `steps` loaded.
fn stepped(instance: &mut Instance<Reagent>, steps: i32) -> String {
    let result = instance
        .plugin_mut()
        .run_command(preset::STEP, &json!({ "steps": steps }))
        .expect("a step");
    result["path"].as_str().expect("a path").to_owned()
}

// Stepping goes through the folder of the last preset.
#[test]
fn stepping_goes_through_the_folder_and_wraps_round() {
    let (mut instance, paths) = folder_of_presets("folder");
    assert_eq!(stepped(&mut instance, 1), paths[1]);
    assert_eq!(stepped(&mut instance, 1), paths[2]);
    assert_eq!(stepped(&mut instance, 1), paths[0]);
    assert_eq!(stepped(&mut instance, -1), paths[2]);
    assert_eq!(stepped(&mut instance, 0), paths[2]);
}

fn midi(number: u8, value: u8) -> Event {
    Event::Midi {
        time: 0,
        port: 0,
        data: [0xB0, number, value],
    }
}

fn note(key: u8) -> Event {
    Event::NoteOn {
        time: 0,
        note: Note::key(0, key),
        velocity: 1.0,
    }
}

// Stepping by controller: 64 or more steps, and the
// controller must fall below 64 before it steps again.
#[test]
fn a_controller_steps_presets_once_a_press() {
    let (mut instance, paths) = folder_of_presets("controller");
    run(
        &mut instance,
        "preset.set_stepping",
        json!({ "mode": "controller", "forward": 21, "backward": 22 }),
    );
    let blocks = [
        vec![midi(21, 127), midi(21, 127), midi(21, 0), midi(21, 100)],
        vec![midi(22, 127)],
    ];
    for events in blocks {
        instance.process(512, &[], &events);
    }
    // Two forward steps and one back, taken on the main thread.
    instance.on_main_thread();
    assert_eq!(stepped(&mut instance, 0), paths[1]);
    // A controller nothing is assigned to steps nothing.
    instance.process(512, &[], &[midi(23, 127)]);
    instance.on_main_thread();
    assert_eq!(stepped(&mut instance, 0), paths[1]);
}

// Stepping by note.
#[test]
fn a_key_steps_presets_and_does_not_sound() {
    let (mut instance, paths) = folder_of_presets("note");
    run(
        &mut instance,
        "preset.set_stepping",
        json!({ "mode": "note", "forward": 0, "backward": 1 }),
    );
    let block = instance.process(512, &[], &[note(0)]);
    let mut heard = block.outputs[0].clone();
    heard.extend(instance.process(512, &[], &[]).outputs[0].clone());
    assert!(measure::peak(&heard) < 1e-6);
    instance.on_main_thread();
    assert_eq!(stepped(&mut instance, 0), paths[1]);
    // Other keys sound.
    instance.process(512, &[], &[note(60)]);
    let block = instance.process(512, &[], &[]);
    assert!(measure::peak(&block.outputs[0]) > 1e-3);
}

// The tuning lock: a preset load leaves the tuning
// file and the concert pitch alone.
#[test]
fn the_lock_keeps_the_tuning_file_and_reference_pitch_through_a_preset_load() {
    let tun = file("lock", "q.tun", "[Tuning]\nnote 69=6950\n");
    let mut instance = active();
    instance.activate(Settings::new(RATE, 512)).expect("active");
    let plain = file("lock", "plain.json", "");
    run(&mut instance, preset::SAVE, json!({ "path": plain }));
    run(&mut instance, "tuning.load", json!({ "path": tun }));
    let tuned = file("lock", "tuned.json", "");
    run(&mut instance, preset::SAVE, json!({ "path": tuned }));

    // Unlocked, a preset brings its own tuning with it: none, then the file.
    run(&mut instance, preset::LOAD, json!({ "path": plain }));
    near(pitch(&mut instance, 69), equal(69));
    run(&mut instance, preset::LOAD, json!({ "path": tuned }));
    near(pitch(&mut instance, 69), equal(69) * 2f64.powf(0.5 / 12.0));

    // Locked, loading the plain preset leaves the file in place.
    run(&mut instance, "tuning.lock", json!({ "locked": true }));
    run(&mut instance, preset::LOAD, json!({ "path": plain }));
    near(pitch(&mut instance, 69), equal(69) * 2f64.powf(0.5 / 12.0));
    // Nor does it take the preset's reference pitch (440 Hz).
    instance
        .host()
        .edit(global::REFERENCE_PITCH, 450.0)
        .expect("an edit");
    run(&mut instance, preset::LOAD, json!({ "path": plain }));
    near(
        pitch(&mut instance, 69),
        equal(69) * 2f64.powf(0.5 / 12.0) * 450.0 / 440.0,
    );
    instance
        .host()
        .edit(global::REFERENCE_PITCH, 440.0)
        .expect("an edit");
    // And the saved state keeps the setting and the file.
    let state = instance.save_state();
    let mut again = active();
    again.load_state(&state).expect("the state loads");
    near(pitch(&mut again, 69), equal(69) * 2f64.powf(0.5 / 12.0));
    run(&mut again, preset::LOAD, json!({ "path": plain }));
    near(pitch(&mut again, 69), equal(69) * 2f64.powf(0.5 / 12.0));
}

// An earlier build's state was the matrix alone.
#[test]
fn a_state_from_before_tuning_still_loads() {
    let mut instance = active();
    instance
        .plugin_mut()
        .load_state(br#"{"version":1,"slots":[],"locked":false}"#)
        .expect("the old state loads");
    near(pitch(&mut instance, 60), equal(60));
}

// A preset made with a tuning file still loads where the file isn't, in equal
// temperament.
#[test]
fn a_preset_whose_tuning_file_is_missing_still_loads() {
    let mut instance = active();
    let tun = file("missing", "q.tun", "[Tuning]\nnote 69=6950\n");
    run(&mut instance, "tuning.load", json!({ "path": tun }));
    let tuned = file("missing", "tuned.json", "");
    run(&mut instance, preset::SAVE, json!({ "path": tuned }));
    fs::remove_file(&tun).expect("the file goes");
    run(&mut instance, preset::LOAD, json!({ "path": tuned }));
    near(pitch(&mut instance, 69), equal(69));
}
