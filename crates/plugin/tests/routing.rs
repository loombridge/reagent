//! The mixer's routing, through the whole engine: series and parallel
//! filters, Main against Dry out, None, the bus sends after Level, buses
//! into each other, and routes that would loop reset to Main and reported
//! to the host as a person's change.
//!
//! Spec rules are cited as `engine-core Rn`.

use lspk_mod::{Polarity, Target};
use lspk_params::ParamId;
use lspk_plugin::{Event, OutputEvent};
use lspk_test::measure::{self, db};
use lspk_test::render::{Instance, Output, Script, Settings};
use reagent::Reagent;
use reagent::params::{BUS, FILTER, OSC, bus, channel, env, env_id, filter, global, id};
use reagent::patches;
use reagent_engine::Registry;

lspk_rt::install_trap!();

const RATE: f64 = 48_000.0;

fn a(param: u16) -> ParamId {
    id(OSC, 0, param)
}

fn f(n: u8, param: u16) -> ParamId {
    id(FILTER, n, param)
}

/// The Init patch with `params`, holding A3 for half a second.
fn play(params: &[(ParamId, f64)]) -> Output {
    let registry = Registry::new();
    let mut patch = patches::init();
    patch.params.extend_from_slice(params);
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patch.state(&registry))
        .expect("the patch loads");
    instance
        .activate(Settings::new(RATE, 128))
        .expect("activation");
    instance
        .render(&Script::new(24_000).note_on(0, 57, 1.0))
        .output
}

fn rms(output: &Output) -> f64 {
    measure::rms(&output.channel(0)[4_800..])
}

// Spec: engine-core R8 (Main, Dry out and None), section 8 (Main −9 dB and
// Dry out 0 dB by default).
#[test]
fn main_dry_out_and_none_go_where_they_say() {
    let main = rms(&play(&[(a(channel::OUTPUT), 1.0)]));
    let dry = rms(&play(&[(a(channel::OUTPUT), 2.0)]));
    assert!((db(dry / main) - 9.0).abs() < 0.05, "{} dB", db(dry / main));
    let none = play(&[(a(channel::OUTPUT), 3.0)]);
    assert!(none.is_silent(), "None reaches no output");
    let dry_off = play(&[(a(channel::OUTPUT), 2.0), (global::DRY_LEVEL, 0.0)]);
    assert!(dry_off.is_silent());
}

// Spec: engine-core R11 (sends after Level, unity at 100 %), R66 (buses go
// to Main, Dry out or the other bus).
#[test]
fn sends_follow_the_level_and_buses_route_on() {
    let main = rms(&play(&[(a(channel::OUTPUT), 1.0)]));
    let sent = rms(&play(&[
        (a(channel::OUTPUT), 1.0),
        (a(channel::SEND_1), 100.0),
    ]));
    assert!(
        (db(sent / main) - 6.02).abs() < 0.05,
        "+{} dB",
        db(sent / main)
    );
    let silent = play(&[
        (a(channel::OUTPUT), 1.0),
        (a(channel::SEND_1), 100.0),
        (a(channel::LEVEL), 0.0),
    ]);
    assert!(silent.is_silent(), "a send is taken after Level");
    // Bus 1 into Bus 2, and Bus 2 into the dry output (+9 dB over Main).
    let only_send = rms(&play(&[
        (a(channel::OUTPUT), 3.0),
        (a(channel::SEND_1), 100.0),
        (id(BUS, 0, bus::OUTPUT), 2.0),
        (id(BUS, 1, bus::OUTPUT), 1.0),
    ]));
    assert!(
        (db(only_send / main) - 9.0).abs() < 0.05,
        "{} dB",
        db(only_send / main)
    );
}

// Spec: engine-core R65 (series and parallel filters), R9.
#[test]
fn filters_run_in_series_or_in_parallel() {
    let low = |n: u8| {
        vec![
            (f(n, filter::ON), 1.0),
            (f(n, filter::TYPE), 7.0),
            (f(n, filter::CUTOFF), 1000.0),
            (f(n, filter::RESONANCE), 0.0),
        ]
    };
    let one: Vec<_> = low(0);
    let mut series = low(0);
    series.extend(low(1));
    series.push((f(0, filter::OUTPUT), 0.0));
    let mut parallel = low(0);
    parallel.extend(low(1));
    // Half of A to each filter, both to Main.
    parallel.push((a(channel::BALANCE), 0.0));
    let tone =
        |o: &Output, harmonic: f64| measure::tone(&o.channel(0)[4_800..], RATE, 220.0 * harmonic).0;
    let (one, series, parallel) = (play(&one), play(&series), play(&parallel));
    // Two octaves above the cutoff, one 24 dB low-pass is −48 dB; two in
    // series, −96 dB.
    let drop = |o: &Output| db(tone(o, 18.0) / tone(o, 1.0));
    assert!(
        drop(&series) < drop(&one) - 30.0,
        "{} {}",
        drop(&series),
        drop(&one)
    );
    // In parallel, the same filter twice at half each is one filter.
    assert!((db(tone(&parallel, 1.0) / tone(&one, 1.0))).abs() < 0.05);
    assert!((db(tone(&parallel, 9.0) / tone(&one, 9.0))).abs() < 0.05);
}

/// The output-parameter changes the plug-in reported to the host in
/// `events`.
fn reports(events: &[OutputEvent], ids: &[ParamId]) -> Vec<(ParamId, f64)> {
    events
        .iter()
        .filter_map(|e| match *e {
            OutputEvent::Param { id, value, .. } if ids.contains(&id) => Some((id, value)),
            _ => None,
        })
        .collect()
}

// Spec: engine-core R65, R68 (observed: routing one into the other while
// the other routes back resets the earlier to Main).
#[test]
fn a_route_back_resets_the_earlier_route_to_main_and_tells_the_host() {
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .activate(Settings::new(RATE, 128))
        .expect("activation");
    let set = |id: ParamId, value: f64| Event::Param { time: 0, id, value };
    let outputs = [f(0, filter::OUTPUT), f(1, filter::OUTPUT)];
    instance.process(128, &[], &[set(outputs[0], 0.0)]);
    let block = instance.process(128, &[], &[set(outputs[1], 0.0)]);
    assert_eq!(reports(&block.events, &outputs), [(outputs[0], 1.0)]);
    assert!(
        matches!(block.events.first(), Some(OutputEvent::GestureBegin { id, .. }) if *id == outputs[0]),
        "{:?}",
        block.events
    );
    assert_eq!(
        instance.values().get(outputs[0]),
        Some(1.0),
        "Filter 1 to Main"
    );
    assert_eq!(
        instance.values().get(outputs[1]),
        Some(0.0),
        "Filter 2 into Filter 1"
    );

    let buses = [id(BUS, 0, bus::OUTPUT), id(BUS, 1, bus::OUTPUT)];
    instance.process(128, &[], &[set(buses[1], 2.0)]);
    let block = instance.process(128, &[], &[set(buses[0], 2.0)]);
    assert_eq!(reports(&block.events, &buses), [(buses[1], 0.0)]);
    assert_eq!(instance.values().get(buses[1]), Some(0.0), "Bus 2 to Main");
}

// Spec: engine-core R10 (with amp envelope off, Env 1 does not scale the
// voice and the voice lasts until the longest release of any envelope in use).
#[test]
fn amp_envelope_switch_off_leaves_voice_unshaped_until_longest_release() {
    let registry = Registry::new();
    let cutoff = Target::Param(f(0, filter::CUTOFF));
    let mut patch = patches::init();
    patch.params.extend_from_slice(&[
        (a(channel::OUTPUT), 1.0),
        (a(channel::AMP_ENV), 0.0),
        (env_id(0, env::ATTACK), 1.0),
        (env_id(0, env::DECAY), 40.0),
        (env_id(0, env::SUSTAIN), 0.0),
        (env_id(0, env::RELEASE), 20.0),
        (env_id(1, env::RELEASE), 250.0),
    ]);
    patch.routes.push(("env2", cutoff, 0.5, Polarity::Unipolar));

    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patch.state(&registry))
        .expect("the patch loads");
    instance
        .activate(Settings::new(RATE, 128))
        .expect("activation");

    // Note held for 0.2 s (9_600 frames), total 0.6 s (28_800 frames).
    let render = instance.render(&Script::new(28_800).note(0, 9_600, 57, 1.0));
    let left = render.output.channel(0);

    // During the note (0.1 s in), Env 1 has decayed to silence; with the switch
    // off, the voice plays at full level unshaped by Env 1.
    let steady = measure::rms(&left[4_800..7_200]);
    assert!(steady > 0.05, "level is unshaped by Env 1: {steady}");

    // After note off at 0.2 s (9_600 frames), Env 1's 20 ms release ends at
    // 0.22 s (10_560 frames). Env 2's 250 ms release lasts until 0.45 s (21_600 frames).
    // At 0.3 s (14_400 frames), the voice is still sounding.
    let releasing = measure::rms(&left[14_400..16_800]);
    assert!(
        releasing > 0.05,
        "voice continues sounding during Env 2's release: {releasing}"
    );

    // After Env 2's release finishes, the voice has ended and gone silent.
    assert!(
        left[24_000..].iter().all(|&s| s == 0.0),
        "silent after Env 2 release"
    );
    assert!(
        render.events.iter().any(
            |e| matches!(e, OutputEvent::NoteEnd { time, .. } if (21_000..=22_500).contains(time))
        ),
        "note ended after Env 2 release: {:?}",
        render.events
    );
    assert!(render.sleeps(), "instance sleeps after voice ends");
}
