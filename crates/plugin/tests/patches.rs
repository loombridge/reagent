//! The hand-built patches and the modulation matrix through the engine:
//! LFOs, envelopes, macros and velocity moving what they're routed to,
//! unison, every quality, the remote-controls pages, and the audio thread
//! off the heap with routes, unison and parameter changes in play.
//!
//! Spec rules are cited as `engine-core Rn`.

use lspk_mod::{Polarity, Target};
use lspk_params::ParamId;
use lspk_plugin::{Event, Plugin};
use lspk_test::measure::{self, db};
use lspk_test::render::{Instance, Output, Script, Settings};
use reagent::params::{FILTER, OSC, channel, filter, global, id, osc};
use reagent::patches::{self, Patch};
use reagent::{APPLY_MACROS, Reagent, SET_NOTE_CURVE, SWAP_MACROS, space};
use reagent_engine::Registry;
use serde_json::json;

lspk_rt::install_trap!();

const RATE: f64 = 48_000.0;

fn instance(patch: &Patch, block: usize) -> Instance<Reagent> {
    let registry = Registry::new();
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patch.state(&registry))
        .expect("the patch loads");
    instance
        .activate(Settings::new(RATE, block))
        .expect("activation");
    instance
}

fn play(patch: &Patch, script: &Script) -> Output {
    instance(patch, 128).render(script).output
}

// Spec: engine-core R81 to R86 (an LFO per voice, restarting at each note,
// here at 0.6 Hz), R103 (a route swings its destination).
#[test]
fn the_pads_lfo_sweeps_its_cutoff() {
    let output = play(&patches::pad(), &Script::new(96_000).note_on(0, 60, 1.0));
    let left = output.channel(0);
    let centroids: Vec<f64> = (4..20)
        .map(|w| measure::centroid(&left[w * 4_800..(w + 1) * 4_800], RATE).expect("a centroid"))
        .collect();
    let (low, high) = centroids
        .iter()
        .fold((f64::MAX, 0.0f64), |(l, h), c| (l.min(*c), h.max(*c)));
    assert!(high > low * 1.3, "the brightness moves: {centroids:?}");
}

// Spec: engine-core R114 (a macro is a source), R112 (velocity to Amp).
#[test]
fn macros_and_velocity_move_what_they_drive() {
    let level = Target::Param(id(OSC, 0, channel::LEVEL));
    let patch = Patch {
        name: "macro",
        params: vec![(id(OSC, 0, channel::LEVEL), 0.0)],
        routes: vec![("macro1", level, 1.0, Polarity::Unipolar)],
    };
    let rms = |macro_value: f64| {
        let script = Script::new(14_400)
            .param(0, global::macro_id(0), macro_value)
            .note_on(0, 57, 1.0);
        measure::rms(&play(&patch, &script).channel(0)[4_800..])
    };
    assert_eq!(rms(0.0), 0.0, "Macro 1 at 0 leaves the level at 0");
    // Macro 1 at 50 % moves the level half its travel: 50 %, gain 0.25.
    assert!((db(rms(100.0) / rms(50.0)) - 12.04).abs() < 0.1);

    let amp = Target::Param(global::AMP);
    let velocity = Patch {
        name: "velocity",
        params: vec![(global::AMP, 0.0)],
        routes: vec![("velocity", amp, 1.0, Polarity::Unipolar)],
    };
    let loud = play(&velocity, &Script::new(14_400).note_on(0, 57, 1.0));
    let soft = play(&velocity, &Script::new(14_400).note_on(0, 57, 0.25));
    let ratio =
        db(measure::rms(&loud.channel(0)[4_800..]) / measure::rms(&soft.channel(0)[4_800..]));
    assert!(ratio > 6.0, "a softer note is quieter by {ratio} dB");
}

// Spec: engine-core R32, R33 (unison copies, level compensated), R39 (they
// don't count against Voices).
#[test]
fn unison_thickens_without_getting_louder_or_taking_voices() {
    let one = play(&patches::init(), &Script::new(48_000).note_on(0, 57, 1.0));
    let mut seven = patches::init();
    seven.params.extend([
        (id(OSC, 0, osc::UNISON), 7.0),
        (id(OSC, 0, osc::DETUNE), 0.4),
    ]);
    let wide = play(&seven, &Script::new(48_000).note_on(0, 57, 1.0));
    let rms = |o: &Output| measure::rms(&o.channel(0)[9_600..]);
    assert!(
        (db(rms(&wide) / rms(&one))).abs() < 3.0,
        "{} dB",
        db(rms(&wide) / rms(&one))
    );
    assert!(
        !wide
            .channel(0)
            .iter()
            .zip(wide.channel(1))
            .all(|(l, r)| l == r),
        "the copies spread across the stereo field"
    );
    // Eight notes of seven copies each all sound.
    let chord = (0..8u8).fold(Script::new(9_600), |s, i| s.note_on(0, 48 + 3 * i, 0.8));
    let mut instance = instance(&seven, 128);
    let render = instance.render(&chord);
    assert!(
        render.events.is_empty(),
        "no voice was stolen: {:?}",
        render.events
    );
}

// Spec: engine-core R132 (Good, High and Ultra run the voices at 1, 2 and 4
// times the rate): the same note at each.
#[test]
fn every_quality_plays_the_same_note() {
    let mut reference = None;
    for quality in [0.0, 1.0, 2.0] {
        let mut patch = patches::init();
        patch.params.push((global::QUALITY, quality));
        let instance = instance(&patch, 128);
        let latency = reagent_engine::engine::latency(quality);
        assert_eq!(lspk_plugin::Plugin::latency(instance.plugin()), latency);
        let mut instance = instance;
        let output = instance
            .render(&Script::new(24_000).note_on(0, 57, 1.0))
            .output;
        let note = &output.channel(0)[4_800..];
        let pitch = measure::pitch(note, RATE).expect("a pitch");
        assert!(
            (pitch - 220.0).abs() < 0.05,
            "quality {quality}: {pitch} Hz"
        );
        let rms = measure::rms(note);
        let reference = *reference.get_or_insert(rms);
        assert!((db(rms / reference)).abs() < 0.2, "quality {quality}");
    }
    assert_eq!(reagent_engine::engine::latency(0.0), 0);
    assert_eq!(reagent_engine::engine::latency(1.0), 16);
    assert_eq!(reagent_engine::engine::latency(2.0), 24);
}

#[test]
fn every_module_has_its_remote_controls_pages() {
    let space = space();
    let defaults: Vec<(ParamId, f64)> = space
        .descriptors()
        .iter()
        .map(|d| (d.id, d.default))
        .collect();
    let pages = space.pages(&|id| defaults.iter().find(|(i, _)| *i == id).map(|(_, v)| *v));
    let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
    for name in [
        "Osc A",
        "Osc A 2",
        "Osc C",
        "Osc Sub",
        "Osc Noise",
        "Filter 1",
        "Filter 2",
        "Bus 1",
        "Env 1",
        "Env 4",
        "LFO 1",
        "LFO 10",
        "Global",
    ] {
        assert!(names.contains(&name), "{name} in {names:?}");
    }
    // The first global page holds the eight macros.
    let global = pages
        .iter()
        .find(|p| p.name == "Global")
        .expect("a global page");
    assert_eq!(global.params[0], Some(global::macro_id(0)));
    assert_eq!(global.params[7], Some(global::macro_id(7)));
}

#[test]
fn the_audio_thread_stays_off_the_heap_with_routes_and_unison() {
    assert!(lspk_rt::trap::is_installed());
    for patch in [patches::supersaw(), patches::bass(), patches::pad()] {
        let mut instance = instance(&patch.with_voices(32), 256);
        let set = |id: ParamId, value: f64| Event::Param {
            time: 64,
            id,
            value,
        };
        // Each `process` call runs inside the allocation trap's real-time
        // section, which fails the test on any heap operation.
        {
            for i in 0..48u8 {
                let events = [
                    Event::NoteOn {
                        time: u32::from(i),
                        note: lspk_plugin::Note::key(0, 36 + i % 40),
                        velocity: 0.8,
                    },
                    set(id(OSC, 0, osc::UNISON), f64::from(1 + i % 16)),
                    set(id(OSC, 0, osc::DETUNE), f64::from(i % 10) / 10.0),
                    set(global::QUALITY, f64::from(i % 3)),
                    Event::NoteOff {
                        time: 200,
                        note: lspk_plugin::Note::key(0, 36 + (i + 20) % 40),
                        velocity: 0.5,
                    },
                ];
                instance.process(256, &[], &events);
            }
        }
    }
}

/// The velocity curve's drawn graph: full velocity to half the travel.
fn half_velocity_curve() -> serde_json::Value {
    json!([{ "x": 0.0, "y": 0.0 }, { "x": 1.0, "y": 0.5 }])
}

// Spec: engine-core R103 to R105 (a route moves its destination; the mod
// wheel is a source of section 12).
#[test]
fn the_mod_wheel_sweeps_a_cutoff() {
    let cutoff = Target::Param(id(FILTER, 0, filter::CUTOFF));
    let patch = Patch {
        name: "mod-wheel",
        params: vec![
            (id(FILTER, 0, filter::ON), 1.0),
            (id(FILTER, 0, filter::CUTOFF), 200.0),
        ],
        routes: vec![("mod_wheel", cutoff, 0.8, Polarity::Unipolar)],
    };
    let brightness = |wheel: u8| {
        let script = Script::new(24_000)
            .event(Event::Midi {
                time: 0,
                port: 0,
                data: [0xB0, 1, wheel],
            })
            .note_on(0, 57, 1.0);
        let output = play(&patch, &script);
        measure::centroid(&output.channel(0)[4_800..14_400], RATE).expect("a centroid")
    };
    let (low, high) = (brightness(0), brightness(127));
    assert!(
        high > low * 2.0,
        "the wheel opens the filter: {low} Hz at 0, {high} Hz at 127"
    );
}

// Spec: engine-core R99 (the velocity curve maps note-on velocity through
// its drawn graph onto the velocity source).
#[test]
fn a_drawn_velocity_curve_maps_velocity_onto_its_travel() {
    let level = Target::Param(id(OSC, 0, channel::LEVEL));
    let patch = Patch {
        name: "velocity-curve",
        params: vec![(id(OSC, 0, channel::LEVEL), 0.0)],
        routes: vec![("velocity", level, 1.0, Polarity::Unipolar)],
    };
    let loud = play(&patch, &Script::new(14_400).note_on(0, 57, 1.0));
    let mut curved = instance(&patch, 128);
    curved
        .plugin_mut()
        .run_command(
            SET_NOTE_CURVE,
            &json!({ "which": "velocity", "points": half_velocity_curve() }),
        )
        .expect("the curve draws");
    let quiet = curved
        .render(&Script::new(14_400).note_on(0, 57, 1.0))
        .output;
    let ratio =
        db(measure::rms(&loud.channel(0)[4_800..]) / measure::rms(&quiet.channel(0)[4_800..]));
    assert!((ratio - 12.04).abs() < 0.2, "half the travel is {ratio} dB");
}

// Spec: engine-core R100 (with Glide curve on, a note played while another
// is held moves the curve's output to the new value over the glide time).
#[test]
fn the_key_curve_glides_a_legato_note() {
    let level = Target::Param(id(OSC, 0, channel::LEVEL));
    let patch = |glide: f64| Patch {
        name: "key-glide",
        params: vec![
            (global::MONO, 1.0),
            (global::LEGATO, 1.0),
            (global::GLIDE_TIME, 500.0),
            (global::CURVE_GLIDE, glide),
            (id(OSC, 0, channel::LEVEL), 0.0),
        ],
        routes: vec![("key", level, 1.0, Polarity::Unipolar)],
    };
    let script = || {
        Script::new(38_400)
            .note_on(0, 48, 1.0)
            .note_on(4_800, 72, 1.0)
    };
    let rise = |glide: f64| {
        let output = play(&patch(glide), &script());
        let left = output.channel(0);
        db(measure::rms(&left[28_900..33_700]) / measure::rms(&left[4_900..9_700]))
    };
    let gliding = rise(1.0);
    assert!(gliding > 4.0, "the key curve glides up by {gliding} dB");
    let stepped = rise(0.0);
    assert!(
        stepped.abs() < 1.0,
        "without the glide the curve steps by {stepped} dB"
    );
}

// Spec: engine-core R101 (Velocity legato decides whether a legato note
// updates the velocity sources).
#[test]
fn velocity_legato_updates_the_velocity_source() {
    let level = Target::Param(id(OSC, 0, channel::LEVEL));
    let patch = |legato: f64| Patch {
        name: "velocity-legato",
        params: vec![
            (global::MONO, 1.0),
            (global::LEGATO, 1.0),
            (global::VELOCITY_LEGATO, legato),
            (id(OSC, 0, channel::LEVEL), 0.0),
        ],
        routes: vec![("velocity", level, 1.0, Polarity::Unipolar)],
    };
    let script = || {
        Script::new(19_200)
            .note_on(0, 48, 1.0)
            .note_on(9_600, 60, 0.25)
    };
    let change = |legato: f64| {
        let output = play(&patch(legato), &script());
        let left = output.channel(0);
        db(measure::rms(&left[10_000..14_800]) / measure::rms(&left[4_800..9_600]))
    };
    let kept = change(0.0);
    assert!(kept.abs() < 1.0, "off, the velocity stays: {kept} dB");
    let moved = change(1.0);
    assert!(
        (moved + 24.08).abs() < 0.5,
        "on, a quarter velocity leaves a quarter of the level: {moved} dB"
    );
}

// Spec: engine-core R103 (a route with an aux is scaled by it), R114 (a
// macro is an aux source).
#[test]
fn a_macro_scales_a_route() {
    let level = Target::Param(id(OSC, 0, channel::LEVEL));
    let patch = Patch {
        name: "macro-aux",
        params: vec![(id(OSC, 0, channel::LEVEL), 0.0)],
        routes: vec![("constant", level, 1.0, Polarity::Unipolar)],
    };
    let rms = |macro_value: f64| {
        let mut instance = instance(&patch, 128);
        instance
            .plugin_mut()
            .run_command(
                "mod.set_aux",
                &json!({ "slot": 0, "source": "macro1", "invert": false }),
            )
            .expect("the aux sets");
        let script = Script::new(14_400)
            .param(0, global::macro_id(0), macro_value)
            .note_on(0, 57, 1.0);
        measure::rms(&instance.render(&script).output.channel(0)[4_800..])
    };
    assert_eq!(rms(0.0), 0.0, "a macro at 0 leaves the route at 0");
    let ratio = db(rms(100.0) / rms(50.0));
    assert!((ratio - 12.04).abs() < 0.2, "half the macro: {ratio} dB");
}

// Spec: engine-core R115 (Apply and Delete adds each macro's current offset
// into the parameters it drives, then removes every macro route).
#[test]
fn the_apply_macros_command_bakes_the_offsets() {
    let registry = Registry::new();
    let level = id(OSC, 0, channel::LEVEL);
    let cutoff = Target::Param(id(FILTER, 0, filter::CUTOFF));
    let patch = Patch {
        name: "apply",
        params: vec![(level, 0.0), (global::macro_id(0), 50.0)],
        routes: vec![
            ("macro1", Target::Param(level), 0.5, Polarity::Unipolar),
            ("lfo1", cutoff, 0.5, Polarity::Unipolar),
        ],
    };
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patch.state(&registry))
        .expect("the patch loads");
    let applied = instance
        .plugin_mut()
        .run_command(APPLY_MACROS, &serde_json::Value::Null)
        .expect("the macros apply");
    assert_eq!(applied["removed"], 1);
    assert_eq!(applied["parameters"], 1);
    assert_eq!(
        instance.host().param(level),
        Some(25.0),
        "half of a half-amount route moves the level a quarter of the way"
    );
    let routes = instance
        .plugin_mut()
        .run_command("mod.list", &serde_json::Value::Null)
        .expect("the list");
    assert_eq!(
        routes[0]["source"], "lfo1",
        "the LFO's route stays: {routes}"
    );
    assert_eq!(routes.as_array().expect("a list").len(), 1);
}

// Spec: engine-core R116 (swapping two macros swaps their routes).
#[test]
fn the_swap_macros_command_moves_their_routes() {
    let registry = Registry::new();
    let patch = Patch {
        name: "swap",
        params: Vec::new(),
        routes: vec![
            (
                "macro1",
                Target::Param(id(OSC, 0, channel::LEVEL)),
                0.5,
                Polarity::Unipolar,
            ),
            (
                "macro2",
                Target::Param(id(FILTER, 0, filter::CUTOFF)),
                0.5,
                Polarity::Unipolar,
            ),
        ],
    };
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patch.state(&registry))
        .expect("the patch loads");
    let swapped = instance
        .plugin_mut()
        .run_command(SWAP_MACROS, &json!({ "first": 1, "second": 2 }))
        .expect("the macros swap");
    assert_eq!(swapped["swapped"], 2);
    let routes = instance
        .plugin_mut()
        .run_command("mod.list", &serde_json::Value::Null)
        .expect("the list");
    let sources: Vec<&str> = routes
        .as_array()
        .expect("a list")
        .iter()
        .map(|r| r["source"].as_str().expect("a source"))
        .collect();
    assert_eq!(sources, ["macro2", "macro1"]);
    // A macro that isn't there is refused.
    assert!(
        instance
            .plugin_mut()
            .run_command(SWAP_MACROS, &json!({ "first": 1, "second": 9 }))
            .is_err()
    );
}

// Spec: engine-core R99 (the curves), R114 (a macro route): both stay
// through the patch state.
#[test]
fn a_patch_state_round_trips_the_macros_and_the_curves() {
    let registry = Registry::new();
    let patch = Patch {
        name: "state",
        params: Vec::new(),
        routes: vec![(
            "macro3",
            Target::Param(id(OSC, 0, channel::LEVEL)),
            0.5,
            Polarity::Unipolar,
        )],
    };
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patch.state(&registry))
        .expect("the patch loads");
    instance
        .plugin_mut()
        .run_command(
            SET_NOTE_CURVE,
            &json!({ "which": "velocity", "points": half_velocity_curve() }),
        )
        .expect("the curve draws");
    let state = instance.save_state();
    let before = instance.plugin().matrix().clone();
    // Something else, then the state back.
    instance
        .plugin_mut()
        .run_command(SWAP_MACROS, &json!({ "first": 3, "second": 8 }))
        .expect("the macros swap");
    instance.load_state(&state).expect("the state loads");
    assert_eq!(instance.plugin().matrix(), &before);
    assert_eq!(
        instance.plugin().matrix().note_curves.velocity.value(1.0),
        0.5
    );
}
