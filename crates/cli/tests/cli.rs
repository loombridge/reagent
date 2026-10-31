//! `reagent-cli`'s verbs: a render repeats to the bit at any block size and
//! writes a WAV that reads back the same, a wrong preset path fails clearly,
//! `inspect` prints what a preset holds, and the benchmark suite is the one
//! the performance target times.

use std::fs;
use std::path::{Path, PathBuf};

use lspk_plugin::Plugin;
use lspk_test::render::Instance;
use reagent_cli::dataset::{self, DatasetArgs};
use reagent_cli::inspect;
use reagent_cli::render::{self, Phrase, RenderArgs};
use reagent_core::Reagent;
use reagent_core::patches::Patch;
use reagent_core::preset;
use reagent_engine::Registry;

lspk_rt::install_trap!();

/// A file in this process's temporary folder.
fn temporary(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("reagent-cli-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("a temporary folder");
    dir.join(name)
}

/// A preset file named `name` holding `patch`.
fn preset_file(name: &str, patch: Patch) -> PathBuf {
    let path = temporary(name);
    let registry = Registry::new();
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patch.state(&registry))
        .expect("the patch loads");
    instance
        .plugin_mut()
        .run_command(
            preset::SAVE,
            &serde_json::json!({ "path": path.to_string_lossy() }),
        )
        .expect("the patch saves");
    path
}

/// A render of `preset` playing `phrase` in `block`-frame blocks.
fn args(preset: &Path, out: &Path, phrase: Phrase, block: usize) -> RenderArgs {
    RenderArgs {
        preset: preset.to_owned(),
        out: out.to_owned(),
        phrase,
        midi: None,
        rate: 48_000.0,
        block,
    }
}

#[test]
fn a_render_repeats_to_the_bit_run_to_run_and_at_any_block_size() {
    // A patch with unison and modulation, which the golden suite proves is
    // block-size invariant.
    let preset = preset_file("render-blocks.json", reagent_core::patches::pad());
    let out = temporary("blocks.wav");
    let first = render::render(&args(&preset, &out, Phrase::Chord, 512)).expect("a render");
    assert!(first.is_finite() && !first.is_silent());
    for block in [512, 1, 32, 64] {
        let again = render::render(&args(&preset, &out, Phrase::Chord, block)).expect("a render");
        assert!(
            first.same_bits(&again),
            "{block}-frame blocks differ by up to {} dBFS",
            lspk_test::measure::db(first.max_difference(&again))
        );
    }
}

#[test]
fn a_render_writes_a_wav_that_reads_back_as_the_same_audio() {
    let preset = preset_file("render-wav.json", reagent_core::patches::bass());
    let out = temporary("written.wav");
    let args = args(&preset, &out, Phrase::Run, 256);
    assert!(render::run(&args).is_ok());
    let bytes = fs::read(&out).expect("the written file");
    let read = lspk_test::agent::decode_wav(&bytes).expect("a WAV file");
    let rendered = render::render(&args).expect("a render");
    assert!(rendered.same_bits(&read));
}

/// A dataset of `count` patches from `seed`, with audio, in a folder named
/// `name` rendered by `threads` threads.
fn dataset_in(name: &str, seed: u64, count: usize, threads: usize) -> PathBuf {
    let out = temporary(name);
    let args = DatasetArgs {
        out: out.clone(),
        count,
        seed,
        audio: true,
        threads: Some(threads),
    };
    dataset::run(&args).expect("a dataset");
    out
}

#[test]
fn a_dataset_repeats_from_its_seed_whatever_the_thread_count() {
    let one = dataset_in("dataset-one", 7, 6, 1);
    let three = dataset_in("dataset-three", 7, 6, 3);
    let other = dataset_in("dataset-other", 8, 6, 2);
    for file in ["dataset.json", "patches.jsonl", "audio/00000005.wav"] {
        let read = |dir: &Path| fs::read(dir.join(file)).expect("a dataset file");
        assert_eq!(read(&one), read(&three), "{file}");
    }
    let lines = |dir: &Path| fs::read_to_string(dir.join("patches.jsonl")).expect("patches");
    assert_ne!(lines(&one), lines(&other));
}

#[test]
fn every_patch_in_a_dataset_is_playable_and_its_features_match_its_render() {
    let dir = dataset_in("dataset-features", 3, 6, 2);
    let text = fs::read_to_string(dir.join("patches.jsonl")).expect("patches");
    let records: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).expect("a JSON record"))
        .collect();
    assert_eq!(records.len(), 6);
    for (index, record) in records.iter().enumerate() {
        assert_eq!(record["index"], index);
        let features = &record["features"];
        let (peak, rms) = (
            features["peak_db"].as_f64().expect("a peak"),
            features["rms_db"].as_f64().expect("a level"),
        );
        assert!(peak <= 0.0 && rms >= -50.0, "patch {index}: {peak}, {rms}");
        let wav = fs::read(dir.join(format!("audio/{index:08}.wav"))).expect("a render");
        let output = lspk_test::agent::decode_wav(&wav).expect("a WAV file");
        let measured = lspk_test::measure::db(lspk_test::measure::peak(&output.mono()));
        assert!((measured - peak).abs() < 1e-9, "patch {index}");
    }
}

#[test]
fn a_midi_file_plays_its_notes() {
    let preset = preset_file("render-midi.json", reagent_core::patches::bass());
    let out = temporary("midi.wav");
    let midi = midi_file();
    let args = RenderArgs {
        midi: Some(midi),
        ..args(&preset, &out, Phrase::Chord, 512)
    };
    let output = render::render(&args).expect("a render");
    assert!(output.is_finite() && !output.is_silent());
}

#[test]
fn a_wrong_preset_path_fails_with_a_clear_error() {
    let missing = temporary("missing.json");
    let _ = fs::remove_file(&missing);
    let out = temporary("no.wav");
    let error = render::render(&args(&missing, &out, Phrase::Chord, 512)).unwrap_err();
    assert!(error.contains(&missing.display().to_string()), "{error}");
    let error = inspect::inspect(&missing).unwrap_err();
    assert!(error.contains(&missing.display().to_string()), "{error}");
}

#[test]
fn a_sample_rate_outside_the_accepted_range_fails() {
    let preset = temporary("rate.json");
    let out = temporary("rate.wav");
    for rate in [0.0, f64::NAN, f64::INFINITY, 1e12] {
        let mut render_args = args(&preset, &out, Phrase::Chord, 512);
        render_args.rate = rate;
        let error = render::render(&render_args).unwrap_err();
        assert!(error.contains("sample rate"), "{rate}: {error}");
    }
}

#[test]
fn inspect_prints_what_a_preset_holds() {
    let path = preset_file("inspect.json", reagent_core::patches::bass());
    let report = inspect::inspect(&path).expect("inspected");
    assert_eq!(report.format, "reagent-preset");
    assert_eq!(report.schema, lspk_presets::SCHEMA);
    assert!(
        report.modules.contains(&"Osc A".to_owned()),
        "{:?}",
        report.modules
    );
    let text = report.text();
    assert!(text.contains("reagent/basic-shapes"), "{text}");

    // Every value the report names is one `lspk-presets` reads, in the real
    // unit its descriptor gives, and the choices by name.
    let instance = Instance::<Reagent>::new().expect("an instance");
    let preset =
        preset::read(&path, instance.values(), instance.plugin().registry()).expect("the preset");
    let space = reagent_core::params::space();
    let value_of = |id| preset.value(id);
    let mut expected: Vec<(String, String)> = preset
        .values
        .iter()
        .map(|(id, value)| {
            let view = space.view(*id, &value_of).expect("a parameter");
            (view.path.to_string(), view.format(*value))
        })
        .collect();
    expected.sort();
    let mut printed = report.values.clone();
    printed.sort();
    assert!(!expected.is_empty());
    assert_eq!(printed, expected);
}

#[test]
fn the_benchmark_suite_is_the_performance_targets() {
    let ids = reagent_cli::benchmark::suite().ids();
    assert_eq!(ids.len(), 6 * 2 * 4);
    for patch in ["init", "supersaw", "unison16", "bass", "pad", "empty"] {
        for voices in [8, 32] {
            assert!(
                ids.contains(&format!("reagent/{patch}-{voices}/process")),
                "{ids:?}"
            );
        }
    }
}

/// A Standard MIDI File with one note: middle C, on for a quarter note.
fn midi_file() -> PathBuf {
    use midly::num::{u4, u7, u15, u28};
    use midly::{Format, Header, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};

    let note = |delta, message| TrackEvent {
        delta: u28::new(delta),
        kind: TrackEventKind::Midi {
            channel: u4::new(0),
            message,
        },
    };
    let track = vec![
        note(
            0,
            MidiMessage::NoteOn {
                key: u7::new(60),
                vel: u7::new(100),
            },
        ),
        note(
            480,
            MidiMessage::NoteOff {
                key: u7::new(60),
                vel: u7::new(0),
            },
        ),
    ];
    let smf = Smf {
        header: Header {
            format: Format::SingleTrack,
            timing: Timing::Metrical(u15::new(480)),
        },
        tracks: vec![track],
    };
    let path = temporary("one-note.mid");
    let mut bytes = Vec::new();
    smf.write_std(&mut bytes).expect("a MIDI file");
    fs::write(&path, bytes).expect("the MIDI file");
    path
}
