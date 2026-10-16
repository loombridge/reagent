//! The reference patches' budgets (engine-core R1.17): every reference
//! patch ends its voice below the noise floor and sleeps, so a host does no
//! work at all while it is idle.
//!
//! The plan's other budgets — the per-voice cost and the worst-case block
//! over a ten-minute render at 32-frame blocks — take ten minutes of audio
//! to measure and run in release, in `benches/soak.rs`, not in the debug
//! test profile: `cargo bench -p reagent --bench soak`.

use lspk_plugin::Event;
use lspk_test::measure;
use lspk_test::render::{Instance, Script, Settings};
use reagent::Reagent;
use reagent::patches::{self, Patch};
use reagent_engine::Registry;
use reagent_engine::params::global;

lspk_rt::install_trap!();

const RATE: f64 = 48_000.0;
/// The rate and block size the per-voice budgets are measured at.
const BLOCK: usize = 128;
/// The plan's loudest voice counts; one voice per note is enough to show a
/// patch sleeps, so the suite's patches run at their default voice count.
const VOICES: usize = 8;

/// `patch` active at `BLOCK`-frame blocks, at `VOICES` voices.
fn instance(patch: &Patch) -> Instance<Reagent> {
    let registry = Registry::new();
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patch.clone().with_voices(VOICES).state(&registry))
        .expect("the patch loads");
    instance
        .activate(Settings::new(RATE, BLOCK))
        .expect("activation");
    instance
}

// Spec: engine-core R1.17 (a voice ends when its output falls below the
// noise floor, so the instrument sleeps and a host does no work at all).
#[test]
fn every_reference_patch_sleeps_and_costs_nothing_when_idle() {
    for patch in patches::all() {
        let mut instance = instance(&patch);
        // A note, released after a quarter second, then two and three
        // quarter seconds of silence: every release and every tail — the
        // pad's longest, 1.5 s — ends well inside it.
        let script = Script::new((3.0 * RATE) as usize).note(0, (RATE / 4.0) as u32, 60, 1.0);
        let render = instance.render(&script);
        assert!(render.sleeps(), "{} never slept", patch.name);
        assert!(
            render.output.channel(0)[(2.5 * RATE) as usize..]
                .iter()
                .all(|s| *s == 0.0),
            "{} still made sound while idle",
            patch.name
        );
    }
}

// Spec: engine-core R1.17 (a run of parameter changes is rebuilt once, so a
// preset a host sends in one block costs one rebuild, not one per value —
// and the render is the same however the host spread the changes).
#[test]
fn a_run_of_parameter_changes_renders_as_the_patch_it_sets() {
    let patch = patches::supersaw();
    let events: Vec<Event> = patch
        .params
        .iter()
        .map(|(id, value)| Event::Param {
            time: 0,
            id: *id,
            value: *value,
        })
        .collect();
    // All the values in one block, then as many silent blocks as the other
    // instance takes, so both have advanced the same frames.
    let mut at_once = instance(&patches::init());
    at_once.process(BLOCK, &[], &events);
    for _ in 1..events.len() {
        at_once.process(BLOCK, &[], &[]);
    }
    // One value a block.
    let mut spread = instance(&patches::init());
    for event in &events {
        spread.process(BLOCK, &[], std::slice::from_ref(event));
    }

    let script = Script::new((0.5 * RATE) as usize).note_on(0, 57, 1.0);
    let at_once = at_once.render(&script).output;
    let spread = spread.render(&script).output;
    assert!(
        at_once.same_bits(&spread),
        "the run differs by up to {:.1} dBFS",
        measure::db(at_once.max_difference(&spread))
    );
}

// Spec: engine-core R1.17 (a voicing setting the voice manager reads before
// a note — mono, the voice limit, stealing — lands ahead of a note at the
// same time as its change; only the modules wait for the next render).
#[test]
fn a_voicing_change_and_a_note_at_the_same_time_render_as_the_change_set_first() {
    let frames = (0.5 * RATE) as usize;
    // The change set before the instance plays anything.
    let mut mono = patches::init();
    mono.params.push((global::MONO, 1.0));
    let before = instance(&mono)
        .render(&Script::new(frames).note_on(0, 48, 1.0).note_on(0, 60, 1.0))
        .output;
    // The change arriving as an event at the same frame as the notes.
    let with_notes = instance(&patches::init())
        .render(
            &Script::new(frames)
                .param(0, global::MONO, 1.0)
                .note_on(0, 48, 1.0)
                .note_on(0, 60, 1.0),
        )
        .output;
    assert!(
        before.same_bits(&with_notes),
        "the change beside the notes differs from it set first by up to {:.1} dBFS",
        measure::db(before.max_difference(&with_notes))
    );
}
