//! Reagent measured through its whole voice path: pitch at every octave
//! and tuning control, the level laws of a channel and Main, the filter's
//! response on the oscillator's harmonics, digital silence when idle, and
//! every hand-built patch at a sensible level.
//!
//! Spec rules are cited as `engine-core Rn`.

use lspk_params::{ParamId, db_to_gain};
use lspk_test::measure::{self, db};
use lspk_test::render::{Instance, Output, Script, Settings};
use reagent::Reagent;
use reagent::params::{FILTER, OSC, channel, filter, global, id, osc};
use reagent::patches::{self, Patch};
use reagent_engine::Registry;

lspk_rt::install_trap!();

const RATE: f64 = 48_000.0;

/// `patch` with `params` on top, holding `key` for `seconds`.
fn hold(patch: &Patch, params: &[(ParamId, f64)], key: u8, seconds: f64) -> Output {
    let registry = Registry::new();
    let mut patch = patch.clone();
    patch.params.extend_from_slice(params);
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patch.state(&registry))
        .expect("the patch loads");
    instance
        .activate(Settings::new(RATE, 128))
        .expect("activation");
    let frames = (seconds * RATE) as usize;
    instance
        .render(&Script::new(frames).note_on(0, key, 1.0))
        .output
}

/// The left channel from 0.1 s in, past the attack.
fn steady(output: &Output) -> Vec<f32> {
    output.channel(0)[4_800..].to_vec()
}

fn hz(key: f64) -> f64 {
    440.0 * 2f64.powf((key - 69.0) / 12.0)
}

// Spec: engine-core R1 (pitch is the note plus Octave, Semitone, Fine and
// Sweep), R137 (Main tuning).
#[test]
fn every_tuning_control_moves_the_pitch() {
    let a = |param: u16| id(OSC, 0, param);
    for (params, semitones) in [
        (vec![], 0.0),
        (vec![(a(osc::OCTAVE), -2.0)], -24.0),
        (vec![(a(osc::SEMITONE), 7.0)], 7.0),
        (vec![(a(osc::FINE), 50.0)], 0.5),
        (vec![(a(osc::SWEEP), -5.5)], -5.5),
        (vec![(global::TUNING, 3.0)], 3.0),
    ] {
        let note = steady(&hold(&patches::init(), &params, 60, 0.6));
        let measured = measure::pitch(&note, RATE).expect("a pitch");
        let expected = hz(60.0 + semitones);
        assert!(
            (measured / expected - 1.0).abs() < 2e-4,
            "{params:?}: {measured} Hz, not {expected} Hz"
        );
    }
}

// Spec: engine-core R7 (Level is gain n²), section 8 (Main level −9 dB by
// default, a gain of √2 × 0.5²).
#[test]
fn level_follows_its_squared_law_and_main_its_default() {
    let rms = |level: f64| {
        let note = steady(&hold(
            &patches::init(),
            &[(id(OSC, 0, channel::LEVEL), level)],
            57,
            0.6,
        ));
        measure::rms(&note)
    };
    // A saw's RMS is 1/√3; at 75 %, gain 0.5625, then Main's √2 × 0.25.
    let expected = 0.5625 * 2f64.sqrt() * 0.25 / 3f64.sqrt();
    let init = rms(75.0);
    assert!(
        (db(init / expected)).abs() < 0.1,
        "{} dB off",
        db(init / expected)
    );
    assert!((db(rms(25.0) / init) - 40.0 * (25f64 / 75.0).log10()).abs() < 0.05);
    assert!((db(rms(100.0) / init) - 40.0 * (100f64 / 75.0).log10()).abs() < 0.05);
}

// Spec: engine-core R50, R51, R57 (a low-pass on the voice path: the
// harmonics below its cutoff pass, those above fall at its slope).
#[test]
fn the_filter_shapes_the_oscillators_harmonics() {
    let f = |param: u16| id(FILTER, 0, param);
    let key = 45; // 110 Hz
    let fundamental = hz(45.0);
    let dry = steady(&hold(&patches::init(), &[], key, 1.1));
    let wet = steady(&hold(
        &patches::init(),
        &[
            (f(filter::ON), 1.0),
            // SVF LP 24, a Butterworth response at resonance 0.
            (f(filter::TYPE), 7.0),
            (f(filter::CUTOFF), 880.0),
            (f(filter::RESONANCE), 0.0),
        ],
        key,
        1.1,
    ));
    let response = |harmonic: f64| {
        let frequency = fundamental * harmonic;
        db(measure::tone(&wet, RATE, frequency).0 / measure::tone(&dry, RATE, frequency).0)
    };
    // Well below the cutoff, flat; at it, −3 dB; an octave above, −24 dB.
    assert!(response(1.0).abs() < 0.2, "{}", response(1.0));
    assert!((response(8.0) + 3.0).abs() < 0.5, "{}", response(8.0));
    assert!((response(16.0) + 24.0).abs() < 1.5, "{}", response(16.0));
    assert!(response(32.0) < -45.0, "{}", response(32.0));
}

// Spec: engine-core R9 (a filter that's off passes its input on).
#[test]
fn a_filter_that_is_off_passes_its_input() {
    let off = hold(&patches::init(), &[], 57, 0.3);
    let main = hold(
        &patches::init(),
        &[(id(OSC, 0, channel::OUTPUT), 1.0)],
        57,
        0.3,
    );
    assert!(
        off.same_bits(&main),
        "the Init patch through Filter 1 (off) is A alone"
    );
    let on = hold(
        &patches::init(),
        &[(id(FILTER, 0, filter::ON), 1.0)],
        57,
        0.3,
    );
    let centroid =
        |o: &Output| measure::centroid(&o.channel(0)[4_800..], RATE).expect("a centroid");
    assert!(
        centroid(&on) < centroid(&off) * 0.7,
        "on, Filter 1 darkens it"
    );
}

#[test]
fn idle_it_is_digitally_silent_and_sleeps() {
    let render = lspk_test::render::render::<Reagent>(&Script::new(4_800), Settings::new(RATE, 64))
        .expect("a render");
    assert_eq!(
        measure::noise_floor(render.output.channel(0)),
        f64::NEG_INFINITY
    );
    assert!(render.sleeps());
}

#[test]
fn every_patch_plays_at_a_sensible_level() {
    for patch in patches::all() {
        let note = hold(&patch, &[], 48, 1.0);
        let left = &note.channel(0)[24_000..];
        let peak = measure::peak(left);
        assert!(note.is_finite(), "{}", patch.name);
        assert!(
            (db_to_gain(-30.0)..1.0).contains(&peak),
            "{}: peak {:.1} dBFS",
            patch.name,
            db(peak)
        );
    }
}
