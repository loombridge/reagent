//! Reagent's golden renders: the Init patch and the three hand-built patches
//! (`reagent::patches`) each playing a phrase, null-tested against the
//! references in `tests/golden/`: bit-exact on the platform that made them,
//! below −96 dBFS elsewhere. Remake them with `cargo xtask golden --update`,
//! never from here, and say why in the pull request.
//!
//! The same script renders to the same bits at every block size, and from
//! the built `.clap` through the CLAP test host.

use lspk_test::golden;
use lspk_test::render::{BLOCK_SIZES, Instance, Output, Script, Settings, block_invariance};
use reagent::patches::{self, Patch};
use reagent::{ID, NAME, Reagent};
use reagent_engine::Registry;

lspk_rt::install_trap!();

const RATE: f64 = 48_000.0;

/// A second and a half: a chord built note by note, a held high note, then
/// a quick run of nine notes, more than the eight voices, so the oldest give
/// way, and every note released before the end.
fn phrase() -> Script {
    let mut script = Script::new(72_000)
        .note(0, 24_000, 48, 1.0)
        .note(3_000, 21_000, 55, 0.8)
        .note(6_000, 18_000, 60, 0.6)
        .note(9_000, 15_000, 64, 0.7)
        .note(26_000, 12_000, 72, 0.9);
    for i in 0..9u32 {
        script = script.note(40_000 + i * 2_000, 6_000, 57 + (i * 5 % 12) as u8, 0.75);
    }
    script
}

/// `patch` playing the phrase at `block` frames a block.
fn render(patch: &Patch, block: usize) -> Output {
    let registry = Registry::new();
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .load_state(&patch.state(&registry))
        .expect("the patch loads");
    instance
        .activate(Settings::new(RATE, block))
        .expect("activation");
    instance.render(&phrase()).output
}

#[test]
fn golden_renders_null_against_their_references() {
    let mut suite = golden::Suite::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden"));
    for patch in patches::all() {
        let output = render(&patch, 512);
        assert!(output.is_finite());
        suite.add(patch.name, output);
    }
    suite.assert();
}

#[test]
fn the_golden_script_renders_the_same_at_every_block_size() {
    // The patches with unison and with modulation, which have the most
    // that could depend on how a block splits.
    for patch in [patches::supersaw(), patches::unison16(), patches::pad()] {
        let invariance = block_invariance(&BLOCK_SIZES, |block| render(&patch, block));
        assert!(invariance.is_exact(), "{}: {invariance:?}", patch.name);
    }
}

#[test]
fn the_built_clap_renders_the_golden_script_to_the_same_bits() {
    let clap = lspk_test::clap::bundle_built_library("reagent", NAME, ID);
    let mut host = lspk_test::ClapTestHost::load(&clap, ID).expect("the built plug-in");
    host.activate(RATE, 512).expect("activation");
    let built = host.render(&phrase(), 512, &reagent::params());
    let in_process = render(&patches::init(), 512);
    assert!(
        built.output.same_bits(&in_process),
        "the .clap differs by up to {} dBFS",
        lspk_test::measure::db(built.output.max_difference(&in_process))
    );
}
