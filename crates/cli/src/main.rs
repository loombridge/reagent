//! Will hold Reagent headless: render a preset and MIDI to audio, convert and inspect presets and wavetables, benchmark, and serve the agent interface over stdio (`reagent-cli mcp`).

// The CLI drives the engine without a host; this keeps the dependency linked
// until the first command uses it.
use reagent_engine as _;

fn main() {
    eprintln!("reagent-cli has no commands yet");
}
