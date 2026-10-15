//! The patch compiler through the whole instrument: the program holds only
//! the modules in use, and a recompile between two blocks keeps every voice,
//! phase and tail, so it is inaudible.
//!
//! Spec rules are cited as `engine-core Rn`.

use lspk_plugin::{Event, Note, ProcessStatus};
use lspk_test::render::{Instance, Output, Script, Settings};
use reagent::Reagent;
use reagent::params::{OSC, channel, id, osc};
use reagent::patches::{self, Patch};
use reagent_engine::Registry;
use reagent_engine::patch::Patch as Program;

lspk_rt::install_trap!();

const RATE: f64 = 48_000.0;
const BLOCK: usize = 128;

/// The Init patch with B and C configured but switched off: three
/// oscillators in the patch, one in use.
fn three_oscillators() -> Patch {
    let b = |param| id(OSC, 1, param);
    let c = |param| id(OSC, 2, param);
    let mut patch = patches::init();
    patch.name = "three";
    patch.params.extend([
        (b(channel::ON), 0.0),
        (b(osc::UNISON), 7.0),
        (b(osc::DETUNE), 0.4),
        (b(osc::OCTAVE), -1.0),
        (b(channel::OUTPUT), 0.0),
        (b(channel::LEVEL), 80.0),
        (b(osc::FRAME), 50.0),
        (c(channel::ON), 0.0),
        (c(osc::UNISON), 3.0),
        (c(osc::SEMITONE), 7.0),
        (c(channel::OUTPUT), 2.0),
        (c(channel::LEVEL), 60.0),
    ]);
    patch
}

/// `patch` activated, holding one note for `seconds`, and the program it
/// compiles to.
fn play(patch: &Patch, seconds: f64) -> (Output, Program) {
    let registry = Registry::new();
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patch.state(&registry))
        .expect("the patch loads");
    let program = instance.plugin().compile_patch();
    instance
        .activate(Settings::new(RATE, BLOCK))
        .expect("activation");
    let output = instance
        .render(&Script::new((seconds * RATE) as usize).note_on(0, 57, 0.9))
        .output;
    (output, program)
}

// Spec: engine-core R8, R59 (a switched-off module and a filter no source
// reaches cost nothing to run: the compiled program leaves them out, so the
// two patches play the same samples).
#[test]
fn unused_oscillators_cost_nothing_and_change_nothing() {
    let (one, program) = play(&patches::init(), 0.3);
    let (three, three_program) = play(&three_oscillators(), 0.3);
    assert_eq!(program.channels(), [0], "only A is on");
    assert_eq!(
        program.filters(),
        [0],
        "A routes to the filters at balance −100, so Filter 1 is in use"
    );
    assert_eq!(
        three_program, program,
        "B and C are off, so they never join the program"
    );
    assert!(
        one.same_bits(&three),
        "a patch with three oscillators but one in use renders as one"
    );
}

/// The Pad patch rendered for two halves of `half` frames each, with a
/// recompile forced between them if `recompile`; the halves joined.
fn render_pad(recompile: bool) -> Output {
    let registry = Registry::new();
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patches::pad().state(&registry))
        .expect("the patch loads");
    instance
        .activate(Settings::new(RATE, BLOCK))
        .expect("activation");
    let half = 4_096;
    let first = instance
        .render(&Script::new(half).note_on(0, 57, 0.9))
        .output;
    if recompile {
        assert!(instance.plugin_mut().recompile(), "the processor is active");
    }
    let second = instance.render(&Script::new(half)).output;
    let mut channels = first.channels;
    for (channel, tail) in channels.iter_mut().zip(&second.channels) {
        channel.extend_from_slice(tail);
    }
    Output::new(RATE, channels)
}

// Spec: engine-core R1.17 (recompiles swap at a block boundary; the voices,
// the oscillators' phases, the filters' tails and the LFOs move across
// untouched, so a render split by a recompile is identical to one that
// never recompiled).
#[test]
fn a_recompile_between_blocks_is_inaudible() {
    let continuous = render_pad(false);
    let recompiled = render_pad(true);
    assert!(continuous.is_finite() && !continuous.is_silent());
    assert!(
        continuous.same_bits(&recompiled),
        "the swap kept every voice, phase and tail"
    );
}

// Spec: engine-core R1.17 (the swap allocates nothing and takes no lock on
// the audio thread). The render harness runs every block of `process` as a
// real-time section when the trap is installed, so the block that takes and
// installs the freshly compiled program is one.
#[test]
fn the_swap_runs_under_the_allocation_trap() {
    let registry = Registry::new();
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patches::init().state(&registry))
        .expect("the patch loads");
    instance
        .activate(Settings::new(RATE, BLOCK))
        .expect("activation");
    instance.plugin_mut().recompile();
    let violations = lspk_rt::trap::thread_violations();
    let block = instance.process(
        BLOCK,
        &[],
        &[Event::NoteOn {
            time: 0,
            note: Note::key(0, 60),
            velocity: 0.8,
        }],
    );
    assert_eq!(
        lspk_rt::trap::thread_violations(),
        violations,
        "taking the patch and swapping it touched no heap"
    );
    assert_eq!(block.status, ProcessStatus::Continue);
    assert!(
        block.outputs[0].iter().any(|s| *s != 0.0),
        "the patch plays after the swap"
    );
}
