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
reagent-cli match --reference note.wav --out patch.json [--evaluations 600] [--seed 0] [--key 48] [--hold 1.0] [--threads n] [--random-start]
reagent-cli train-match --dataset dir [--dataset dir ...] --out model.bin [--hidden 64] [--epochs 80] [--seed 0] [--threads n]
reagent-cli compare-match --dataset dir [--cases 20] [--evaluations 150] [--threads n]
reagent-cli match-benchmark [--out results.json] [--budgets 150,600] [--seeds 3] [--references 20] [--threads n] [--write-references dir] [--scratch dir -- <command>]
reagent-cli text-to-patch ["a dark, wide pad"] [--out results.json]
reagent-cli mcp [--autonomous]
```

- **render** plays a preset through the same engine code a host runs, faster than real time, and writes a 32-bit float WAV. The same preset and phrase render to the same samples every time and at any block size. `--midi` plays a Standard MIDI File instead of a built-in phrase. A debug build fails if the engine touches the heap inside a block.
- **inspect** reads a preset with the plug-in kit's preset reader and prints its schema version, the modules it sets, each non-default value in its real unit and its choice by name, and the wavetables its oscillators name.
- **benchmark** times the same suite `cargo xtask bench` runs and prints its table; build in release for numbers worth keeping.
- **dataset** draws random, playable patches from a seed, renders each, and writes the patches with features measured on the renders, for training and for sound matching (below).
- **match** searches for a patch that sounds like a recording and writes it as a preset (below). `--random-start` leaves out the learnt model's patch.
- **train-match** trains the model that proposes where `match` starts, from datasets `dataset` made, and **compare-match** measures it against random starts on a dataset it never saw (below).
- **match-benchmark** scores matching on a fixed set of twenty references (below).
- **text-to-patch** builds a patch from a description by the loop an agent runs: a recipe for the instrument word (bass, lead, pad or pluck), then for each quality word (dark, bright, wide, slow, snappy) it renders a note, reads the perception features and moves one control until the reading fits, and last it sets the level. It prints the patch's parameters, each step and the final readings as JSON. With no description it runs the evaluation set (below) and exits `1` if a case fails its judge.
- **mcp** serves the agent interface over standard input and output, for an agent client to start. It offers the same tools the plug-in offers inside a host: `list_commands`, `describe_command`, `run` (a batch is all or nothing, and `dry_run` checks without changing anything), `query` (every parameter by ID, name or path, with a summary of the signal path), `checkpoint`, `preview` (renders a note, chord or phrase and returns the audio with an analysis) and the guides. Destructive and external commands, such as saving and loading preset files, are refused with the reason unless `--autonomous` is given. It exits `0` when standard input ends.

## Datasets

`reagent-cli dataset` makes a folder from a seed and a count. Patch `i` is drawn from the seed and `i` alone, so the same seed and count make the same folder byte for byte, on any number of threads.

- The sampler draws what a player would: oscillator A always sounds, through Filter 1 or straight to Main; oscillator B, the sub, the noise, a modulation envelope on the cutoff and an LFO come and go; levels, times, cutoffs and resonance stay in playing ranges. A patch is kept only if its render is finite, no quieter than −50 dBFS RMS and no louder than 0 dBFS; `attempts` says how many draws that took.
- Every patch plays MIDI key 48 at velocity 0.8, held for 1 s, in a render of 1.5 s at 48 kHz.
- `dataset.json` holds the format version (1), the seed, the count, the sample rate, the note and what each feature means.
- `patches.jsonl` holds one JSON object per line, in index order: `index`, `attempts`, `params` (each parameter the patch sets as `[id, value]`, the value in real units; every other parameter is at its default), `routes` (`source` key, `destination` parameter ID, `amount`, `polarity`) and `features`: `peak_db`, `rms_db`, `centroid_hz`, `pitch_hz`, `onset_s`, `attack_end_s` and `decay_end_s`.
- With `--audio`, `audio/<index>.wav` holds each render, the index in eight digits (`audio/00000042.wav`), as a 32-bit float WAV (about 0.55 MB each).

Throughput on an Apple M1 Pro (10 cores), release build, 1,000 patches of seed 1, no audio files: 7,300 patches a minute on 10 threads, and 1,400 a minute on one. With `--audio` on 3 threads it writes 3,800 a minute.

## Matching a sound

`match_sound` takes a recording of one held note and searches Reagent's parameters for a patch that sounds like it. It renders candidate patches offline, reads each with the plug-in kit's perception features (the spectrum over time, the envelope, the pitch, the rate and depth of any wobble in level or brightness, the stereo width) and keeps the one whose features are nearest the recording's by the kit's distance. The search is gradient-free, since the engine isn't differentiable and some parameters are choices: the learnt model's patch (below) and random points over the whole space, then three evolution-strategy chains from the best of them, each making children of a parent with noise that shrinks as the chain goes on. The same recording, seed and budget find the same patch on any number of threads.

- **What it can set:** oscillators A and B (octave, frame, unison and detune for A, fine pitch for B, levels), Filter 1 (on, type among the first 32, cutoff, resonance, drive), Env 1, Env 2's decay and sustain, LFO 1's rate, and the routes from Env 2 and LFO 1 to the cutoff: the ranges the dataset sampler draws from. The sub, the noise, the second filter and the effects are left at their defaults.
- **The commands** (so an agent reaches it through `run`): `reagent.match_sound` reads the WAV file, starts the search on a thread of its own and returns at once with its status; it is an external command, so a person is asked first, or `--autonomous` allows it. `reagent.match_status` reads the progress (candidates rendered of the budget, the best distance, the seconds) and, once the search ends, the patch (each parameter's ID, name and value, and the routes) with a report: the distance, the Init patch's distance to read it against, `closeness` (1 minus the first over the second) and the key and hold used. `reagent.match_apply` sets the patch, as `reagent.init_patch` and then its parameters; `reagent.match_cancel` stops the search and keeps its best so far.
- **The recording** is one note held and released, as a patch plays it. The key defaults to the one nearest its pitch, and the hold to two thirds of its length; give them when you know them. Up to the first 4 s are matched.
- **What a match recovers.** Features are coarser than parameters, so the patch it finds sounds like the recording and may be set differently: the unison count against the detune, or the filter type against its resonance, can trade off. For patches Reagent rendered itself (one oscillator through the filter, 1 s, 600 candidates, the six references of `the_search_recovers_the_parameters_of_patches_reagent_rendered`, run with `cargo nextest run --release`), the match renders to within 40 % of the Init patch's distance from the reference, plays the same octave and has the same release to within 15 %. That test is skipped in a debug build, where rendering is thirty times slower.

Time to a result, on an Apple M5 Max (18 cores, shared with other jobs), release build: the six searches of the test above take 11 s in all, about 1.8 s each; `reagent-cli match` on a 1.5 s dataset render takes 3.7 s for 600 candidates and 22 s for 3,000.

### The learnt starting point

A small neural network proposes a patch from the recording's perception features, and the search starts from it. It is trained only on Reagent's own renders, the datasets `reagent-cli dataset` makes, and runs inside Reagent with no network and no other file: its weights, `crates/core/src/match_model.bin` (160 KB), are compiled in. The input is the 284 numbers of the features (the spectrum and envelope, duration, level, pitch, the two modulations and the stereo image, each scaled to about 0 to 1); one hidden layer of 128 ReLU units and a linear layer give each dimension of the search space, clamped into its range. A choice such as the filter type comes out as the share of its range the network expects, which decodes to the choice in that share. The network is a start, not an answer: it is one candidate among the random points of the first stage, it starts a chain only if it beats them, and a recording unlike the training data costs the search one render.

- **Training data:** the patches of two datasets (`reagent-cli dataset --count 20000 --seed 100` and `--count 60000 --seed 101`, no audio) that the search space can make, 45,269 of them (patches with the sub or the noise on, or an LFO routed to a frame, are outside it). Each is rendered again for the features, since a dataset's own features are the simple ones. The network was made with `reagent-cli train-match --dataset a --dataset b --out crates/core/src/match_model.bin --hidden 128 --epochs 80`, which gives the same file for the same datasets. The dataset plays one key (48), so the network is best on recordings near it; retrain it when the search space changes (a test fails until you do).
- **Measured on held-out renders:** `evidence/match-model/` holds `reagent-cli compare-match` on 40 patches of a dataset with seed 1, which the network never saw, each matched twice with the same budget and seed. With 150 candidates, matching from the model's patch ends nearer in 25 of the 40 cases, at a mean distance of 0.94 against 1.08 from random points (the Init patch's is 3.66), and reaches the random search's final distance in fewer candidates in 15. With the default 600 candidates the two are level (0.676 against 0.684; nearer in 17 of 40): the model saves candidates, and the search closes the gap when it has them. `matching_from_the_models_patch_gets_nearer_than_from_random_points` runs the 150-candidate comparison; build in release (`cargo nextest run --release`), as a debug build skips it.


### The benchmark

`reagent-cli match-benchmark` scores `match_sound` on twenty references, notes rendered from patches Loombridge made (sixteen the dataset sampler draws from seed 2026, four designed by hand; half are patches the search can't set exactly), by the perception features' distance and the error in each reading (brightness, pitch, level, attack, duration, width). It also scores the same search from random points only and the Init patch, and, given a command after `--` (run as `program args… <reference.wav> <out.wav>`, with `--scratch` outside any repository), any other tool. The same arguments give the same scores on any number of threads. `--write-references dir` writes the references as WAV files. The results of a run are in [`evidence/match-benchmark/results.json`](evidence/match-benchmark/results.json): with 600 candidates, `match_sound` closes 71 % of the way from the Init patch to the reference on average (1 minus its distance over the Init patch's); the run takes five minutes in release.

## Text to patch

The recipes are in the guide resource, under "Sound-design recipes", for a model to follow through the agent interface. `reagent-cli text-to-patch` is a scripted agent that follows the same loop, so the loop and the judging can run without a model.

The evaluation set is `crates/cli/src/eval.json`: written descriptions, each with a judge, a range for each reading of the result's perception features (`brightness_oct`, `attack_s`, `width`, `pitch_hz`, `level_db`, `duration_s`). The judges don't know the loop's goals, so a model's patches are judged the same way. The latest run is recorded in [`evidence/text-to-patch/results.json`](evidence/text-to-patch/results.json), and a test checks that a fresh run passes and fails the same cases.

## Matching a reference

`reagent-match` sweeps one parameter on a reference instrument and on Reagent, and prints the pairs, the law each curve follows (linear, power, logarithmic or exponential, with its constants) and the level between them, as JSON.

```sh
reagent-match sweep --param "Osc A Level" --values 25,50,75,100 [--metric rms|pitch|centroid] [--key 60] --scratch DIR -- <command>
```

The reference is the command after `--`. For each value it's run as `<command> <case.json> <out.wav>`: it plays the case, one note on the reference's initial patch with the named parameters changed (`{"changes": [{"param", "value"}], "key", "velocity", "seconds", "rate"}`), and writes the audio to the WAV file. A command that hosts a real instrument headlessly is the maintainer's to write and keep; `reagent-match standin <case.json> <out.wav>` is Reagent in that role, which is how CI tests the harness without one. Renders are written only under `--scratch`, which must be outside any repository, and are deleted once measured.

## Licence

Reagent is licensed under [GPL-3.0-or-later](LICENSE).
