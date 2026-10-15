//! Reagent's control-rate rule, measured through the modulation engine
//! (engine-core R1.13): a route the ear can't follow runs at the control
//! rate, and its ramp stays within a period's change of the audio-rate
//! version, while a route that reads an oscillator stays at audio rate
//! sample for sample.
//!
//! Spec rules are cited as `engine-core Rn`.

use lspk_mod::{Config, Engine, Matrix, NoteOn, Output, Polarity, Rate, Signals, Target};
use reagent_engine::Registry;
use reagent_engine::params::{FILTER, OSC, filter, global, id, osc};

const RATE: f32 = 48_000.0;
const VOICES: usize = 4;
const BLOCK: usize = 512;

fn config() -> Config {
    Config {
        sample_rate: RATE,
        max_frames: BLOCK,
        voices: VOICES,
    }
}

/// A per-voice signal of `frames` frames, `f(frame, voice)`.
fn signal(frames: usize, f: impl Fn(usize, usize) -> f32) -> Vec<f32> {
    let mut data = vec![0.0; frames * VOICES];
    for k in 0..frames {
        for v in 0..VOICES {
            data[k * VOICES + v] = f(k, v);
        }
    }
    data
}

/// `registry`'s engine running `matrix`, with a note on every voice.
fn playing(registry: &Registry, matrix: &Matrix) -> Engine {
    let mut engine = Engine::new(&registry.sources, &registry.destinations, config());
    engine.swap(Box::new(registry.compile(matrix, config())));
    for v in 0..VOICES {
        engine.note_on(v, NoteOn::new(60.0, 1.0));
    }
    engine
}

// Spec: engine-core R1.13 (a route measured inaudible at the control rate
// runs at it; its ramp lags the audio-rate version by at most the route's
// change over two control periods).
#[test]
fn a_slow_control_route_matches_its_audio_rate_version() {
    let registry = Registry::new();
    // The same source, amount and polarity into an audio-rate destination
    // (the octave, R1) and a control-rate one (the cutoff, R51).
    let audio = Target::Param(id(OSC, 0, osc::OCTAVE));
    let control = Target::Param(id(FILTER, 0, filter::CUTOFF));
    let source = registry.sources.find("lfo1").expect("the standard sources");
    let mut matrix = Matrix::new();
    matrix.add(source, audio, 1.0, Polarity::Unipolar).unwrap();
    matrix
        .add(source, control, 1.0, Polarity::Unipolar)
        .unwrap();
    let mut engine = playing(&registry, &matrix);
    let audio = registry.dest(audio).expect("a destination");
    let control = registry.dest(control).expect("a destination");
    assert_eq!(engine.program().rate(audio), Some(Rate::Audio));
    assert_eq!(engine.program().rate(control), Some(Rate::Control));

    // A 2 Hz sine over the whole travel: slow enough that the ear can't
    // tell the difference.
    let slow = |k: usize| 0.5 + 0.5 * (std::f32::consts::TAU * 2.0 * k as f32 / RATE).sin();
    let mut worst = 0.0f32;
    for block in 0..40 {
        let data = signal(BLOCK, |k, _| slow(block * BLOCK + k));
        let mut signals = Signals::new();
        signals.bind(source, &data, VOICES);
        engine.process(&signals, 0..BLOCK);
        let Output::Audio(exact) = engine.output(audio) else {
            panic!("the octave is audio rate")
        };
        let Output::Control(ramped) = engine.output(control) else {
            panic!("the cutoff is control rate")
        };
        let mut ramp = vec![0.0; BLOCK];
        ramped.ramp(0, &mut ramp);
        if block > 0 {
            for (k, value) in ramp.iter().enumerate() {
                worst = worst.max((value - exact.at(k, 0)).abs());
            }
        }
    }
    // A full-travel 2 Hz sine moves by under 0.0084 of the travel over two
    // 32-frame periods at 48 kHz; allow a little slack.
    assert!(worst < 0.01, "the control route differs by {worst}");
}

// Spec: engine-core R1.13 (a route reading an oscillator, the sub, the
// noise or a filter stays at audio rate), R27 to R31 (the audio sources).
#[test]
fn an_audio_route_is_unchanged_sample_for_sample() {
    let registry = Registry::new();
    // Main tuning is unbounded (R137), so the route's value reaches the
    // output exactly: an oscillator into the pitch, the audio-rate case.
    let pitch = Target::Param(global::TUNING);
    let source = registry
        .sources
        .find("osc_a")
        .expect("the standard sources");
    let mut matrix = Matrix::new();
    matrix.add(source, pitch, 1.0, Polarity::Bipolar).unwrap();
    let mut engine = playing(&registry, &matrix);
    let pitch = registry.dest(pitch).expect("a destination");
    assert_eq!(engine.program().rate(pitch), Some(Rate::Audio));

    let osc = signal(BLOCK, |k, v| ((k + v * 7) as f32 * 0.01).sin());
    let mut signals = Signals::new();
    signals.bind(source, &osc, VOICES);
    engine.process(&signals, 0..BLOCK);
    let Output::Audio(out) = engine.output(pitch) else {
        panic!("the pitch is audio rate")
    };
    for k in 0..BLOCK {
        for v in 0..VOICES {
            assert_eq!(
                out.at(k, v),
                osc[k * VOICES + v],
                "frame {k}, voice {v} is unchanged"
            );
        }
    }
}

// Spec: engine-core R1.13 (a route reading an oscillator runs an adaptive
// destination at audio rate, and a control route otherwise).
#[test]
fn an_audio_source_raises_an_adaptive_destination_to_audio_rate() {
    let registry = Registry::new();
    let frame = Target::Param(id(OSC, 0, osc::FRAME));
    let lfo = registry.sources.find("lfo1").expect("the standard sources");
    let osc = registry
        .sources
        .find("osc_a")
        .expect("the standard sources");
    let mut matrix = Matrix::new();
    matrix.add(lfo, frame, 0.5, Polarity::Unipolar).unwrap();
    let engine = playing(&registry, &matrix);
    let dest = registry.dest(frame).expect("a destination");
    assert_eq!(engine.program().rate(dest), Some(Rate::Control));

    // A second route into the same destination reads an oscillator: the
    // destination runs at audio rate.
    matrix.add(osc, frame, 0.5, Polarity::Unipolar).unwrap();
    let engine = playing(&registry, &matrix);
    assert_eq!(engine.program().rate(dest), Some(Rate::Audio));
}
