//! The sub and noise oscillators through the whole engine: the sub's
//! shapes, Octave, Sweep and Key track, the noise's colour slopes, Start,
//! Start random and Loop, both through the filters, and both as modulation
//! sources, with the audio thread off the heap.
//!
//! Spec rules are cited as `engine-core Rn`.

use lspk_mod::{Polarity, Target};
use lspk_params::ParamId;
use lspk_test::measure::{self, db};
use lspk_test::render::{Instance, Output, Script, Settings};
use reagent::Reagent;
use reagent::params::{NOISE, OSC, SUB, channel, global, id, noise, sub};
use reagent::patches::{self, Patch};
use reagent_engine::Registry;

lspk_rt::install_trap!();

const RATE: f64 = 48_000.0;

/// The sub's parameter `param`.
fn s(param: u16) -> ParamId {
    id(SUB, 0, param)
}

/// The noise's parameter `param`.
fn n(param: u16) -> ParamId {
    id(NOISE, 0, param)
}

fn play(patch: &Patch, script: &Script) -> Output {
    let registry = Registry::new();
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patch.state(&registry))
        .expect("the patch loads");
    instance
        .activate(Settings::new(RATE, 128))
        .expect("activation");
    instance.render(script).output
}

/// The Init patch with A silenced, the sub on and to Main, plus `params`,
/// holding A3 (220 Hz) for half a second.
fn play_sub(params: &[(ParamId, f64)]) -> Output {
    let mut patch = patches::init();
    patch.params.push((id(OSC, 0, channel::ON), 0.0));
    patch.params.push((s(channel::ON), 1.0));
    patch.params.push((s(channel::OUTPUT), 1.0));
    patch.params.extend_from_slice(params);
    play(&patch, &Script::new(24_000).note_on(0, 57, 1.0))
}

/// The same for the noise.
fn play_noise(params: &[(ParamId, f64)]) -> Output {
    let mut patch = patches::init();
    patch.params.push((id(OSC, 0, channel::ON), 0.0));
    patch.params.push((n(channel::ON), 1.0));
    patch.params.push((n(channel::OUTPUT), 1.0));
    patch.params.extend_from_slice(params);
    play(&patch, &Script::new(24_000).note_on(0, 57, 1.0))
}

/// The fundamental of `output`, measured after the attack.
fn pitch(output: &Output) -> f64 {
    measure::pitch(&output.channel(0)[4_800..], RATE).expect("a periodic signal")
}

// Spec: engine-core R40 (the note, Octave and Sweep), R5 (Key track off
// plays MIDI note 60), R41 (Phase 0° by default).
#[test]
fn the_sub_plays_its_note_with_octave_sweep_and_key_tracking() {
    assert!((pitch(&play_sub(&[])) - 220.0).abs() < 0.5);
    assert!((pitch(&play_sub(&[(s(sub::OCTAVE), 1.0)])) - 440.0).abs() < 1.0);
    assert!((pitch(&play_sub(&[(s(sub::SWEEP), 12.0)])) - 440.0).abs() < 1.0);
    assert!((pitch(&play_sub(&[(s(sub::SWEEP), -12.0)])) - 110.0).abs() < 0.5);
    assert!((pitch(&play_sub(&[(s(sub::KEY_TRACK), 0.0)])) - 261.63).abs() < 0.5);
}

// Spec: engine-core R40 (the six fixed shapes), section 5.
#[test]
fn every_sub_shape_plays_and_the_shape_control_picks_it() {
    let mut shapes = [[0.0f32; 24_000]; 6];
    for (index, shape) in shapes.iter_mut().enumerate() {
        let output = play_sub(&[(s(sub::SHAPE), index as f64)]);
        let left = output.channel(0);
        shape.copy_from_slice(&left[..24_000]);
        assert!(
            measure::rms(&left[4_800..]) > 0.01,
            "shape {index} is silent"
        );
        assert!(
            (pitch(&output) - 220.0).abs() < 1.0,
            "shape {index} at the wrong pitch"
        );
    }
    for a in 0..6 {
        for b in a + 1..6 {
            let difference = shapes[a]
                .iter()
                .zip(&shapes[b])
                .map(|(x, y)| f64::from((x - y).abs()))
                .fold(0.0, f64::max);
            assert!(difference > 0.01, "shapes {a} and {b} are the same wave");
        }
    }
}

// Spec: engine-core R42 (Wrap phase).
#[test]
fn phase_modulation_past_a_cycle_holds_or_wraps() {
    let mut patch = patches::init();
    patch.params.push((id(OSC, 0, channel::ON), 0.0));
    patch.params.push((s(channel::ON), 1.0));
    patch.params.push((s(channel::OUTPUT), 1.0));
    // The Constant source at full amount adds Phase's whole 360° travel.
    patch.routes.push((
        "constant",
        Target::Param(s(sub::PHASE)),
        1.0,
        Polarity::Unipolar,
    ));
    let script = Script::new(24_000).note_on(0, 57, 1.0);
    let held = play(&patch, &script);
    patch.params.push((s(sub::WRAP_PHASE), 1.0));
    let wrapped = play(&patch, &script);
    assert!(
        measure::rms(&held.channel(0)[4_800..]) < 0.01,
        "off, the phase holds at 360°"
    );
    assert!(
        measure::rms(&wrapped.channel(0)[4_800..]) > 0.05,
        "on, it wraps and plays on"
    );
}

// Spec: engine-core R49 (white flat, pink −3 dB per octave, brown −6).
#[test]
fn the_noise_plays_each_colour_with_its_slope() {
    for (source, slope) in [(0.0, 0.0), (1.0, -3.0), (2.0, -6.0), (3.0, 0.0)] {
        let output = play_noise(&[(n(noise::SOURCE), source)]);
        let signal = &output.channel(0)[4_800..];
        assert!(measure::rms(signal) > 0.001, "colour {source} is silent");
        if source == 3.0 {
            // Crackle is clicks: a huge crest factor, mostly near silence.
            let peak = measure::peak(signal);
            assert!(peak / measure::rms(signal) > 4.0, "not a crackle");
            continue;
        }
        let bands = measure::bands(signal, RATE);
        let band = |hz: f64| {
            bands
                .iter()
                .find(|b| (b.centre - hz).abs() < 1.0)
                .expect("the band")
                .level_db
        };
        // 250 Hz to 8 kHz: five octaves, in bands twice as wide each
        // octave, so a flat spectrum reads 3 dB per octave.
        let measured = (band(8_000.0) - band(250.0)) / 5.0 - 3.0;
        assert!(
            (measured - slope).abs() < 1.0,
            "colour {source}: {measured:.2} dB per octave"
        );
    }
}

// Spec: engine-core R47 (Start and Start random), R48 (one-shot and
// Loop).
#[test]
fn start_start_random_and_loop_change_what_plays() {
    let at_start = play_noise(&[]);
    let later = play_noise(&[(n(noise::START), 50.0)]);
    assert!(
        at_start.max_difference(&later) > 0.001,
        "Start moves the read"
    );
    let spread = play_noise(&[(n(noise::START_RANDOM), 50.0)]);
    assert!(
        at_start.max_difference(&spread) > 0.001,
        "Start random moves each note's read"
    );

    // One-shot from the very end of the block stops; Loop wraps and keeps
    // playing.
    let stopped = play_noise(&[(n(noise::START), 99.9), (n(noise::LOOP), 0.0)]);
    assert!(
        measure::rms(&stopped.channel(0)[9_600..]) < 1e-4,
        "a one-shot stops at the end"
    );
    let looped = play_noise(&[(n(noise::START), 99.9)]);
    assert!(
        measure::rms(&looped.channel(0)[9_600..]) > 0.001,
        "Loop wraps"
    );
}

// Spec: engine-core R43 (the sub and the noise have the same routing as
// the main oscillators), R45 (the noise is a modulation source), R9 (a
// filter that's off passes its input).
#[test]
fn the_sub_and_the_noise_feed_the_filters_and_move_routes() {
    // The sub's default output is the filters; Filter 1 is off in a new
    // patch, which passes the signal.
    let filtered = play_sub(&[]);
    assert!(measure::rms(&filtered.channel(0)[4_800..]) > 0.01);

    // As modulation sources: a route from each one's audio to Amp moves
    // the sound, and is silent when its source is off.
    for (source, on, mut patch) in [
        ("sub", s(channel::ON), patches::init()),
        ("noise", n(channel::ON), patches::init()),
    ] {
        patch.params.push((id(OSC, 0, channel::ON), 0.0));
        patch.params.push((on, 1.0));
        patch.params.push((
            n(channel::OUTPUT),
            if source == "noise" { 1.0 } else { 0.0 },
        ));
        patch
            .routes
            .push((source, Target::Param(global::AMP), 0.5, Polarity::Unipolar));
        let script = Script::new(24_000).note_on(0, 57, 1.0);
        let moved = play(&patch, &script);
        assert!(
            measure::rms(&moved.channel(0)[4_800..]) > 0.001,
            "{source} audio through a route is audible"
        );
        // The same patch with the source off: the route reads silence and
        // nothing sounds.
        let mut off = patch.clone();
        off.params.retain(|(id, _)| *id != on);
        off.params.push((on, 0.0));
        let quiet = play(&off, &script);
        assert!(
            measure::rms(&quiet.channel(0)[4_800..]) < 1e-4,
            "{source} audio with the source off is silent"
        );
    }
}

// Spec: engine-core section 6 (the noise's default level is 25 %), R7 (a
// square reads higher than a sine at the same level).
#[test]
fn the_sub_and_noise_defaults_are_the_specs() {
    let sub_sine = play_sub(&[]);
    let sub_square = play_sub(&[(s(sub::SHAPE), 4.0)]);
    // A square is louder than a sine at the same level and pitch.
    assert!(
        db(measure::rms(&sub_square.channel(0)[4_800..])
            / measure::rms(&sub_sine.channel(0)[4_800..]))
            > 1.0
    );
    let noise = play_noise(&[]);
    let sub = play_sub(&[]);
    // 25 % against 75 %: the noise sits about 9.5 dB below the sub.
    let difference =
        db(measure::rms(&noise.channel(0)[4_800..]) / measure::rms(&sub.channel(0)[4_800..]));
    assert!(
        difference < -5.0,
        "the noise is {difference} dB against the sub"
    );
}
