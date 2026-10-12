# AGENTS.md

Shared context for any agent or person working in this repository. Keep it current and short; private notes belong in your own memory, not here.

## What this is

Reagent is an open-source wavetable synthesiser with the capabilities of Xfer Records' Serum 2, written in Rust for macOS (Apple Silicon), Windows, and Linux. It ships as a native Beatlayer instrument, as CLAP, VST3 and Audio Unit plug-ins, as a standalone app, and headless through `reagent-cli`. A Serum 2 user should find the same capabilities, in Reagent's own design and with Reagent's own words where Xfer's are product names.

Reagent is built on the plug-in kit (`loombridge/plugin-kit`, the `lspk-` crates). Anything a second plug-in would need belongs in the kit; Reagent's own crates hold only what makes it Reagent.

## Read first

1. [README.md](README.md): what Reagent is and how to build it.
2. [CONTRIBUTING.md](CONTRIBUTING.md), and in it [Building without Xfer's code](CONTRIBUTING.md#building-without-xfers-code): what you may and may not learn from Serum 2. Read it before you write anything.

The plan isn't published yet; a maintainer gives you the work package and the context it needs.

## Lines not crossed

- No Xfer code, content, trademarks or graphic design, by any route. Never decompile, disassemble or statically analyse Serum or any Xfer binary.
- Code ported from GPL projects (Surge XT, Vital) stays in Reagent's own crates, with a header comment naming its source, and never goes into the plug-in kit.
- Every user-facing action is a registered command with a schema, a description, and a permission class, so agents reach every feature through the agent interface.
- No allocation, lock, or blocking call on an audio thread.
- `reagent-engine`, `reagent-content` and `reagent-import` never depend on `reagent-ui` or `lspk-ui`, or reach a UI toolkit. `cargo xtask layers` enforces this.
- Every feature has a headless test path.
- No dependency whose licence `cargo deny` rejects. No plug-in binaries, Xfer files, or third-party audio in the repository.
- No `git add -A`.

## How work is done

One work package per branch (`wp/<id>-<slug>`) and per pull request, with acceptance evidence pasted in the PR.

Before a PR, `cargo xtask check` must pass on macOS locally, with its summary pasted in the PR, as must Windows clippy (`CARGO_FEATURE_PURE=1 cargo clippy --workspace --all-targets --all-features --locked --target x86_64-pc-windows-msvc -- -D warnings`). CI runs the check on macOS, Windows and Linux on every pull request and every push to `main`.

Reagent depends on the plug-in kit by git revision (`rev` in the workspace `Cargo.toml`). A change that needs the kit lands in the kit first; [CONTRIBUTING.md](CONTRIBUTING.md#working-on-the-plug-in-kit-and-reagent-at-once) explains working on both at once.

Commits: the project's own commits are authored as `Loombridge <code@loombridge.io>`. Set it once per checkout: `git config user.name Loombridge && git config user.email code@loombridge.io`. Outside contributors commit under their own names with the Developer Certificate of Origin ([CONTRIBUTING.md](CONTRIBUTING.md)).

Merging: a maintainer merges a pull request when `cargo xtask check` and CI pass, review finds no blocker, and the acceptance evidence is complete.

One-time setup: install Rust with rustup (the repository pins the toolchain in `rust-toolchain.toml`), then `cargo install --locked cargo-deny` and `cargo install --locked cargo-nextest@0.9.146`.
