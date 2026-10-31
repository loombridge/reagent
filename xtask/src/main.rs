//! Reagent's `cargo xtask`: the plug-in kit's tasks, run in this repository.
//! Run `cargo xtask help` for the list; `reagent-parity` is Reagent's own.

mod parity;

use lspk_xtask::Repo;

/// This repository to the kit's tasks: its plug-in crate, which `cargo xtask
/// validate` checks when none is named; the crates that build and test
/// headless, which never depend on a UI crate or reach a UI toolkit; and
/// the licence in the kit's fixture that deny.toml must reject.
const REPO: Repo = Repo {
    headless: &[
        "xtask",
        "reagent-engine",
        "reagent-content",
        "reagent-import",
        "reagent-core",
        "reagent-cli",
    ],
    downstream: no_crate,
    ..Repo::plugin(&["reagent"], lspk_xtask::REJECTS_AGPL)
};

/// No crate here is one the others mustn't depend on.
fn no_crate(_: &str) -> bool {
    false
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|task| task == "reagent-parity") {
        return match parity::run(&args[1..]) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("error: {message}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    lspk_xtask::main(&REPO)
}
