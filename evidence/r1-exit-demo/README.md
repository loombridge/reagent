# R1 exit demo: three patches from text

An agent, headless through `reagent-cli mcp`, built three patches from text descriptions, heard each one with `preview`, adjusted it and saved it. These files are the proof.

For each patch (`bass`, `pad`, `lead`):

- `<name>.transcript.json` holds the description the agent was given and every call it made to the agent interface, in order, from the empty Init patch to the saved preset. `{preset}` stands for the path the replay saves to.
- `<name>.preset.json` is the preset that transcript builds.
- `<name>.analysis.json` is the analysis `preview` returned for the final version.
- `renders/<name>.flac` is that final preview, 24-bit, with `renders/manifest.json` recording the platform that made it.

## What replays in CI

`crates/cli/tests/golden.rs` plays each transcript through the plug-in kit's scripted MCP client against a real `reagent-cli mcp --autonomous` process, on all three platforms, and checks that:

- the preset it saves is the kept preset;
- the final preview null-tests against the kept render (bit-exact on the platform that made it, below -96 dBFS elsewhere);
- the analysis fits the description: the bass is low, the pad is wide and its brightness moves, the lead is bright;
- every command a transcript runs is registered (the check fails on a command that isn't);
- each kept preset loads in a build of Reagent without its exports or editor, which is how Beatlayer's native devices carry it, and plays notes.

To remake the kept files after a change that moves the sound, run `cargo xtask golden --update` and say why in the pull request.

## Listening check (RH3)

For the maintainer to listen to, then to load the three presets in Beatlayer and in the shipped formats (CLAP, VST3, Audio Unit, standalone):

| Patch | Render | Preset |
| --- | --- | --- |
| Bass | `renders/bass.flac` | `bass.preset.json` |
| Pad | `renders/pad.flac` | `pad.preset.json` |
| Lead | `renders/lead.flac` | `lead.preset.json` |

Verdict: not recorded yet.
