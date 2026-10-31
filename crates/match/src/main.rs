//! `reagent-match`: sweep a parameter beside a reference instrument and
//! print the fitted curves and the level between them, as JSON.

// A debug build counts heap use on audio threads, as the other tools do.
#[cfg(debug_assertions)]
lspk_rt::install_trap!();

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use reagent_match::{Case, Change, External, Metric, Reference, Standin, Sweep, fit};

#[derive(Debug, Parser)]
#[command(name = "reagent-match", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Sweep one parameter on the reference and on Reagent, and print the
    /// pairs, the law each curve follows, and the level between them.
    ///
    /// The reference is the command after `--`, run as
    /// `program args… <case.json> <out.wav>` for each value: it plays the
    /// case on the reference and writes the audio to the WAV file. Renders
    /// stay in the scratch folder and are deleted once measured.
    Sweep {
        /// The parameter in Reagent, as a host shows it, such as `Osc A Level`.
        #[arg(long)]
        param: String,
        /// The reference's name for it, when it isn't the same.
        #[arg(long)]
        reference_param: Option<String>,
        /// The values to render, in the parameter's real unit.
        #[arg(long, value_delimiter = ',', required = true)]
        values: Vec<f64>,
        /// What to measure.
        #[arg(long, value_enum, default_value_t = Metric::Rms)]
        metric: Metric,
        /// The MIDI key held.
        #[arg(long, default_value_t = 60)]
        key: u8,
        /// Seconds per render.
        #[arg(long, default_value_t = 0.6)]
        seconds: f64,
        /// The sample rate.
        #[arg(long, default_value_t = 48_000.0)]
        rate: f64,
        /// Where the reference's renders are written, outside any repository.
        #[arg(long)]
        scratch: PathBuf,
        /// The command that hosts the reference.
        #[arg(last = true, required = true)]
        reference: Vec<String>,
    },
    /// Render a case on Reagent, as a reference command does.
    ///
    /// `reagent-match standin <case.json> <out.wav>` is what `sweep` runs
    /// when Reagent stands in for the reference; `--set` gives the stand-in
    /// a known difference.
    Standin {
        /// A change under every case, as `Name=value`.
        #[arg(long = "set")]
        base: Vec<String>,
        /// The case, as JSON.
        case: PathBuf,
        /// Where to write the WAV.
        out: PathBuf,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse().command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command) -> Result<(), String> {
    match command {
        Command::Sweep {
            param,
            reference_param,
            values,
            metric,
            key,
            seconds,
            rate,
            scratch,
            reference,
        } => {
            let sweep = Sweep {
                param,
                reference_param,
                values,
                metric,
                key,
                seconds,
                rate,
            };
            let pairs = reagent_match::sweep(
                &sweep,
                &mut External::new(reference, &scratch)?,
                &mut Standin::new(Vec::new()),
            )?;
            let curve = |pick: fn(&reagent_match::Pair) -> f64| {
                let points: Vec<_> = pairs.0.iter().map(|p| (p.x, pick(p))).collect();
                fit::fit(&points)
            };
            let report = serde_json::json!({
                "pairs": pairs,
                "reference_fit": curve(|p| p.reference),
                "reagent_fit": curve(|p| p.reagent),
                "level_offset_db": (metric == Metric::Rms).then(|| pairs.level_offset_db())
                    .flatten().map(|(mean, spread)| serde_json::json!({"mean": mean, "spread": spread})),
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
            );
            Ok(())
        }
        Command::Standin { base, case, out } => {
            let base = base
                .iter()
                .map(|set| {
                    let (param, value) = set.split_once('=').ok_or("--set takes Name=value")?;
                    Ok(Change {
                        param: param.to_owned(),
                        value: value
                            .parse()
                            .map_err(|_| format!("{value} isn't a number"))?,
                    })
                })
                .collect::<Result<_, String>>()?;
            let bytes = fs::read(&case).map_err(|e| format!("{}: {e}", case.display()))?;
            let case: Case = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            let audio = Standin::new(base).render(&case)?;
            fs::write(&out, lspk_test::agent::encode_wav(&audio)?)
                .map_err(|e| format!("{}: {e}", out.display()))
        }
    }
}
