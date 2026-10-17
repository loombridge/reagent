//! Reagent's `cargo xtask`: the plug-in kit's tasks, run in this repository.
//! Run `cargo xtask help` for the list.

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
    lspk_xtask::main(&REPO)
}
