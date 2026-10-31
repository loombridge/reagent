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
| `reagent-match` | The matching harness: sweeps a parameter beside a reference instrument and fits the curves and levels |

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

1. Make this platform's installers, unsigned until release signing is set up:

   ```sh
   cargo xtask installer
   ```

   It writes `target/installer/dist`: a `.pkg` on macOS (it needs `pkgbuild` and `productbuild`, which come with Xcode's command-line tools), an `.msi` on Windows (it needs [WiX](https://wixtoolset.org) 5, `dotnet tool install --global wix --version 5.0.2`) and a `.deb` and an `.rpm` on Linux (they need `dpkg-deb` and `rpmbuild`). Each carries the CLAP, VST3 and, on macOS, Audio Unit plug-ins, the standalone app, and the factory content from `crates/content/factory`; the plug-ins go where hosts look for them, and `target/installer/manifest.txt` lists every path installed. CI's Installers workflow builds them on all three platforms, installs each, checks the manifest, uninstalls it and checks again (`installer/test-*`). The macOS package installs an `uninstall.sh` beside the factory content; Windows and Linux use their own uninstallers.

Reagent depends on the plug-in kit by git revision. To work on both at once, see [CONTRIBUTING.md](CONTRIBUTING.md#working-on-the-plug-in-kit-and-reagent-at-once).

## Headless

`reagent-cli` renders, inspects and benchmarks Reagent, and serves its agent interface, with no host and no window. Every verb exits `0` when it succeeds, and `1` with a message on standard error when it fails.

```sh
reagent-cli render --preset patch.json --out out.wav [--phrase chord|run] [--midi file.mid] [--rate 48000] [--block 512]
reagent-cli inspect --preset patch.json
reagent-cli benchmark [--quick]
reagent-cli dataset --out dir --count 1000 [--seed 0] [--audio] [--threads n]
reagent-cli mcp [--autonomous]
```

- **render** plays a preset through the same engine code a host runs, faster than real time, and writes a 32-bit float WAV. The same preset and phrase render to the same samples every time and at any block size. `--midi` plays a Standard MIDI File instead of a built-in phrase. A debug build fails if the engine touches the heap inside a block.
- **inspect** reads a preset with the plug-in kit's preset reader and prints its schema version, the modules it sets, each non-default value in its real unit and its choice by name, and the wavetables its oscillators name.
- **benchmark** times the same suite `cargo xtask bench` runs and prints its table; build in release for numbers worth keeping.
- **dataset** draws random, playable patches from a seed, renders each, and writes the patches with features measured on the renders, for training and for sound matching (below).
- **mcp** serves the agent interface over standard input and output, for an agent client to start. It offers the same tools the plug-in offers inside a host: `list_commands`, `describe_command`, `run` (a batch is all or nothing, and `dry_run` checks without changing anything), `query` (every parameter by ID, name or path, with a summary of the signal path), `checkpoint`, `preview` (renders a note, chord or phrase and returns the audio with an analysis) and the guides. Destructive and external commands, such as saving and loading preset files, are refused with the reason unless `--autonomous` is given. It exits `0` when standard input ends.

## Datasets

`reagent-cli dataset` makes a folder from a seed and a count. Patch `i` is drawn from the seed and `i` alone, so the same seed and count make the same folder byte for byte, on any number of threads.

- The sampler draws what a player would: oscillator A always sounds, through Filter 1 or straight to Main; oscillator B, the sub, the noise, a modulation envelope on the cutoff and an LFO come and go; levels, times, cutoffs and resonance stay in playing ranges. A patch is kept only if its render is finite, no quieter than −50 dBFS RMS and no louder than 0 dBFS; `attempts` says how many draws that took.
- Every patch plays MIDI key 48 at velocity 0.8, held for 1 s, in a render of 1.5 s at 48 kHz.
- `dataset.json` holds the format version (1), the seed, the count, the sample rate, the note and what each feature means.
- `patches.jsonl` holds one JSON object per line, in index order: `index`, `attempts`, `params` (each parameter the patch sets as `[id, value]`, the value in real units; every other parameter is at its default), `routes` (`source` key, `destination` parameter ID, `amount`, `polarity`) and `features`: `peak_db`, `rms_db`, `centroid_hz`, `pitch_hz`, `onset_s`, `attack_end_s` and `decay_end_s`.
- With `--audio`, `audio/<index>.wav` holds each render, the index in eight digits (`audio/00000042.wav`), as a 32-bit float WAV (about 0.55 MB each).

Throughput on an Apple M1 Pro (10 cores), release build, 1,000 patches of seed 1, no audio files: 7,300 patches a minute on 10 threads, and 1,400 a minute on one. With `--audio` on 3 threads it writes 3,800 a minute.

## Matching a reference

`reagent-match` sweeps one parameter on a reference instrument and on Reagent, and prints the pairs, the law each curve follows (linear, power, logarithmic or exponential, with its constants) and the level between them, as JSON.

```sh
reagent-match sweep --param "Osc A Level" --values 25,50,75,100 [--metric rms|pitch|centroid] [--key 60] --scratch DIR -- <command>
```

The reference is the command after `--`. For each value it's run as `<command> <case.json> <out.wav>`: it plays the case, one note on the reference's initial patch with the named parameters changed (`{"changes": [{"param", "value"}], "key", "velocity", "seconds", "rate"}`), and writes the audio to the WAV file. A command that hosts a real instrument headlessly is the maintainer's to write and keep; `reagent-match standin <case.json> <out.wav>` is Reagent in that role, which is how CI tests the harness without one. Renders are written only under `--scratch`, which must be outside any repository, and are deleted once measured.

## Licence

Reagent is licensed under [GPL-3.0-or-later](LICENSE).
