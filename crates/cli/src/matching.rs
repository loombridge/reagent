//! `reagent-cli match`: a recording searched for a patch that sounds like
//! it, and the patch written as a preset.
//!
//! The search is `reagent_core::matching`'s, run to its budget on every
//! core; the report is printed as JSON, with the time it took.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use clap::Parser;
use lspk_plugin::Plugin;
use lspk_test::render::Instance;
use reagent_core::matching::{self, Args};
use reagent_core::{Reagent, preset};
use reagent_engine::Registry;
use serde_json::json;

use crate::Outcome;

/// How to match.
#[derive(Debug, Parser)]
pub struct MatchArgs {
    /// The recording to match, a WAV file of one held note.
    #[arg(long)]
    pub reference: PathBuf,
    /// Where to write the patch it finds, as a preset file.
    #[arg(long)]
    pub out: PathBuf,
    /// How many candidate patches to render.
    #[arg(long, default_value_t = matching::DEFAULT_EVALUATIONS)]
    pub evaluations: usize,
    /// The seed the search draws from.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
    /// The MIDI key the note is played on; default the key nearest the
    /// reference's pitch.
    #[arg(long)]
    pub key: Option<u8>,
    /// How long the key is held, in seconds; default two thirds of the
    /// reference's length.
    #[arg(long)]
    pub hold: Option<f64>,
    /// How many patches render at once; default one per core.
    #[arg(long)]
    pub threads: Option<usize>,
}

/// Searches, writes the preset and prints the report.
pub fn run(args: &MatchArgs) -> Outcome {
    let bytes =
        fs::read(&args.reference).map_err(|e| format!("{}: {e}", args.reference.display()))?;
    let reference = lspk_test::agent::decode_wav(&bytes)
        .map_err(|e| format!("{}: {e}", args.reference.display()))?;
    let found = matching::search(
        &reference,
        &Args {
            evaluations: Some(args.evaluations),
            seed: args.seed,
            key: args.key,
            hold: args.hold,
            threads: args.threads,
        },
        &mut |_| {},
        &AtomicBool::new(false),
    )?;
    let mut instance = Instance::<Reagent>::new()?;
    instance.load_state(&found.patch.state(&Registry::new()))?;
    instance
        .plugin_mut()
        .run_command(preset::SAVE, &json!({ "path": args.out.to_string_lossy() }))
        .map_err(|e| e.message().to_owned())?;
    println!(
        "{:#}",
        json!({
            "distance": found.distance,
            "baseline": found.baseline,
            "evaluations": found.evaluations,
            "seconds": found.seconds,
            "key": found.key,
            "hold": found.hold,
        })
    );
    Ok(ExitCode::SUCCESS)
}
