//! The interface's live values (R2.3): while a note plays and an editor
//! watches, the audio thread publishes a modulated value, the voice's phases
//! and envelope stage, the levels, a scope and a spectrum, all inside the
//! allocation trap; with no editor open it writes nothing, and the
//! instrument still sleeps when silent.

use lspk_mod::telemetry::Frame;
use lspk_mod::{Polarity, Target};
use lspk_plugin::Event;
use lspk_test::render::{Instance, Script, Settings};
use reagent::Reagent;
use reagent::params::{LFO, id, lfo};
use reagent::patches::Patch;
use reagent_engine::Registry;
use reagent_engine::params::global;

lspk_rt::install_trap!();

const RATE: f64 = 48_000.0;
const BLOCK: usize = 128;

/// The Init patch with Amp riding LFO 1, so Amp's modulated value is the
/// LFO's.
fn instance() -> Instance<Reagent> {
    let registry = Registry::new();
    let patch = Patch {
        name: "telemetry",
        params: vec![
            (global::AMP, 0.0),
            // Free-running at 2 Hz: its phase is the clock's, whatever the note.
            (id(LFO, 0, lfo::SONG_LOCK), 0.0),
        ],
        routes: vec![("lfo1", Target::Param(global::AMP), 1.0, Polarity::Unipolar)],
    };
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patch.state(&registry))
        .expect("the patch loads");
    instance
        .activate(Settings::new(RATE, BLOCK))
        .expect("activation");
    instance
}

/// The latest frame, with an editor's handle.
fn latest(instance: &Instance<Reagent>) -> Frame {
    instance
        .plugin()
        .telemetry()
        .lock()
        .unwrap()
        .latest()
        .clone()
}

fn watch(instance: &Instance<Reagent>, on: bool) {
    instance.plugin().telemetry().lock().unwrap().watch(on);
}

#[test]
fn an_open_editor_reads_a_modulated_value_and_a_scope_while_a_note_plays() {
    let mut instance = instance();
    watch(&instance, true);
    // Half a second, so the LFO has moved and the oscillator has run.
    let render = instance.render(&Script::new(24_000).note_on(0, 69, 1.0));
    assert!(!render.output.is_silent());
    let frame = latest(&instance);

    let amp = Registry::new()
        .dest(Target::Param(global::AMP))
        .expect("Amp is a destination");
    let value = frame.modulated_at(amp, 0).expect("the route reaches Amp");
    assert!(
        value > 0.0 && value <= 1.0,
        "Amp's modulated value, {value}"
    );
    assert!(
        frame.modulated_at(amp, 1).is_none(),
        "voice 1 isn't playing"
    );

    assert!(frame.active[0] && !frame.active[1]);
    assert!(frame.clock > 0);
    assert_ne!(frame.env_stage[0], lspk_dsp::envelope::Stage::Idle);
    assert!((0.0..1.0).contains(&frame.voice_phase[0]));
    assert!(frame.lfo_phase[0] > 0.0, "LFO 1 has run");
    assert!(frame.levels[0] > 0.0, "the left channel's peak");
    assert!(
        frame.scope.iter().any(|s| s.abs() > 0.0),
        "the scope holds the note"
    );
    let loudest = frame.spectrum.iter().copied().fold(0.0, f32::max);
    assert!(loudest > 0.0, "the spectrum holds the note");
}

#[test]
fn a_closed_editor_gets_nothing_written() {
    let mut instance = instance();
    let before = latest(&instance).clock;
    instance.render(&Script::new(4_800).note_on(0, 69, 1.0));
    assert_eq!(latest(&instance).clock, before, "nothing was published");
    assert_eq!(before, 0);
}

#[test]
fn the_instrument_still_sleeps_when_silent_with_an_editor_open() {
    let mut instance = instance();
    watch(&instance, true);
    let render =
        instance.render(&Script::new((0.5 * RATE) as usize).note(0, (0.2 * RATE) as u32, 60, 1.0));
    assert!(render.sleeps());
    let frame = latest(&instance);
    assert!(
        frame.active.iter().all(|a| !a),
        "the last frame shows the silence it fell into"
    );
    // A note wakes it, and the editor sees it.
    let block = instance.process(
        BLOCK,
        &[],
        &[Event::NoteOn {
            time: 0,
            note: lspk_plugin::Note::key(0, 60),
            velocity: 1.0,
        }],
    );
    assert_ne!(block.status, lspk_plugin::ProcessStatus::Sleep);
}
