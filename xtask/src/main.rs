//! Reagent's `cargo xtask`: the plug-in kit's tasks, run in this repository.
//! Run `cargo xtask help` for the list; `reagent-parity` is Reagent's own.

mod installer;
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
        "reagent-match",
    ],
    downstream: no_crate,
    ..Repo::plugin(&["reagent"], lspk_xtask::REJECTS_AGPL)
};

/// No crate here is one the others mustn't depend on.
fn no_crate(_: &str) -> bool {
    false
}

const INSTALLER_USAGE: &str = "\
  installer      Make this platform's installers in target/installer/dist (a .pkg on
                 macOS, an .msi on Windows, a .deb and an .rpm on Linux) from the
                 plug-in bundles and the factory content, and list what they install in
                 target/installer/manifest.txt
";

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
    match args.first().map(String::as_str) {
        Some("installer") if args.len() == 1 => {
            // xtask sits directly under the workspace root.
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
            match installer::installer(&root) {
                Ok(()) => std::process::ExitCode::SUCCESS,
                Err(message) => {
                    eprintln!("error: {message}");
                    std::process::ExitCode::FAILURE
                }
            }
        }
        None | Some("help" | "-h" | "--help") => {
            let code = lspk_xtask::main(&REPO);
            print!("{INSTALLER_USAGE}");
            code
        }
        _ => lspk_xtask::main(&REPO),
    }
}
