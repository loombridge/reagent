# Reagent: a guide for agents

Reagent is a wavetable synthesiser. This guide is the resource `lspk://plugin-guide`; read `lspk://guide` first for the tools. Every control below is a parameter: `query` lists them all, and `params.set` changes them by name (`Osc A Level`) or path (`osc "A" / param "Level"`).

## The signal path

Each note is a voice. In a voice:

1. **Oscillators A, B and C** each play a wavetable. `Frame` picks the table's frame (0 % the first, 100 % the last; the default table holds a saw, a square, a triangle and a sine). Pitch is `Octave`, `Semitone`, `Fine` (cents) and `Sweep` (continuous semitones, the one to modulate). `Level` is per cent with a squared law: 75 % is −5 dB.
2. **Sub and Noise** (`Osc Sub`, `Osc Noise`): the sub plays one of six fixed shapes (`Shape`) at the note's pitch plus `Octave` and `Sweep`, starting each note at `Phase`; the noise plays Reagent's own `White`, `Pink`, `Brown` or `Crackle` colour noise (or, later, a user sample), with `Start`, `Start random`, `Rate`, `Fine`, `Key track`, `Loop`, `Stereo` and `Tone`. Both have the same `Output`, sends, `Level` and `Pan` as the oscillators, and both are modulation sources (`sub`, `noise`).
3. Each oscillator's **Output** is `Filter` (split between the filters by `Filter balance`, −100 all to Filter 1, +100 all to Filter 2), `Main`, `Dry out` (skips every effect) or `None` (silent, but still a modulation source). `Send 1` and `Send 2` copy it, after its level, to Bus 1 and Bus 2.
4. **Filter 1 and Filter 2**: `Type` (ladder low-pass, state-variable low-, high- and band-pass, peak and notch, at several slopes), `Cutoff`, `Resonance`, `Drive`, `Character` (saturation in the resonance path), `Stereo`, `Mix` and `Level`. A filter that's off passes its input on. Its `Output` is the other filter (the two in series), `Main`, `Dry out` or `None`. Routing a filter into one that already routes back sets the other to Main; buses work the same way.
5. **Env 1** shapes the voice's level wherever a signal leaves the voice, for each channel whose `Amp envelope` is on (the default). `Amp` scales each voice after it.

Then the **buses** go to Main, Dry out or each other, and **Main level** (−9 dB in a new patch) and **Dry out level** set the output.

## Modulation

Four envelopes (`Env 1` is also the amplifier), ten LFOs (synced to the tempo by default, a quarter note, restarting at each note), eight macros, velocity, the key, the mod wheel, pitch bend, pressure and more modulate any continuous parameter through the 64-slot matrix. Its commands are in the `mod` area: `mod.add_route` takes a source key (`env2`, `lfo1`, `velocity`, `macro1`, `mod_wheel`, ...), a destination (`{"param": <ID>}`, the ID `query` shows) and an amount from −1 to 1 of the destination's whole travel; `"polarity": "bipolar"` swings around the control's position instead of adding upward. `mod.list` shows the routes. `reagent.set_note_curve` redraws the velocity or the key curve, the graph a note's velocity or number is mapped through before it reaches the matrix; `reagent.swap_macros` exchanges two macros' routes, and `reagent.apply_macros` adds each macro's current offset into the parameters it drives and then removes the macro routes.

## Tuning and presets

Keys sound in twelve-tone equal temperament unless a tuning says otherwise. `tuning.load` tunes the instance by a `.tun` file or a Scala `.scl` (with its `.kbm` mapping in `keymap`); `tuning.clear` drops it. With no file, the instance follows an MTS-ESP master when one is connected (`tuning.set_mts` turns that off, or reads the master at each note's start only). A file always overrides MTS-ESP. `Reference pitch` and `Main tuning` apply on top of any tuning. `tuning.lock` keeps the tuning file and `Reference pitch` through a preset load. `preset.step` loads the next or previous preset among the files beside the last one loaded or saved; `preset.set_stepping` assigns the MIDI controllers or keys that do the same.

## Getting good results

- Start from the Init patch (`reagent.init_patch`): one saw on oscillator A through Filter 1, which is off.
- Turn Filter 1 on and lower `Cutoff` before adding resonance; route `env2` to the cutoff for a filter sweep, with Env 2's `Decay` and `Sustain` setting its shape.
- For width, detune B and C a few cents either side of A with `Fine`, pan them apart, and lower each level so the sum doesn't clip.
- Keep levels below full: three oscillators at 75 % with resonance can clip; preview and watch the peak.
- `Quality` High (the default) runs the voices at twice the host's rate; Ultra is cleaner and costs twice as much again.

## Working with the tools

- `checkpoint` (`save`) before a big change, so `restore` puts the whole patch back as one step if it goes wrong.
- Send related edits in one `run` batch: they land as one undo step, and if one fails nothing changes. Add `dry_run` to see what a batch would do first.
- `query` after an edit reads the patch back, in prose in `summary`, so check that it says what you meant before you listen.
- `preview` a note, a chord or a phrase after each change that you expect to be audible: it renders a separate copy of the patch and returns the audio and an analysis (pitch, loudness, spectrum, envelope, stereo width). Trust its numbers over your guess at what a setting sounds like.
- Commands that read or write files (`preset.load`, `preset.save`) ask the person first, or are refused where there is no person to ask; use the patch commands instead.
