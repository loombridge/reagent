//! `reagent-cli`: render, inspect and benchmark Reagent headlessly, with no
//! host and no window.

// A debug build counts heap use on audio threads (the real-time rules allow
// none); `render` and `benchmark` fail if they find any.
#[cfg(debug_assertions)]
lspk_rt::install_trap!();

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use reagent_cli::Outcome;

/// Reagent's headless tool.
#[derive(Debug, Parser)]
#[command(name = "reagent-cli", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Render a preset and a phrase, or a MIDI file, to a WAV file.
    ///
    /// The preset plays through the same engine code a host runs, faster
    /// than real time, with no audio device. The same preset and phrase
    /// render to the same samples every time and at any block size. Exits
    /// with status 1 if a file can't be read or written, a sample isn't
    /// finite, or (in a debug build) the engine touches the heap inside a
    /// block.
    Render(reagent_cli::render::RenderArgs),
    /// Read a preset and print what it holds.
    ///
    /// Its schema version, the modules it sets, each non-default value in
    /// its real unit and its choice by name, and the wavetables its
    /// oscillators name. Exits with status 1 if the file can't be read or
    /// isn't a preset this build reads.
    Inspect(reagent_cli::inspect::InspectArgs),
    /// Run Reagent's benchmark suite and print its table.
    ///
    /// The suite `cargo xtask bench` runs, timed on this machine. Build in
    /// release for numbers worth keeping. Exits with status 1 if a
    /// scenario's plug-in can't be made or given its state.
    Benchmark(reagent_cli::benchmark::BenchmarkArgs),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let outcome: Outcome = match &cli.command {
        Command::Render(args) => reagent_cli::render::run(args),
        Command::Inspect(args) => reagent_cli::inspect::run(args),
        Command::Benchmark(args) => reagent_cli::benchmark::run(args),
    };
    match outcome {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}
