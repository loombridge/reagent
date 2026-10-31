//! `reagent-cli`'s verbs, headless: render a preset and a phrase to a WAV,
//! print what a preset holds, run Reagent's benchmark suite, render datasets of
//! random patches, search for a patch that sounds like a recording, and serve
//! the agent interface over stdio.
//!
//! Each verb is a function here and a subcommand in the binary, so a test
//! drives it without a process. Every render runs the engine the way a host
//! does, one block after another on this thread, under the allocation trap a
//! debug build installs.

pub mod benchmark;
pub mod dataset;
pub mod inspect;
pub mod matching;
pub mod mcp;
pub mod render;

use std::process::ExitCode;

/// What a verb returns: an exit code, or a message for a failure.
pub type Outcome = Result<ExitCode, String>;
