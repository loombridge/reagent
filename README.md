# Reagent

Reagent is an open-source wavetable synthesiser written in Rust, for macOS (Apple Silicon), Windows and Linux. It aims to offer the capabilities of Serum 2, Xfer Records' wavetable synthesiser, in its own design and with its own sounds: wavetable oscillators with warp modes and unison, sub and noise, two filters, an effects rack, a modulation matrix, an arpeggiator and a clip sequencer.

It runs inside [Beatlayer](https://github.com/loombridge/beatlayer) as a native instrument, in other hosts as a CLAP, VST3 or Audio Unit plug-in, as a standalone app, and headless from the command line. Every action a person can take in Reagent, an agent can take too. Presets are documented JSON.

Reagent is built on the [plug-in kit](https://github.com/loombridge/plugin-kit), Loombridge's shared crates for audio plug-ins. It is in early development: the voice architecture plays (three wavetable oscillators with unison, two filters in series or parallel, buses, four envelopes, ten LFOs and a 64-slot modulation matrix) in every format, with the kit's generic editor; the sub and noise oscillators, the effects, Reagent's own interface and the rest are still to come.

Reagent isn't affiliated with or endorsed by Xfer Records. Serum is a trademark of Xfer Records. How Reagent learns from Serum 2 without using its code or content is in [CONTRIBUTING.md](CONTRIBUTING.md#building-without-xfers-code).

## Crates

| Crate | What it holds |
|---|---|
| `reagent-engine` | The synthesiser: oscillators, filters, the mixer and routing, envelopes, LFOs, the modulation matrix and macros; the effects rack, the arpeggiator and the clip sequencer to come |
| `reagent-content` | Factory presets and wavetables, all made by Loombridge, and the scripts that generate them |
| `reagent-import` | Importers for presets and wavetables made in other synthesisers |
| `reagent-ui` | Reagent's interface, built from the plug-in kit's widgets |
| `reagent-core` | The plug-in without a format export or an editor: its parameters, presets and hand-built patches, headless |
| `reagent` | The plug-in (CLAP, VST3 and Audio Unit v2) and the standalone app, with the editor and the agent interface, made from `reagent-core` |
| `reagent-cli` | Headless: render, inspect, convert, benchmark, and the agent interface over stdio |

## Develop

1. Install Rust with [rustup](https://rustup.rs). The repository pins the toolchain, so rustup fetches the right version on first build.
1. Install cargo-deny, which checks dependency licences and advisories, and cargo-nextest, which runs the tests in parallel:

   ```sh
   cargo install --locked cargo-deny
   cargo install --locked cargo-nextest@0.9.146
   ```

1. Run every check: format, clippy, tests, crate layering, and the dependency policy.

   ```sh
   cargo xtask check
   ```

1. Build Reagent in every format and run each format's validators (clap-validator, Steinberg's VST3 validator, and on macOS auval, and pluginval where it's installed):

   ```sh
   cargo xtask validate
   ```

   `cargo xtask bundle reagent` lays the plug-ins and the standalone app out in `target/bundle`, `cargo xtask golden` checks the golden renders, and `cargo xtask bench` times the benchmarks.

Reagent depends on the plug-in kit by git revision. To work on both at once, see [CONTRIBUTING.md](CONTRIBUTING.md#working-on-the-plug-in-kit-and-reagent-at-once).

## Headless

`reagent-cli` renders, inspects and benchmarks Reagent, and serves its agent interface, with no host and no window. Every verb exits `0` when it succeeds, and `1` with a message on standard error when it fails.

```sh
reagent-cli render --preset patch.json --out out.wav [--phrase chord|run] [--midi file.mid] [--rate 48000] [--block 512]
reagent-cli inspect --preset patch.json
reagent-cli benchmark [--quick]
reagent-cli mcp [--autonomous]
```

- **render** plays a preset through the same engine code a host runs, faster than real time, and writes a 32-bit float WAV. The same preset and phrase render to the same samples every time and at any block size. `--midi` plays a Standard MIDI File instead of a built-in phrase. A debug build fails if the engine touches the heap inside a block.
- **inspect** reads a preset with the plug-in kit's preset reader and prints its schema version, the modules it sets, each non-default value in its real unit and its choice by name, and the wavetables its oscillators name.
- **benchmark** times the same suite `cargo xtask bench` runs and prints its table; build in release for numbers worth keeping.
- **mcp** serves the agent interface over standard input and output, for an agent client to start. It offers the same tools the plug-in offers inside a host: `list_commands`, `describe_command`, `run` (a batch is all or nothing, and `dry_run` checks without changing anything), `query` (every parameter by ID, name or path, with a summary of the signal path), `checkpoint`, `preview` (renders a note, chord or phrase and returns the audio with an analysis) and the guides. Destructive and external commands, such as saving and loading preset files, are refused with the reason unless `--autonomous` is given. It exits `0` when standard input ends.

## Licence

Reagent is licensed under [GPL-3.0-or-later](LICENSE).
