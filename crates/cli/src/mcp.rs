//! `reagent-cli mcp`: the agent interface over stdio, with no window and no
//! host.
//!
//! An agent client starts `reagent-cli mcp` and speaks MCP on its standard
//! input and output. The tools are the plug-in kit's, the same ones the
//! plug-in serves inside a host (`lspk-plugin-agent`), over a Reagent
//! instance that nothing plays: an agent edits the patch with Reagent's
//! registered commands and hears it with `preview`, which renders a
//! separate copy.

use std::io;
use std::process::ExitCode;

use clap::Parser;
use lspk_plugin_agent::headless::Headless;
use reagent::Reagent;

use crate::Outcome;

/// The command a refusal names.
const COMMAND: &str = "reagent-cli mcp";

/// How to serve.
#[derive(Debug, Parser)]
pub struct McpArgs {
    /// Let agents run destructive and external commands without asking.
    /// There is no person to ask here, so without this they are refused.
    #[arg(long)]
    pub autonomous: bool,
}

/// Serves the agent interface until standard input ends.
pub fn run(args: &McpArgs) -> Outcome {
    let mut instance = Headless::<Reagent>::new()?;
    eprintln!(
        "{COMMAND}: serving the agent interface on standard input and output{}",
        if args.autonomous {
            " (autonomous: destructive and external commands run)"
        } else {
            ""
        }
    );
    instance
        .serve(
            COMMAND,
            args.autonomous,
            io::stdin().lock(),
            io::stdout().lock(),
        )
        .map_err(|error| format!("{COMMAND}: {error}"))?;
    Ok(ExitCode::SUCCESS)
}
