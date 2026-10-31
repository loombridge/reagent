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
    /// Render a dataset of random, playable patches and measure each.
    ///
    /// Draws `--count` patches from `--seed`, plays each through the engine
    /// on every core, and writes a folder: `dataset.json`, `patches.jsonl`
    /// (each patch's parameters, routes and features) and, with `--audio`,
    /// the renders. The same seed and count make the same folder. Exits with
    /// status 1 if a file can't be written or no playable patch turns up.
    Dataset(reagent_cli::dataset::DatasetArgs),
    /// Search for a patch that sounds like a recording.
    ///
    /// Renders `--evaluations` candidate patches on every core, keeps the
    /// one whose perception features are nearest the reference's, writes it
    /// to `--out` as a preset, and prints how near it got and how long the
    /// search took. The same reference, seed and budget find the same patch.
    /// Exits with status 1 if a file can't be read or written, or the
    /// reference is silent.
    Match(reagent_cli::matching::MatchArgs),
    /// Build a patch from a description, or run the evaluation set.
    ///
    /// Picks the recipe for the instrument word (bass, lead, pad or pluck),
    /// then renders a note, reads the perception features and adjusts one
    /// control for each quality word (dark, bright, wide, slow, snappy)
    /// until the reading fits. With no description, runs the written
    /// evaluation set and judges each result. Exits with status 1 if a
    /// description has no instrument word, a file can't be written or an
    /// evaluation case fails its judge.
    TextToPatch(reagent_cli::text_to_patch::TextToPatchArgs),
    /// Serve the agent interface over standard input and output.
    ///
    /// An agent client starts this and speaks MCP on its standard input and
    /// output: discovery, `run` (a batch is one step, with a dry run),
    /// `query`, `preview`, `checkpoint` and the guide. Reads and edits run;
    /// destructive and external commands (saving and loading preset files)
    /// are refused, with the reason, unless `--autonomous` is given. Ends
    /// when standard input does, with status 0.
    Mcp(reagent_cli::mcp::McpArgs),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let outcome: Outcome = match &cli.command {
        Command::Render(args) => reagent_cli::render::run(args),
        Command::Inspect(args) => reagent_cli::inspect::run(args),
        Command::Benchmark(args) => reagent_cli::benchmark::run(args),
        Command::Dataset(args) => reagent_cli::dataset::run(args),
        Command::Match(args) => reagent_cli::matching::run(args),
        Command::TextToPatch(args) => reagent_cli::text_to_patch::run(args),
        Command::Mcp(args) => reagent_cli::mcp::run(args),
    };
    match outcome {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}
