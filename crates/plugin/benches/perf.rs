//! Reagent's benchmark scenarios, at the rate and block size the plug-in
//! kit's reference patches use: 48 kHz in 128-frame blocks. The Init patch
//! and the three hand-built patches (`reagent::patches`) each hold 8 and
//! then 32 notes for a second and release them, for the CPU per voice at
//! the default quality (High, twice the host's rate).

use std::process::ExitCode;

use lspk_test::bench::{Scenario, Suite};
use lspk_test::{Script, Settings};
use reagent::{Reagent, patches};
use reagent_engine::Registry;

lspk_rt::install_trap!();

const RATE: u32 = 48_000;

/// `voices` notes, spread over three octaves, held for a second.
fn chord(voices: usize) -> Script {
    (0..voices as u32).fold(Script::new(RATE as usize * 6 / 5), |script, i| {
        script.note(i * 7, RATE, (40 + (i * 7) % 36) as u8, 0.8)
    })
}

fn main() -> ExitCode {
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
    suite.main()
}
