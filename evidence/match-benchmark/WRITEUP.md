# Matching a sound: a benchmark

**Draft, not published.** This file is not linked from the README or the release notes, and it should stay that way until the comparison section below has been reviewed. It makes no claim about any tool except Reagent's own.

Sound matching takes a recording of one note and finds a synthesiser patch that sounds like it. This benchmark measures how near Reagent's `match_sound` gets, on a fixed set of references, by metrics anyone can recompute, and it can score any other matching tool by the same measures.

## Reproduce it

```sh
cargo run --release -p reagent-cli -- match-benchmark --out results.json
```

The results of the run recorded here are in [`results.json`](results.json). The same arguments give the same scores, on any number of threads (a test checks it); the only value that differs from run to run is `mean_seconds`. The run took five minutes on an Apple silicon Mac, release build, ten render threads.

To score another tool, give the command that drives it after `--`. It is run once per reference as `program args… <reference.wav> <out.wav>`: it hears the reference and writes the sound it made to match it. `--scratch` names a folder outside any repository for those files, and they are deleted once read. `--write-references DIR` writes the references as WAV files, for matching them by hand in a tool that can't be scripted.

## The references

Twenty notes, all made by Loombridge and rendered from patches in this repository, so no third-party audio is committed:

- **Sixteen drawn** by the dataset sampler from seed 2026, the patches a player might make: one or two wavetable oscillators, often a sub or noise, a filter, an envelope and an LFO.
- **Four designed** by hand: a unison lead, a sixteen-voice unison, a bass and a pad moved by LFOs.

Each plays one key held for one second, in a render of one and a half seconds at 48 kHz. The search isn't told the key or the hold. Ten of the twenty are patches `match_sound` could set exactly (`in_space` in the results); the other ten use a sub, the noise or modulation it can't set, so the best it can do is nearby.

## The metrics

Both come from the plug-in kit's perception features: the spectrum over time, the envelope, the pitch, the rate and depth of any wobble in level or brightness, and the stereo image.

- **Distance:** the kit's distance between the features of the reference and of what a tool made. 0 is the same features; about 1 is a difference a listener calls large (12 dB of spectrum, an octave of pitch). A sound that renders silence counts as 100.
- **Closeness:** one minus the distance over the Init patch's distance from the same reference. 1 is a match, 0 is no nearer than the Init patch.
- **Reading errors:** the mean absolute difference in the readings a listener can name: brightness in octaves, pitch in cents, level in dB, attack and duration in seconds, stereo width from 0 to 1.

`match_sound` searches by minimising the distance, so the distance favours it over a tool that doesn't. The reading errors don't: they are the fairer comparison, and the table shows both.

## Results

Means over the twenty references, and over three seeds for the searches. "Random search" is the same search from random points only, without the learnt starting point.

| | Distance (mean) | Distance (median) | Closeness | Nearest on |
|---|---|---|---|---|
| Init patch | 2.80 | 2.55 | 0.00 | |
| `match_sound`, 150 candidates | 1.00 | 0.92 | 0.58 | 0 |
| Random search, 150 | 1.08 | 0.98 | 0.55 | 0 |
| `match_sound`, 600 candidates | 0.70 | 0.62 | 0.71 | 11 |
| Random search, 600 | 0.74 | 0.62 | 0.70 | 9 |

("Nearest on" counts the references where a tool had the lowest mean distance, the Init patch left out.)

| Mean absolute error | Init | `match_sound` 150 | `match_sound` 600 | Random search 600 |
|---|---|---|---|---|
| Brightness (octaves) | 0.80 | 0.17 | 0.10 | 0.10 |
| Pitch (cents) | 6.7 | 87 | 30 | 9.0 |
| Level (dB) | 4.1 | 5.1 | 2.0 | 2.7 |
| Attack (s) | 0.085 | 0.083 | 0.055 | 0.063 |
| Duration (s) | 0.28 | 0.088 | 0.029 | 0.029 |
| Stereo width | 0.25 | 0.10 | 0.067 | 0.066 |

Split by whether `match_sound` could make the reference exactly, at 600 candidates (the means of the per-reference distances in `results.json`):

| | Init | `match_sound` | Random search |
|---|---|---|---|
| In reach (10) | 3.16 | 0.61 | 0.65 |
| Out of reach (10) | 2.44 | 0.79 | 0.83 |

What the numbers say, and what they don't:

- With 600 candidates `match_sound` closes about 71 % of the way from the Init patch to the reference, on average, and gets less near on references it can't make exactly: the closeness is 0.78 in reach and 0.64 out of it.
- The learnt starting point helps at a small budget (distance 1.00 against 1.08 at 150 candidates) and little at 600 (0.70 against 0.74). Each is nearest on about half the references.
- Pitch is its weakest reading. The distance weighs an octave of pitch about the same as 12 dB of spectrum, so a few tens of cents cost it little, and the search can leave a patch tens of cents off the reference's pitch. With 600 candidates the mean error is 30 cents against 9 for the random search, and the random search's smaller pitch error didn't make it nearer overall. This is where the search is worth improving.
- Twenty references and three seeds are few. A difference of 0.04 in mean distance is within what a different reference set would move.

## Other tools

None was run for this draft: the comparison needs a tool that can be driven headlessly, and none ships with Reagent. The harness is ready for one. Results from another tool belong in this section only after they have been reviewed, with the tool's name and version, how it was driven, and the full `results.json` beside this file.

Things to settle in that review:

- That the references and the metrics treat another tool fairly: the distance favours `match_sound`, and the references are patches Reagent can render, so a tool whose instrument can't make them is at a disadvantage the table doesn't show. The reading errors and the in-reach split go some way to answering both.
- How a tool is named, and whether any comparison is published at all.
