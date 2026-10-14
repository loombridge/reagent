//! Reagent measured through its whole voice path: pitch at every octave
//! and tuning control, the level laws of a channel and Main, the filter's
//! response on the oscillator's harmonics, digital silence when idle, and
//! every hand-built patch at a sensible level.
//!
//! Spec rules are cited as `engine-core Rn`.

use lspk_mod::{Polarity, Target};
use lspk_params::{ParamId, db_to_gain};
use lspk_test::measure::{self, db};
use lspk_test::render::{Instance, Output, Script, Settings};
use reagent::Reagent;
use reagent::params::{self, FILTER, LFO, OSC, channel, env, filter, global, id, lfo, osc};
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

// Spec: engine-core R136 (reference pitch sets A4 for every oscillator and
// key-tracked filter).
#[test]
fn reference_pitch_sets_a4_for_oscillators_and_key_tracked_filters() {
    let f = |param: u16| id(FILTER, 0, param);
    // 1. Oscillators: playing A4 (key 69) with different reference pitches.
    for ref_pitch in [430.0, 432.0, 440.0, 448.0, 450.0] {
        let note = steady(&hold(
            &patches::init(),
            &[(global::REFERENCE_PITCH, ref_pitch)],
            69,
            0.6,
        ));
        let measured = measure::pitch(&note, RATE).expect("a pitch");
        assert!(
            (measured / ref_pitch - 1.0).abs() < 2e-4,
            "ref_pitch {ref_pitch}: measured {measured} Hz"
        );
    }

    // 2. Filters: key tracking on vs off with reference pitch.
    let dry_440 = steady(&hold(
        &patches::init(),
        &[(global::REFERENCE_PITCH, 440.0)],
        69,
        0.6,
    ));
    let dry_430 = steady(&hold(
        &patches::init(),
        &[(global::REFERENCE_PITCH, 430.0)],
        69,
        0.6,
    ));

    // With key tracking on, the filter tracks the oscillator pitch:
    // the attenuation of harmonic 2 (an octave above fundamental) is identical
    // at 440 Hz and 430 Hz because both oscillator and filter scale together.
    let filter_kt_440 = steady(&hold(
        &patches::init(),
        &[
            (global::REFERENCE_PITCH, 440.0),
            (f(filter::ON), 1.0),
            (f(filter::TYPE), 7.0), // SVF LP 24
            (f(filter::CUTOFF), 440.0),
            (f(filter::KEY_TRACK), 1.0),
        ],
        69,
        0.6,
    ));
    let filter_kt_430 = steady(&hold(
        &patches::init(),
        &[
            (global::REFERENCE_PITCH, 430.0),
            (f(filter::ON), 1.0),
            (f(filter::TYPE), 7.0),
            (f(filter::CUTOFF), 440.0),
            (f(filter::KEY_TRACK), 1.0),
        ],
        69,
        0.6,
    ));
    let resp_kt_440 =
        db(measure::tone(&filter_kt_440, RATE, 880.0).0 / measure::tone(&dry_440, RATE, 880.0).0);
    let resp_kt_430 =
        db(measure::tone(&filter_kt_430, RATE, 860.0).0 / measure::tone(&dry_430, RATE, 860.0).0);
    assert!(
        (resp_kt_440 - resp_kt_430).abs() < 0.2,
        "with key tracking, attenuation relative to note harmonics is identical: 440={resp_kt_440}, 430={resp_kt_430}"
    );

    // With key tracking off, cutoff is fixed in absolute hertz (500 Hz),
    // so it does not scale with reference pitch.
    let filter_off_440 = steady(&hold(
        &patches::init(),
        &[
            (global::REFERENCE_PITCH, 440.0),
            (f(filter::ON), 1.0),
            (f(filter::TYPE), 7.0),
            (f(filter::CUTOFF), 500.0),
            (f(filter::KEY_TRACK), 0.0),
        ],
        69,
        0.6,
    ));
    let filter_off_430 = steady(&hold(
        &patches::init(),
        &[
            (global::REFERENCE_PITCH, 430.0),
            (f(filter::ON), 1.0),
            (f(filter::TYPE), 7.0),
            (f(filter::CUTOFF), 500.0),
            (f(filter::KEY_TRACK), 0.0),
        ],
        69,
        0.6,
    ));
    // At 440 Hz (above cutoff), 440 Hz fundamental is attenuated more than 430 Hz fundamental
    let resp_off_440 =
        db(measure::tone(&filter_off_440, RATE, 440.0).0 / measure::tone(&dry_440, RATE, 440.0).0);
    let resp_off_430 =
        db(measure::tone(&filter_off_430, RATE, 430.0).0 / measure::tone(&dry_430, RATE, 430.0).0);
    assert!(
        resp_off_440 < resp_off_430,
        "fixed cutoff attenuates higher frequency 440 Hz more than 430 Hz: 440={resp_off_440}, 430={resp_off_430}"
    );
}

// Spec: engine-core R80 (envelope scale multiplies every envelope's times).
#[test]
fn envelope_scale_multiplies_envelope_times() {
    let e = |param: u16| params::env_id(0, param);
    let base_params = [
        (e(env::ATTACK), 100.0),
        (e(env::HOLD), 0.0),
        (e(env::DECAY), 100.0),
        (e(env::SUSTAIN), 0.0),
    ];
    let render_scale = |scale: f64| {
        let mut p = base_params.to_vec();
        p.push((global::ENV_SCALE, scale));
        let registry = Registry::new();
        let mut patch = patches::init();
        patch.params.extend(p);
        let mut instance = Instance::<Reagent>::new().expect("instance");
        instance.load_state(&patch.state(&registry)).unwrap();
        instance.activate(Settings::new(RATE, 128)).unwrap();
        instance
            .render(&Script::new((0.4 * RATE) as usize).note_on(0, 60, 1.0))
            .output
    };
    let out_100 = render_scale(100.0);
    let out_200 = render_scale(200.0);
    let out_50 = render_scale(50.0);

    let peak_sample = |out: &Output| {
        let ch = out.channel(0);
        ch.iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))
            .map(|(i, _)| i)
            .unwrap()
    };
    let peak_100 = peak_sample(&out_100);
    let peak_200 = peak_sample(&out_200);
    let peak_50 = peak_sample(&out_50);

    // 100 ms is 4800 samples at 48 kHz.
    assert!(
        (peak_100 as f64 - 4800.0).abs() < 256.0,
        "peak_100: {peak_100}"
    );
    assert!(
        (peak_200 as f64 - 9600.0).abs() < 256.0,
        "peak_200: {peak_200}"
    );
    assert!(
        (peak_50 as f64 - 2400.0).abs() < 256.0,
        "peak_50: {peak_50}"
    );
}

// Spec: engine-core R97 (LFO scale scales every LFO).
#[test]
fn lfo_scale_scales_lfo_period() {
    let l = |param: u16| params::id(LFO, 0, param);
    let registry = Registry::new();
    let mut patch = patches::init();
    patch
        .params
        .extend([(l(lfo::SYNC), 0.0), (l(lfo::RATE), 10.0)]);
    patch.routes.push((
        "lfo1",
        Target::Param(global::TUNING),
        1.0,
        Polarity::Bipolar,
    ));

    let render_lfo_scale = |scale: f64| {
        let mut p = patch.clone();
        p.params.push((global::LFO_SCALE, scale));
        let mut instance = Instance::<Reagent>::new().expect("instance");
        instance.load_state(&p.state(&registry)).unwrap();
        instance.activate(Settings::new(RATE, 128)).unwrap();
        instance
            .render(&Script::new((0.5 * RATE) as usize).note_on(0, 69, 1.0))
            .output
    };

    let out_100 = render_lfo_scale(100.0);
    let out_200 = render_lfo_scale(200.0);
    assert!(!out_100.same_bits(&out_200));
}

// Spec: engine-core R132 (Quality oversampling: Good 1×, High 2×, Ultra 4×).
#[test]
fn quality_sets_oversampling_factor_and_latency() {
    assert_eq!(reagent_engine::engine::quality_factor(0.0), 1);
    assert_eq!(reagent_engine::engine::quality_factor(1.0), 2);
    assert_eq!(reagent_engine::engine::quality_factor(2.0), 4);

    let lat_good = reagent_engine::engine::latency(0.0);
    let lat_high = reagent_engine::engine::latency(1.0);
    let lat_ultra = reagent_engine::engine::latency(2.0);

    assert_eq!(lat_good, 0);
    assert!(lat_high > 0);
    assert!(lat_ultra > lat_high);
}

// Spec: engine-core R52 (with key tracking on, cutoff moves one octave per
// octave, following the pitch of the first of A, B and C with key tracking on,
// glide included; if none tracks, it follows the played note).
#[test]
fn filter_key_tracking_follows_first_key_tracked_oscillator() {
    let a = |param: u16| id(OSC, 0, param);
    let b = |param: u16| id(OSC, 1, param);
    let c = |param: u16| id(OSC, 2, param);
    let f = |param: u16| id(FILTER, 0, param);

    // 1. With A tracking and at +12 semitones, the cutoff is an octave above the
    // played-note case.
    let dry_a12 = steady(&hold(&patches::init(), &[(a(osc::OCTAVE), 1.0)], 60, 0.6));
    let wet_a_tracking = steady(&hold(
        &patches::init(),
        &[
            (a(osc::OCTAVE), 1.0),
            (f(filter::ON), 1.0),
            (f(filter::TYPE), 7.0),
            (f(filter::CUTOFF), 440.0),
            (f(filter::KEY_TRACK), 1.0),
        ],
        60,
        0.6,
    ));
    let wet_none_tracking = steady(&hold(
        &patches::init(),
        &[
            (a(osc::OCTAVE), 1.0),
            (a(osc::KEY_TRACK), 0.0),
            (b(osc::KEY_TRACK), 0.0),
            (c(osc::KEY_TRACK), 0.0),
            (f(filter::ON), 1.0),
            (f(filter::TYPE), 7.0),
            (f(filter::CUTOFF), 440.0),
            (f(filter::KEY_TRACK), 1.0),
        ],
        60,
        0.6,
    ));
    let h2_freq = hz(72.0) * 2.0;
    let dry_h2 = measure::tone(&dry_a12, RATE, h2_freq).0;
    let resp_a_tracking = db(measure::tone(&wet_a_tracking, RATE, h2_freq).0 / dry_h2);
    let resp_none_tracking = db(measure::tone(&wet_none_tracking, RATE, h2_freq).0 / dry_h2);

    assert!(
        (resp_a_tracking - resp_none_tracking) > 18.0,
        "A tracking at +12 st moves cutoff 1 octave higher: tracking={resp_a_tracking} dB, none={resp_none_tracking} dB"
    );

    // 2. With A not tracking and B tracking, B's pitch counts.
    let dry_b12 = steady(&hold(
        &patches::init(),
        &[
            (a(channel::ON), 0.0),
            (a(osc::KEY_TRACK), 0.0),
            (b(channel::ON), 1.0),
            (b(channel::OUTPUT), 0.0),
            (b(osc::KEY_TRACK), 1.0),
            (b(osc::OCTAVE), 1.0),
        ],
        60,
        0.6,
    ));
    let wet_b_tracking_12 = steady(&hold(
        &patches::init(),
        &[
            (a(channel::ON), 0.0),
            (a(osc::KEY_TRACK), 0.0),
            (b(channel::ON), 1.0),
            (b(channel::OUTPUT), 0.0),
            (b(osc::KEY_TRACK), 1.0),
            (b(osc::OCTAVE), 1.0),
            (f(filter::ON), 1.0),
            (f(filter::TYPE), 7.0),
            (f(filter::CUTOFF), 440.0),
            (f(filter::KEY_TRACK), 1.0),
        ],
        60,
        0.6,
    ));
    let dry_b_h2 = measure::tone(&dry_b12, RATE, h2_freq).0;
    let resp_b12 = db(measure::tone(&wet_b_tracking_12, RATE, h2_freq).0 / dry_b_h2);
    assert!(
        resp_b12 > -10.0,
        "with B tracking at +12 st, B's pitch counts (cutoff 880 Hz): resp={resp_b12} dB"
    );

    // 3. With none tracking, the played note counts.
    // When none of A, B and C track, A plays 8.18 Hz regardless of key (R5),
    // but the filter cutoff follows the played note (220 Hz at 48, 440 Hz at 60,
    // 880 Hz at 72).
    let none_params = [
        (a(osc::KEY_TRACK), 0.0),
        (b(osc::KEY_TRACK), 0.0),
        (c(osc::KEY_TRACK), 0.0),
        (f(filter::ON), 1.0),
        (f(filter::TYPE), 7.0),
        (f(filter::CUTOFF), 440.0),
        (f(filter::KEY_TRACK), 1.0),
    ];
    let wet_none_48 = steady(&hold(&patches::init(), &none_params, 48, 0.6));
    let wet_none_60 = steady(&hold(&patches::init(), &none_params, 60, 0.6));
    let wet_none_72 = steady(&hold(&patches::init(), &none_params, 72, 0.6));
    let c48 = measure::centroid(&wet_none_48, RATE).unwrap();
    let c60 = measure::centroid(&wet_none_60, RATE).unwrap();
    let c72 = measure::centroid(&wet_none_72, RATE).unwrap();
    assert!(
        c48 < c60 && c60 < c72,
        "cutoff follows played note when none tracks: c48={c48}, c60={c60}, c72={c72}"
    );

    // When filter key tracking is off, cutoff stays fixed and centroid does not change with key:
    let mut none_kt_off = none_params.to_vec();
    none_kt_off[6] = (f(filter::KEY_TRACK), 0.0);
    let off_48 = steady(&hold(&patches::init(), &none_kt_off, 48, 0.6));
    let off_72 = steady(&hold(&patches::init(), &none_kt_off, 72, 0.6));
    let c_off_48 = measure::centroid(&off_48, RATE).unwrap();
    let c_off_72 = measure::centroid(&off_72, RATE).unwrap();
    assert!(
        (c_off_48 - c_off_72).abs() < 1e-2,
        "with filter key tracking off, cutoff is fixed: c48={c_off_48}, c72={c_off_72}"
    );

    // 4. Glide moves the cutoff with the pitch.
    // In mono legato with glide, note 48 glides to note 60 over 400 ms.
    // With A's key tracking off (fixed 8.18 Hz saw input), the rise in the
    // 440 Hz harmonic is driven entirely by the filter cutoff gliding from
    // 220 Hz to 440 Hz.
    let registry = Registry::new();
    let mut patch = patches::init();
    patch.params.extend([
        (a(osc::KEY_TRACK), 0.0),
        (f(filter::ON), 1.0),
        (f(filter::TYPE), 7.0),
        (f(filter::CUTOFF), 440.0),
        (f(filter::KEY_TRACK), 1.0),
        (global::MONO, 1.0),
        (global::LEGATO, 1.0),
        (global::GLIDE_TIME, 400.0),
    ]);
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance.load_state(&patch.state(&registry)).unwrap();
    instance.activate(Settings::new(RATE, 128)).unwrap();
    let render = instance
        .render(
            &Script::new((0.8 * RATE) as usize)
                .note_on(0, 48, 1.0)
                .note_on((0.1 * RATE) as u32, 60, 1.0),
        )
        .output;
    let ch = render.channel(0);
    let early = &ch[(0.05 * RATE) as usize..(0.09 * RATE) as usize];
    let mid = &ch[(0.25 * RATE) as usize..(0.35 * RATE) as usize];
    let late = &ch[(0.65 * RATE) as usize..(0.75 * RATE) as usize];
    let tone_early = measure::tone(early, RATE, 440.0).0;
    let tone_mid = measure::tone(mid, RATE, 440.0).0;
    let tone_late = measure::tone(late, RATE, 440.0).0;
    assert!(
        tone_early < tone_mid && tone_mid < tone_late,
        "glide moves cutoff smoothly: early={tone_early}, mid={tone_mid}, late={tone_late}"
    );
}
