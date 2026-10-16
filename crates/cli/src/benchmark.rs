//! `reagent-cli benchmark`: Reagent's benchmark suite timed on this machine
//! and printed as the table `cargo bench` prints.
//!
//! The suite is the one `crates/plugin/benches/perf.rs` times and `cargo
//! xtask bench` runs: the Init patch and the three hand-built patches, each
//! holding 8 and then 32 notes, at 48 kHz in 128-frame blocks. Build the
//! binary in release for numbers worth keeping.

use std::process::ExitCode;

use clap::Parser;
use lspk_test::bench::{Budget, Scenario, Suite};
use lspk_test::{Script, Settings};
use reagent::{Reagent, patches};
use reagent_engine::Registry;

use crate::Outcome;

/// The sample rate the suite times at, as the reference patches use.
const RATE: u32 = 48_000;

/// What to time.
#[derive(Debug, Parser)]
pub struct BenchmarkArgs {
    /// A tenth of the time, to check the benchmarks run.
    #[arg(long)]
    pub quick: bool,
}

/// `voices` notes, spread over three octaves, held for a second.
fn chord(voices: usize) -> Script {
    (0..voices as u32).fold(Script::new(RATE as usize * 6 / 5), |script, i| {
        script.note(i * 7, RATE, (40 + (i * 7) % 36) as u8, 0.8)
    })
}

/// Reagent's benchmark suite, as `benches/perf.rs` builds it.
#[must_use]
pub fn suite() -> Suite {
    let settings = Settings::new(f64::from(RATE), 128);
    let registry = Registry::new();
    let mut suite = Suite::new("reagent");
    for patch in patches::all() {
        for voices in [8, 32] {
            let state = patch.clone().with_voices(voices).state(&registry);
            suite.scenario(
                Scenario::new::<Reagent>(
                    &format!("{}-{voices}", patch.name),
                    chord(voices),
                    settings,
                )
                .voices(voices)
                .state(state),
            );
        }
    }
    suite
}

/// Times the suite and prints its table.
pub fn run(args: &BenchmarkArgs) -> Outcome {
    let budget = if args.quick {
        Budget::QUICK
    } else {
        Budget::FULL
    };
    let measurements = suite().time(budget)?;
    print!("{}", lspk_test::bench::table(&measurements));
    Ok(ExitCode::SUCCESS)
}
