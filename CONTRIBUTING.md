# Contributing to Reagent

Thank you for helping build Reagent. This page explains how a change gets from your machine into `main`: what to read first, how work is organised, the checks every change passes, and the certificate you give with each commit.

## Read first

1. [README.md](README.md) and [AGENTS.md](AGENTS.md): what Reagent is, the lines no change crosses, and how work is done.
1. [Building without Xfer's code](#building-without-xfers-code): what you may and may not learn from Serum 2. Read it before you write anything.

## Building without Xfer's code

Reagent matches what Xfer Records' Serum 2 does, not how Serum's code does it. In one line: learn from what Serum 2 shows any user, never from inside its binaries, and never copy Xfer's content. Every contribution, from a person or an agent, keeps to these rules:

- **Learn from what Serum 2 shows any user:** Xfer's public manual, product pages, release notes and changelogs; published reviews, tutorials and walkthrough videos; running a licensed copy and watching what it does; measuring its audio output from patches you made yourself; and presets and wavetables you made yourself, for reading the file formats. Paraphrase; never copy text.
- **Never look inside Serum:** don't open, unpack or analyse the code of Serum or any Xfer binary, by any tool.
- **Never copy Xfer's content:** no code, manual text, factory presets, wavetables, samples, noise sources, skins, artwork, icons or fonts, in the repository or in Reagent's content. Presets and wavetables you made in Serum 2 stay on your machine; tests generate their own fixtures. Don't commit audio Serum 2 rendered.
- **Never use Xfer's trademarks as names.** "Serum" never appears in Reagent's interface, preset names or product name. Reagent's modules and features have their own names.
- **Never copy Serum's graphic design.** Reagent puts the same controls in the same places where function drives it, so a Serum user finds them, but draws its own panels, knobs, colours and layout details.
- **Open-source synthesisers, within their licences.** Reagent is GPL-3.0-or-later, so its own crates may port code from GPL-3.0 projects such as Surge XT and Vital, with a header comment in each derived file naming the source. That code never goes into the plug-in kit, which is MIT OR Apache-2.0. Their names and branding are their authors'.
- **Cite your sources.** A change that reproduces Serum 2's behaviour says in its pull request what it rests on: a manual section, an observation of Serum 2 running (with its version and platform), a measurement with its method, or a published note.

If you have ever studied the inside of Xfer's code, say so in your pull request and don't work on the matching subsystem.

## Find something to work on

- **Work packages.** Most work is a package a maintainer gives you. Each package has its own branch, `wp/<id>-<slug>`, and its own pull request.
- **Bugs and features.** Open an issue first. Describe Serum 2's behaviour in your own words; don't paste Xfer's text or attach Xfer's files.
- **Small fixes**, such as a typo or a broken link, can go straight to a pull request.

## Make the change

1. Build to the acceptance criteria. Each criterion is a test or a command whose output you paste into the pull request.
1. Give every feature a headless test path, through `reagent-cli` or a unit test. A feature that can only be tested by clicking isn't done.
1. Register every user-facing action as a command, with a schema, a model-facing description and a permission class, so agents can reach it.
1. Keep to the rules in [AGENTS.md](AGENTS.md): no allocation, lock, or blocking call on an audio thread; the engine, content and import crates never depend on a UI crate; no dependency that `cargo deny` rejects.
1. If the change would serve a second plug-in too, it belongs in the plug-in kit: make it there first (see the next section).
1. Run the checks. They must pass before you open the pull request.

   ```sh
   cargo xtask check
   ```

   It runs format, clippy with warnings denied, the tests under cargo-nextest and then the doctests, the layer rules, `cargo deny`, and a self-test that proves `deny.toml` still rejects an incompatible licence. Reagent's `cargo xtask` is the plug-in kit's tasks run in this repository, so `cargo xtask validate`, `golden`, `bench` and `bundle` are there too (`cargo xtask help`).

1. If the change moves Reagent's sound, remake the golden renders with `cargo xtask golden --update` and say why in the pull request. If it adds a parameter, run `LSPK_IDS_UPDATE=1 cargo test -p reagent --test ids` and commit `crates/plugin/tests/ids.txt`, which may only be added to. CI's Linux job fails a pull request that makes any benchmark more than 3 % slower than `bench/baseline.json`; when that's intended, update it with `cargo xtask bench --update-baseline --reason "..." --from <counts>` from the counts the job prints, and say why.

## Working on the plug-in kit and Reagent at once

Reagent depends on the [plug-in kit](https://github.com/loombridge/plugin-kit) by git revision: every `lspk-` crate in the workspace `Cargo.toml` names the same `rev`. When a change needs the kit too, point Cargo at a local checkout of the kit with a `[patch]` section, so both build together as you work.

1. Clone the kit beside Reagent, so the folder holding both looks like this, and start a branch in each:

   ```text
   code/
     plugin-kit/
     reagent/
   ```

1. Create `.cargo/config.toml` in the folder that holds both checkouts (`code/` above), not in either repository, with one line for each kit crate Reagent depends on. Paths are relative to that folder:

   ```toml
   [patch."https://github.com/loombridge/plugin-kit"]
   lspk-agent = { path = "plugin-kit/crates/agent" }
   lspk-commands = { path = "plugin-kit/crates/commands" }
   lspk-dsp = { path = "plugin-kit/crates/dsp" }
   lspk-mod = { path = "plugin-kit/crates/mod" }
   lspk-params = { path = "plugin-kit/crates/params" }
   lspk-plugin = { path = "plugin-kit/crates/plugin" }
   lspk-plugin-agent = { path = "plugin-kit/crates/plugin-agent" }
   lspk-presets = { path = "plugin-kit/crates/presets" }
   lspk-rt = { path = "plugin-kit/crates/rt" }
   lspk-standalone = { path = "plugin-kit/crates/standalone" }
   lspk-test = { path = "plugin-kit/crates/test" }
   lspk-ui = { path = "plugin-kit/crates/ui" }
   lspk-voice = { path = "plugin-kit/crates/voice" }
   lspk-wavetable = { path = "plugin-kit/crates/wavetable" }
   lspk-xtask = { path = "plugin-kit/xtask" }
   ```

   Cargo reads it from every checkout below that folder, and it patches only crates that come from the kit's repository. Reagent now builds against your kit checkout, and `cargo xtask check` passes in both repositories as you work. Cargo rewrites `Cargo.lock` to drop the kit's git source while the patch is on; don't commit it in that state.

1. When the kit's half is ready, push its branch and open its pull request.
1. Turn the patch off (rename or delete that `.cargo/config.toml`), set `rev` in Reagent's `Cargo.toml` to the kit commit your change needs, on every `lspk-` line, and run `cargo xtask check`. Cargo updates `Cargo.lock` to the new revision. Commit `Cargo.toml` and `Cargo.lock` with the rest of the change, and say in Reagent's pull request which kit pull request it needs.
1. The kit's pull request merges first, with a merge commit, so the revision stays reachable from the kit's `main`. Reagent's then merges, pinned to that revision or to the kit's `main` after it.

To patch for one command only, without a config file, pass the same table on the command line, for example `cargo --config 'patch."https://github.com/loombridge/plugin-kit".lspk-dsp.path="../plugin-kit/crates/dsp"' check`.

## Commit

- Stage files by path. Never use `git add -A` or `git add .`, and check `git status` so build output, `.DS_Store`, and scratch files stay out.
- Start the subject line with the package ID when there is one.
- Sign off every commit, as the next section explains.

## Developer Certificate of Origin

Reagent takes contributions under the [Developer Certificate of Origin](https://developercertificate.org) (DCO), version 1.1, instead of a contributor licence agreement. You keep the copyright in your work. When you sign off a commit, you certify the statements below: in short, that you wrote the change, or otherwise have the right to submit it under the project's open-source licence, and that the record of your contribution is public.

```text
Developer Certificate of Origin
Version 1.1

Copyright (C) 2004, 2006 The Linux Foundation and its contributors.

Everyone is permitted to copy and distribute verbatim copies of this
license document, but changing it is not allowed.


Developer's Certificate of Origin 1.1

By making a contribution to this project, I certify that:

(a) The contribution was created in whole or in part by me and I
    have the right to submit it under the open source license
    indicated in the file; or

(b) The contribution is based upon previous work that, to the best
    of my knowledge, is covered under an appropriate open source
    license and I have the right under that license to submit that
    work with modifications, whether created in whole or in part
    by me, under the same open source license (unless I am
    permitted to submit under a different license), as indicated
    in the file; or

(c) The contribution was provided directly to me by some other
    person who certified (a), (b) or (c) and I have not modified
    it.

(d) I understand and agree that this project and the contribution
    are public and that a record of the contribution (including all
    personal information I submit with it, including my sign-off) is
    maintained indefinitely and may be redistributed consistent with
    this project or the open source license(s) involved.
```

### How to sign off

A sign-off is a `Signed-off-by` line at the end of the commit message, with your name and email address. Git adds it from your `user.name` and `user.email` settings when you commit with `-s`:

```sh
git commit -s -m "Fix the filter's drive range"
```

The message then ends with a line like this one:

```text
Signed-off-by: Your Name <you@example.com>
```

Use the name you're known by in the project and an email address that reaches you. Anonymous sign-offs aren't accepted.

If you forget:

- For the last commit, run `git commit --amend --no-edit --signoff`.
- For every commit on your branch, run `git rebase --signoff origin/main`, then `git push --force-with-lease`.

To see which commits on your branch are signed off, run:

```sh
git log origin/main..HEAD --format='%h %s%n  %(trailers:key=Signed-off-by,valueonly,separator=%x2C )'
```

A commit with nothing on the indented line under it isn't signed off.

### Changes written with an AI agent

If an AI coding agent wrote some or all of your change, you're still the person certifying it. Review the change, then sign it off yourself. The agent records its part with a `Co-Authored-By` trailer, and never adds a `Signed-off-by` line, in your name or its own, because only a person can give the certificate.

Commits made by the project itself, authored as Loombridge, carry no sign-off.

### Checking sign-offs

For now, the reviewer checks sign-offs by hand.

## Open a pull request

- Open it against `main`. For a work package, title it `<id>: <package title>`.
- Say what the change adds, paste the acceptance evidence and your `cargo xtask check` summary, cite the sources any Serum 2 behaviour rests on, and list anything that needs a maintainer.
- Update any document whose facts your change alters, in the same pull request.
- A maintainer reviews and merges. Don't merge your own pull request.

## Write in the house style

Write prose, including documents, code comments, commit messages, and pull requests, in plain British English, with sentence-case headings. Address the reader in the second person. `rustfmt` and `clippy` settle the code's formatting, and `cargo xtask check` runs both.

## Licence

Reagent is licensed under GPL-3.0-or-later ([LICENSE](LICENSE)), and you contribute under the same licence, certifying each commit with the Developer Certificate of Origin.
