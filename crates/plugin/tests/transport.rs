//! The host's transport through the engine: a synced LFO follows its tempo
//! (R85), a Song-locked LFO its song position (R84), and without a
//! transport the engine keeps its own clock.
//!
//! Spec rules are cited as `engine-core Rn`.

use lspk_mod::{Polarity, Target};
use lspk_plugin::Transport;
use lspk_test::measure;
use lspk_test::render::{Instance, Output, Script, Settings};
use reagent::Reagent;
use reagent::params::{LFO, id, lfo};
use reagent::patches::Patch;
use reagent_engine::Registry;
use reagent_engine::params::global;

lspk_rt::install_trap!();

const RATE: f64 = 48_000.0;

/// The Init patch with Amp riding LFO 1, a quarter note from the start, so
/// the audio's level reads the LFO's phase. `song_lock` is the switch under
/// test.
fn lfo_patch(song_lock: bool) -> Patch {
    Patch {
        name: "lfo-transport",
        params: vec![
            // Amp at silence: the route's one-way travel is the whole gain.
            (global::AMP, 0.0),
            (id(LFO, 0, lfo::SONG_LOCK), f64::from(song_lock)),
        ],
        routes: vec![("lfo1", Target::Param(global::AMP), 1.0, Polarity::Unipolar)],
    }
}

/// An active instance playing `patch`, offered `transport` by the fake host.
fn instance(patch: &Patch, transport: Option<Transport>) -> Instance<Reagent> {
    let registry = Registry::new();
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patch.state(&registry))
        .expect("the patch loads");
    instance
        .activate(Settings::new(RATE, 128))
        .expect("activation");
    instance.set_transport(transport);
    instance
}

/// Plays a held note from the start of the render.
fn play(patch: &Patch, transport: Option<Transport>, frames: usize) -> Output {
    instance(patch, transport)
        .render(&Script::new(frames).note_on(0, 60, 1.0))
        .output
}

/// The transport of a playing host at `tempo`, `beats` in, in 4/4.
fn playing(tempo: f64, beats: f64) -> Transport {
    Transport {
        tempo,
        playing: true,
        beats,
        seconds: beats * 60.0 / tempo,
        ..Transport::default()
    }
}

/// A window of the left channel's level.
fn level(output: &Output, from: usize, to: usize) -> f64 {
    measure::rms(&output.channel(0)[from..to])
}

// Spec: engine-core R85 (a synced rate's cycle is a division of the host's
// tempo). A quarter note at 120 beats per minute lasts half a second, at
// 240 a quarter of one.
#[test]
fn a_synced_lfo_follows_the_hosts_tempo() {
    let patch = lfo_patch(true);
    // With Song lock on, the phase is the song position, so the LFO's cycle
    // ends where the level returns to silence; find that window.
    let period = |tempo: f64| {
        let output = play(&patch, Some(playing(tempo, 0.0)), 26_000);
        let left = output.channel(0);
        let envelope: Vec<f64> = left.chunks(100).map(measure::rms).collect();
        let (at, _) = envelope[10..]
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.total_cmp(b.1))
            .expect("a window");
        (at + 10) as f64 * 100.0
    };
    let fast = period(240.0);
    let slow = period(120.0);
    assert!(
        (slow / fast - 2.0).abs() < 0.05,
        "periods {slow} and {fast} frames"
    );
}

// Spec: engine-core R84 (with Song lock on, the phase is fixed to the
// host's song position, so two runs at the same position align and another
// position plays another phase).
#[test]
fn song_lock_fixes_the_phase_to_the_song_position() {
    let patch = lfo_patch(true);
    let at = |beats: f64| play(&patch, Some(playing(120.0, beats)), 14_400);
    let (rise, top) = (at(0.0), at(0.5));
    assert!(rise.same_bits(&at(0.0)), "the same position repeats");
    // From the top of the cycle the sound starts loudest and falls to
    // silence by the cycle's end; from its foot, the other way round.
    let early = |output| level(output, 480, 2_400);
    let late = |output| level(output, 11_000, 13_000);
    assert!(
        early(&top) > early(&rise) * 2.0,
        "the top starts loud: {} against {}",
        early(&top),
        early(&rise)
    );
    assert!(
        late(&rise) > late(&top) * 4.0,
        "half a cycle on, the two have changed places: {} against {}",
        late(&rise),
        late(&top)
    );
}

// Spec: engine-core R84 (without Song lock a restarted LFO runs from its
// own note), R83.
#[test]
fn without_song_lock_the_song_position_does_not_move_the_phase() {
    let patch = lfo_patch(false);
    let from_the_start = play(&patch, Some(playing(120.0, 0.0)), 9_600);
    let from_elsewhere = play(&patch, Some(playing(120.0, 0.5)), 9_600);
    assert!(
        from_the_start.same_bits(&from_elsewhere),
        "the note starts it, wherever the song stands"
    );
    assert!(
        level(&from_the_start, 2_400, 4_800) > 0.0,
        "the LFO plays, from its own note"
    );
}

// Spec: engine-core R83, R84 (with no host transport a plug-in follows a
// clock of its own, and a render repeats).
#[test]
fn without_a_host_transport_the_engines_clock_serves_and_a_render_repeats() {
    let patch = lfo_patch(true);
    let render = play(&patch, None, 24_000);
    assert!(render.same_bits(&play(&patch, None, 24_000)), "it repeats");
    // The engine's own clock runs: half a cycle on, the triangle is up at
    // the top where it started near zero.
    let near = level(&render, 480, 2_400);
    let later = level(&render, 6_480, 8_400);
    assert!(
        near < later * 0.5,
        "the phase moves with the engine's own clock: {near} then {later}"
    );
}
