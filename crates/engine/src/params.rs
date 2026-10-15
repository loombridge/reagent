//! Reagent's parameter space: every parameter of every module, with an ID
//! that never changes and its real name.
//!
//! IDs follow the plug-in kit's scheme (`lspk_params::space`): a module
//! type, its instance and the parameter's number in the module. Module
//! types and parameter numbers here are permanent: a new parameter takes the
//! next free number in its module, a new module the next free type, and a
//! new choice goes at the end of its list. `crates/plugin/tests/ids.txt`
//! keeps every published ID, and the test that reads it fails if one
//! changes meaning.
//!
//! Values are in real units, as the engine specification gives them
//! (engine-core sections 1 to 15): levels in per cent with gain n²
//! (engine-core R7), gains as linear factors read in decibels, times in
//! milliseconds following `32 s × n⁵` (R70), cutoff in hertz on an
//! exponential travel (R51).

use lspk_params::{
    ModuleType, ParamAddress, ParamDescriptor, ParamId, ParamSpace, ParamUnit, gain_to_db,
};

/// The main oscillators' module type: `Osc A`, `Osc B`, `Osc C`.
pub const OSC: u8 = 1;
/// The sub oscillator's module type: `Osc Sub`.
pub const SUB: u8 = 2;
/// The noise oscillator's module type: `Osc Noise`.
pub const NOISE: u8 = 3;
/// The filters' module type: `Filter 1`, `Filter 2`.
pub const FILTER: u8 = 4;
/// The buses' module type: `Bus 1`, `Bus 2`.
pub const BUS: u8 = 5;
/// The amplifier envelope's module type: `Env 1`.
pub const AMP_ENV: u8 = 6;
/// The modulation envelopes' module type: `Env 2` to `Env 4`.
pub const MOD_ENV: u8 = 7;
/// The LFOs' module type: `LFO 1` to `LFO 10`.
pub const LFO: u8 = 8;
/// The module type the modulation matrix's slots are addressed by as
/// destinations (a slot's amount and output). These are modulation targets
/// only, never host parameters.
pub const MATRIX: u8 = 120;

/// How many main oscillators.
pub const OSCILLATORS: usize = 3;
/// How many filters.
pub const FILTERS: usize = 2;
/// How many buses.
pub const BUSES: usize = 2;
/// How many envelopes, Env 1 first.
pub const ENVELOPES: usize = 4;
/// How many LFOs (engine-core R81).
pub const LFOS: usize = 10;
/// How many macros (engine-core section 13).
pub const MACROS: usize = 8;

/// The parameters every mixer channel (each oscillator, the sub and the
/// noise) has, numbered the same in each of their modules.
pub mod channel {
    /// On or off.
    pub const ON: u16 = 0;
    /// Level, 0 to 100 %, as gain n² (engine-core R7).
    pub const LEVEL: u16 = 1;
    /// Pan, −50 to +50.
    pub const PAN: u16 = 2;
    /// Output: Filter, Main, Dry out or None (R8).
    pub const OUTPUT: u16 = 3;
    /// Filter balance, −100 (Filter 1) to +100 (Filter 2).
    pub const BALANCE: u16 = 4;
    /// Send 1, 0 to 100 %, after Level (R11).
    pub const SEND_1: u16 = 5;
    /// Send 2.
    pub const SEND_2: u16 = 6;
    /// Whether Env 1 scales the channel on Main and Dry out (R10).
    pub const AMP_ENV: u16 = 7;
}

/// A main oscillator's own parameters, after the channel's.
pub mod osc {
    /// Octave, −4 to +4 (engine-core R1).
    pub const OCTAVE: u16 = 16;
    /// Semitone, −12 to +12.
    pub const SEMITONE: u16 = 17;
    /// Fine, −100 to +100 cents.
    pub const FINE: u16 = 18;
    /// Sweep, −64 to +64 semitones, continuous.
    pub const SWEEP: u16 = 19;
    /// Key tracking (R5).
    pub const KEY_TRACK: u16 = 20;
    /// Frame, 0 to 100 % of the table (R14).
    pub const FRAME: u16 = 21;
    /// Smooth frames (R14).
    pub const SMOOTH: u16 = 22;
    /// Phase, 0 to 360 degrees (R16).
    pub const PHASE: u16 = 23;
    /// Phase random, 0 to 100 % (R17).
    pub const PHASE_RANDOM: u16 = 24;
    /// Phase mode: Note start, Continue or Per voice (R19).
    pub const PHASE_MODE: u16 = 25;
    /// Phase legato (R18).
    pub const PHASE_LEGATO: u16 = 26;
    /// Unison: copies per voice, 1 to 16 (R32).
    pub const UNISON: u16 = 27;
    /// Detune, 0 to 1, shown as the square of the knob (R33).
    pub const DETUNE: u16 = 28;
    /// Unison mix, 0 to 100 %: the outer copies against the centre (R35).
    pub const UNISON_MIX: u16 = 29;
    /// Detune law: Linear, Dense, Exponential, Inverse, Random (R34).
    pub const DETUNE_LAW: u16 = 30;
    /// Stack (R36).
    pub const STACK: u16 = 31;
    /// Width, −100 to +100 % (R37).
    pub const WIDTH: u16 = 32;
    /// Detune range, 0 to 48 semitones (R33).
    pub const DETUNE_RANGE: u16 = 33;
    /// Frame spread, −100 to +100 (R38).
    pub const FRAME_SPREAD: u16 = 34;
}

/// The sub oscillator's own parameters, after the channel's (engine-core
/// section 5).
pub mod sub {
    /// Octave, −4 to +4 (R40).
    pub const OCTAVE: u16 = 16;
    /// Sweep, −64 to +64 semitones, continuous (R40).
    pub const SWEEP: u16 = 17;
    /// Shape: Sine, Rounded square, Triangle, Saw, Square, Pulse (R40).
    pub const SHAPE: u16 = 18;
    /// Phase, 0 to 360 degrees: where a note starts (R41).
    pub const PHASE: u16 = 19;
    /// Continue phase: a note carries on from the last one's phase (R41).
    pub const CONTINUE_PHASE: u16 = 20;
    /// Wrap phase: phase modulated past 360° wraps (R42).
    pub const WRAP_PHASE: u16 = 21;
    /// Key tracking; off plays MIDI note 60 (R5).
    pub const KEY_TRACK: u16 = 22;
}

/// The noise oscillator's own parameters, after the channel's (engine-core
/// section 6).
pub mod noise {
    /// Source: the colour noise it plays (R49).
    pub const SOURCE: u16 = 16;
    /// Loop: one-shot stops at the end of the block, Loop wraps (R48).
    pub const LOOP: u16 = 17;
    /// Start, 0 to 100 % of the block (R47).
    pub const START: u16 = 18;
    /// Start random, 0 to 100 %: spread each note's start (R47).
    pub const START_RANDOM: u16 = 19;
    /// Rate, 0 to 100 %, 50 % the block's original speed (R46).
    pub const RATE: u16 = 20;
    /// Fine, −1 to +1: a fine detune of the playback speed (R46).
    pub const FINE: u16 = 21;
    /// Key tracking: the pitch follows the note (R46).
    pub const KEY_TRACK: u16 = 22;
    /// Stereo, 0 to 100 %, for the colour noises (R49).
    pub const STEREO: u16 = 23;
    /// Tone, low-passed at 0, flat at 50, high-passed at 100 (R49).
    pub const TONE: u16 = 24;
}

/// A filter's parameters (engine-core sections 7 and 8).
pub mod filter {
    /// On or off; a filter that's off passes its input on (R9).
    pub const ON: u16 = 0;
    /// Type: the filter types built so far ([`FILTER_TYPES`](super::FILTER_TYPES)).
    pub const TYPE: u16 = 1;
    /// Cutoff, 8.18 Hz to 22.05 kHz (R51).
    pub const CUTOFF: u16 = 2;
    /// Resonance, 0 to 100 %.
    pub const RESONANCE: u16 = 3;
    /// Drive, 0 to 100 % (R53).
    pub const DRIVE: u16 = 4;
    /// Clean drive (R53).
    pub const CLEAN_DRIVE: u16 = 5;
    /// Character, 0 to 100 % (R54).
    pub const CHARACTER: u16 = 6;
    /// Stereo offset, 0 to 100, 50 for none (R55).
    pub const STEREO: u16 = 7;
    /// Mix, 0 to 100 % (R56).
    pub const MIX: u16 = 8;
    /// Level, −∞ to +12 dB (R57).
    pub const LEVEL: u16 = 9;
    /// Key tracking (R52).
    pub const KEY_TRACK: u16 = 10;
    /// Output: the other filter, Main, Dry out or None (R65).
    pub const OUTPUT: u16 = 11;
    /// Send 1, 0 to 100 %.
    pub const SEND_1: u16 = 12;
    /// Send 2.
    pub const SEND_2: u16 = 13;
    /// Whether Env 1 scales the filter's output (on in a new patch).
    pub const AMP_ENV: u16 = 14;
    /// The pole-zero filter's X, 0 to 1.
    pub const MORPH_X: u16 = 15;
    /// The pole-zero filter's Y, 0 to 1.
    pub const MORPH_Y: u16 = 16;
}

/// A bus's parameters (engine-core section 8).
pub mod bus {
    /// Level, −∞ to +12 dB.
    pub const LEVEL: u16 = 0;
    /// Output: Main, Dry out or the other bus (R66, R68).
    pub const OUTPUT: u16 = 1;
}

/// An envelope's parameters (engine-core section 9). Env 1 has no start
/// and end levels.
pub mod env {
    /// Attack, 0 to 32 s (R70).
    pub const ATTACK: u16 = 0;
    /// Hold.
    pub const HOLD: u16 = 1;
    /// Decay.
    pub const DECAY: u16 = 2;
    /// Sustain: Env 1 a gain to 0 dB, Env 2 to 4 per cent of full (R74).
    pub const SUSTAIN: u16 = 3;
    /// Release.
    pub const RELEASE: u16 = 4;
    /// Attack curve, 0 to 100 %, 50 % straight (R73).
    pub const ATTACK_CURVE: u16 = 5;
    /// Decay curve.
    pub const DECAY_CURVE: u16 = 6;
    /// Release curve.
    pub const RELEASE_CURVE: u16 = 7;
    /// Legato inverted (R76).
    pub const LEGATO_INVERTED: u16 = 8;
    /// Steal restart: from the current level or the start level (R79).
    pub const STEAL_RESTART: u16 = 9;
    /// Start level (Env 2 to 4), −∞ to 0 dB (R71).
    pub const START: u16 = 10;
    /// End level (Env 2 to 4).
    pub const END: u16 = 11;
}

/// An LFO's parameters (engine-core section 10).
pub mod lfo {
    /// Type: Drawn, XY path, Lorenz, Rössler, Sample and hold.
    pub const TYPE: u16 = 0;
    /// Mode: Free, Restart, One-shot (R83).
    pub const MODE: u16 = 1;
    /// Shared: one LFO for every voice (R82).
    pub const SHARED: u16 = 2;
    /// Direction: Forward, Reverse, Ping-pong (R91).
    pub const DIRECTION: u16 = 3;
    /// Sync to the host's tempo.
    pub const SYNC: u16 = 4;
    /// The synced rate's division (R85).
    pub const DIVISION: u16 = 5;
    /// The rate in hertz, when not synced (R86).
    pub const RATE: u16 = 6;
    /// Rate ×10 (R86).
    pub const X10: u16 = 7;
    /// Straight, triplet or dotted (R85).
    pub const FEEL: u16 = 8;
    /// Song lock (R84).
    pub const SONG_LOCK: u16 = 9;
    /// Swing (R87).
    pub const SWING: u16 = 10;
    /// Fade in, off or a division (R88).
    pub const FADE_IN: u16 = 11;
    /// Delay, off or a division (R88).
    pub const DELAY: u16 = 12;
    /// Smooth, 0 to 100 (R89).
    pub const SMOOTH: u16 = 13;
    /// Phase, 0 to 360 degrees (R90).
    pub const PHASE: u16 = 14;
    /// Phase snap (R90).
    pub const PHASE_SNAP: u16 = 15;
}

/// The global parameters, numbered in the flat space below 2²⁴.
pub mod global {
    use lspk_params::ParamId;

    /// Macro `n` (0 to 7), 0 to 100 % (engine-core section 13).
    #[must_use]
    pub const fn macro_id(n: usize) -> ParamId {
        ParamId(1 + n as u32)
    }
    /// Main level, −∞ to +3 dB (R66).
    pub const MAIN_LEVEL: ParamId = ParamId(16);
    /// Dry out level, −∞ to +12 dB.
    pub const DRY_LEVEL: ParamId = ParamId(17);
    /// Amp: each voice's gain, −∞ to +6 dB, unity by default.
    pub const AMP: ParamId = ParamId(18);
    /// Main tuning, −64 to +64 semitones (R137).
    pub const TUNING: ParamId = ParamId(19);
    /// Quality: oversampling of the voice path (R132).
    pub const QUALITY: ParamId = ParamId(20);
    /// Voices, 1 to 32 (R117).
    pub const VOICES: ParamId = ParamId(21);
    /// Mono (R118).
    pub const MONO: ParamId = ParamId(22);
    /// Legato (R118, R119).
    pub const LEGATO: ParamId = ParamId(23);
    /// Steal priority (R122).
    pub const STEAL: ParamId = ParamId(24);
    /// One voice per key: Off, Sustain, On (R120).
    pub const SAME_NOTE: ParamId = ParamId(25);
    /// Glide time, 0 to 8 s (R123).
    pub const GLIDE_TIME: ParamId = ParamId(26);
    /// Glide curve, −100 to +100 (R123).
    pub const GLIDE_CURVE: ParamId = ParamId(27);
    /// Glide always (R124).
    pub const GLIDE_ALWAYS: ParamId = ParamId(28);
    /// Glide by interval (R125).
    pub const GLIDE_SCALED: ParamId = ParamId(29);
    /// Bend up, −24 to +24 semitones (R127).
    pub const BEND_UP: ParamId = ParamId(30);
    /// Bend down, −24 to +24 semitones (R127).
    pub const BEND_DOWN: ParamId = ParamId(31);
    /// MPE (R128).
    pub const MPE: ParamId = ParamId(32);
    /// MPE bend range, 1 to 96 semitones (R128).
    pub const MPE_BEND: ParamId = ParamId(33);
    /// Velocity legato (R101).
    pub const VELOCITY_LEGATO: ParamId = ParamId(34);
    /// Curve glide: whether a legato note moves the velocity and key curves
    /// to its values over the glide time (R100).
    pub const CURVE_GLIDE: ParamId = ParamId(38);
    /// Reference pitch: frequency of A4, 430 to 450 Hz in whole hertz (R136).
    pub const REFERENCE_PITCH: ParamId = ParamId(35);
    /// Envelope scale: multiplies every envelope's times, 10 % to 1000 % (R80).
    pub const ENV_SCALE: ParamId = ParamId(36);
    /// LFO scale: scales every LFO, 10 % to 1000 % (R97).
    pub const LFO_SCALE: ParamId = ParamId(37);
}

/// The oscillators' instance names.
pub const OSC_NAMES: [&str; OSCILLATORS] = ["A", "B", "C"];

/// The filter types, in the order of the Type parameter's choices: the
/// reference's Normal group in its order (the ladder first), then the
/// Multi, Flanges, Misc and Extended groups, 108 in all, as the reference
/// lists them (engine-core section 7). Later types go at the end.
pub const FILTER_TYPES: [&str; 108] = [
    "Ladder LP 6",
    "Ladder LP 12",
    "Ladder LP 18",
    "Ladder LP 24",
    "SVF LP 6",
    "SVF LP 12",
    "SVF LP 18",
    "SVF LP 24",
    "SVF HP 6",
    "SVF HP 12",
    "SVF HP 18",
    "SVF HP 24",
    "SVF BP 12",
    "SVF BP 24",
    "Peak 12",
    "Peak 24",
    "Notch 12",
    "Notch 24",
    "Dual LH 6",
    "Dual LH 12",
    "Dual LB 12",
    "Dual LP 12",
    "Dual LN 12",
    "Dual HB 12",
    "Dual HP 12",
    "Dual HN 12",
    "Dual BP 12",
    "Dual BN 12",
    "Dual PP 12",
    "Dual PN 12",
    "Dual NN 12",
    "Morph L/B/H 12",
    "Morph L/B/H 24",
    "Morph L/P/H 12",
    "Morph L/P/H 24",
    "Morph L/N/H 12",
    "Morph L/N/H 24",
    "Morph B/P/N 12",
    "Morph B/P/N 24",
    "Comb +",
    "Comb -",
    "Comb LP +",
    "Comb LP -",
    "Comb HP +",
    "Comb HP -",
    "Comb band +",
    "Comb band -",
    "Flanger +",
    "Flanger -",
    "Flanger LP +",
    "Flanger LP -",
    "Flanger HP +",
    "Flanger HP -",
    "Flanger band +",
    "Flanger band -",
    "Phaser 12 +",
    "Phaser 12 -",
    "Phaser 24 +",
    "Phaser 24 -",
    "Phaser 36 +",
    "Phaser 36 -",
    "Phaser 48 +",
    "Phaser 48 -",
    "Phaser 48 LP +",
    "Phaser 48 LP -",
    "Phaser 48 HP +",
    "Phaser 48 HP -",
    "Phaser 48 band +",
    "Phaser 48 band -",
    "Flange phaser 12 band +",
    "Flange phaser 12 band -",
    "Low EQ 6",
    "Low EQ 12",
    "Band EQ 12",
    "High EQ 6",
    "High EQ 12",
    "Ring",
    "Ring x2",
    "Sample and hold",
    "S&H difference",
    "Combs",
    "All-passes",
    "Room",
    "Two-resonance LP",
    "Zero-delay LP",
    "Bass add",
    "Vowel 1",
    "Vowel 2",
    "Vowel 3",
    "Band reject",
    "Comb-feedback LP 1",
    "Comb-feedback BP 1",
    "Comb-feedback LP 2",
    "Comb-feedback BP 2",
    "Shriek LP",
    "Shriek BP",
    "Buzz morph",
    "DJ sweep",
    "Diffuser",
    "Transistor ladder",
    "Diode ladder",
    "Clean diode ladder",
    "Driven ladder",
    "Pole-zero",
    "Comb II",
    "Expander multimode",
    "Expander BP",
    "K35",
];

/// A new patch's filter type: the 12 dB ladder (observed default; the
/// manual's 6 dB contradicted, engine-core section 7).
pub const DEFAULT_FILTER_TYPE: usize = 1;

/// A channel's outputs, by the index its Output parameter stores (R8).
pub const CHANNEL_OUTPUTS: [&str; 4] = ["Filter", "Main", "Dry out", "None"];
/// A filter's outputs (R65).
pub const FILTER_OUTPUTS: [&str; 4] = ["Other filter", "Main", "Dry out", "None"];
/// A bus's outputs (R66).
pub const BUS_OUTPUTS: [&str; 3] = ["Main", "Dry out", "Other bus"];
/// The quality settings, by oversampling factor 1, 2 and 4 (R132).
pub const QUALITIES: [&str; 3] = ["Good", "High", "Ultra"];
/// A new patch's quality: High (observed, engine-core section 15).
pub const DEFAULT_QUALITY: usize = 1;
/// The sub's shapes, in the order the Shape control lists them (R40).
pub const SUB_SHAPES: [&str; 6] = [
    "Sine",
    "Rounded square",
    "Triangle",
    "Saw",
    "Square",
    "Pulse",
];
/// The colour noises the noise oscillator can play, in the order the
/// Source control lists them (R49).
pub const NOISE_SOURCES: [&str; 4] = ["White", "Pink", "Brown", "Crackle"];
/// A new patch's noise source: White (Reagent's own library, R49).
pub const DEFAULT_NOISE_SOURCE: usize = 0;

/// The synced LFO divisions, by the index the kit's `Division` takes.
pub const DIVISIONS: [&str; 15] = [
    "32 bars", "16 bars", "8 bars", "4 bars", "2 bars", "1 bar", "1/2", "1/4", "1/8", "1/16",
    "1/32", "1/64", "1/128", "1/256", "Fast",
];
/// Fade in and Delay, by the index the kit's `Span::from_index` takes.
pub const SPANS: [&str; 15] = [
    "Off", "1/256", "1/128", "1/64", "1/32", "1/16", "1/8", "1/4", "1/2", "1 bar", "2 bars",
    "4 bars", "8 bars", "16 bars", "32 bars",
];

/// The lowest cutoff: MIDI note 0 at A4 = 440 Hz (R51).
pub const CUTOFF_MIN: f64 = 8.175_798_915_643_707;
/// The highest cutoff the control shows (R51).
pub const CUTOFF_MAX: f64 = 22_050.0;
/// The longest envelope stage, in milliseconds (R70).
pub const ENV_MAX_MS: f64 = 32_000.0;
/// The longest glide, in milliseconds (R123).
pub const GLIDE_MAX_MS: f64 = 8_000.0;

/// Parameter `param` of instance `instance` of module type `module`.
#[must_use]
pub const fn id(module: u8, instance: u8, param: u16) -> ParamId {
    ParamAddress::new(module, instance, param).id()
}

fn n(number: u16) -> ParamId {
    ParamId(u32::from(number))
}

fn percent(number: u16, name: &str, default: f64, text: &str) -> ParamDescriptor {
    ParamDescriptor::continuous(n(number), name, (0.0, 100.0), default, ParamUnit::Percent)
        .describe(text)
}

fn toggle(number: u16, name: &str, on: bool, text: &str) -> ParamDescriptor {
    ParamDescriptor::toggle(n(number), name, on).describe(text)
}

fn choice(
    number: u16,
    name: &str,
    choices: &[&str],
    default: usize,
    text: &str,
) -> ParamDescriptor {
    ParamDescriptor::choice(n(number), name, choices, default).describe(text)
}

fn time(number: u16, name: &str, default: f64, text: &str) -> ParamDescriptor {
    ParamDescriptor::continuous(
        n(number),
        name,
        (0.0, ENV_MAX_MS),
        default,
        ParamUnit::Milliseconds,
    )
    .power(5)
    .describe(text)
}

/// The parameters every mixer channel has.
fn channel_params(kind: ModuleType, level: f64, what: &str) -> ModuleType {
    use channel::*;
    kind.param(toggle(ON, "On", false, &format!("Whether {what} plays.")))
        .param(percent(
            LEVEL,
            "Level",
            level,
            &format!(
                "How loud {what} plays: the gain is the square of the level, so 75 % is −5 dB."
            ),
        ))
        .param(
            ParamDescriptor::continuous(n(PAN), "Pan", (-50.0, 50.0), 0.0, ParamUnit::None)
                .describe(&format!(
                    "Where {what} sits between left (−50) and right (+50)."
                )),
        )
        .param(choice(
            OUTPUT,
            "Output",
            &CHANNEL_OUTPUTS,
            1,
            &format!(
                "Where {what} goes: through the filters (split by Filter balance), straight to \
                 Main, to the dry output that skips every effect, or nowhere (it still runs as a \
                 modulation source)."
            ),
        ))
        .param(
            ParamDescriptor::continuous(
                n(BALANCE),
                "Filter balance",
                (-100.0, 100.0),
                -100.0,
                ParamUnit::None,
            )
            .describe(&format!(
                "How {what} is split between the filters when its output is Filter: −100 all to \
                 Filter 1, +100 all to Filter 2."
            )),
        )
        .param(percent(
            SEND_1,
            "Send 1",
            0.0,
            &format!("How much of {what}, after its level, goes to Bus 1; 100 % is unity gain."),
        ))
        .param(percent(
            SEND_2,
            "Send 2",
            0.0,
            &format!("How much of {what}, after its level, goes to Bus 2; 100 % is unity gain."),
        ))
        .param(toggle(
            AMP_ENV,
            "Amp envelope",
            true,
            &format!(
                "Whether Env 1 shapes {what} on Main, Dry out and the sends. Off, {what} sounds \
                 until every envelope has finished its release."
            ),
        ))
}

/// The main oscillator module.
#[must_use]
pub fn oscillator() -> ModuleType {
    use osc::*;
    channel_params(ModuleType::new(OSC, "osc", "Osc"), 75.0, "the oscillator")
        .param(
            ParamDescriptor::integer(n(OCTAVE), "Octave", (-4, 4), 0, ParamUnit::None)
                .describe("Transposes the oscillator by whole octaves."),
        )
        .param(
            ParamDescriptor::integer(n(SEMITONE), "Semitone", (-12, 12), 0, ParamUnit::Semitones)
                .describe("Transposes the oscillator by whole semitones."),
        )
        .param(
            ParamDescriptor::continuous(n(FINE), "Fine", (-100.0, 100.0), 0.0, ParamUnit::Cents)
                .describe("Tunes the oscillator in cents."),
        )
        .param(
            ParamDescriptor::continuous(
                n(SWEEP),
                "Sweep",
                (-64.0, 64.0),
                0.0,
                ParamUnit::Semitones,
            )
            .describe(
                "Transposes the oscillator continuously, up to 64 semitones either way: the \
                 control to modulate for wide pitch sweeps.",
            ),
        )
        .param(toggle(
            KEY_TRACK,
            "Key track",
            true,
            "Whether the oscillator follows the played note. Off, it plays MIDI note 0 \
             (8.18 Hz), still following pitch bend and modulation.",
        ))
        .param(percent(
            FRAME,
            "Frame",
            0.0,
            "Which frame of the wavetable plays, from the first (0 %) to the last (100 %).",
        ))
        .param(toggle(
            SMOOTH,
            "Smooth frames",
            false,
            "Whether positions between frames blend the neighbouring frames, rather than \
             playing the nearest.",
        ))
        .param(
            ParamDescriptor::continuous(n(PHASE), "Phase", (0.0, 360.0), 180.0, ParamUnit::None)
                .describe("Where in the frame a note starts, in degrees."),
        )
        .param(percent(
            PHASE_RANDOM,
            "Phase random",
            100.0,
            "How far each new note's start phase is randomised; 100 % is fully random.",
        ))
        .param(choice(
            PHASE_MODE,
            "Phase mode",
            &["Note start", "Continue", "Per voice"],
            0,
            "Where a note's phase starts: from Phase each note, carrying on from the last note, \
             or from each voice's stored phase.",
        ))
        .param(toggle(
            PHASE_LEGATO,
            "Phase legato",
            false,
            "Inverts which notes restart the phase: stolen voices restart, legato notes don't.",
        ))
        .param(
            ParamDescriptor::integer(n(UNISON), "Unison", (1, 16), 1, ParamUnit::None).describe(
                "How many detuned copies of the oscillator each note plays; copies don't count \
                 against Voices.",
            ),
        )
        .param(
            ParamDescriptor::continuous(n(DETUNE), "Detune", (0.0, 1.0), 0.25, ParamUnit::None)
                .power(2)
                .describe(
                    "How far the unison copies spread in pitch: at 1 the outer copies sit a full \
                     Detune range either side of the note.",
                ),
        )
        .param(percent(
            UNISON_MIX,
            "Unison mix",
            75.0,
            "The outer copies' level against the centre's; at 75 % every copy plays equally.",
        ))
        .param(choice(
            DETUNE_LAW,
            "Detune law",
            &["Linear", "Dense", "Exponential", "Inverse", "Random"],
            0,
            "How the copies spread across the detune: evenly, slightly unevenly, packed near \
             the centre, pushed outward, or at random each note.",
        ))
        .param(choice(
            STACK,
            "Stack",
            &[
                "Off",
                "Octave x1",
                "Octave x2",
                "Octave x3",
                "Octave and fifth x1",
                "Octave and fifth x2",
                "Octave and fifth x3",
                "Centre -12",
                "Centre -24",
            ],
            0,
            "Stacks the copies at octaves (and fifths) above the note, or drops the centre copy \
             an octave or two.",
        ))
        .param(
            ParamDescriptor::continuous(
                n(WIDTH),
                "Width",
                (-100.0, 100.0),
                100.0,
                ParamUnit::Percent,
            )
            .describe("How far the copies spread across the stereo field; negative mirrors them."),
        )
        .param(
            ParamDescriptor::continuous(
                n(DETUNE_RANGE),
                "Detune range",
                (0.0, 48.0),
                2.0,
                ParamUnit::Semitones,
            )
            .describe("The outer copies' detune at full Detune."),
        )
        .param(
            ParamDescriptor::continuous(
                n(FRAME_SPREAD),
                "Frame spread",
                (-100.0, 100.0),
                0.0,
                ParamUnit::None,
            )
            .describe("Spreads the copies' frames around Frame, so they play different frames."),
        )
}

/// The sub oscillator's module (engine-core section 5).
#[must_use]
pub fn sub() -> ModuleType {
    use sub::*;
    channel_params(
        ModuleType::new(SUB, "sub", "Osc"),
        75.0,
        "the sub oscillator",
    )
    .param(
        ParamDescriptor::integer(n(OCTAVE), "Octave", (-4, 4), 0, ParamUnit::None)
            .describe("Transposes the sub by whole octaves."),
    )
    .param(
        ParamDescriptor::continuous(n(SWEEP), "Sweep", (-64.0, 64.0), 0.0, ParamUnit::Semitones)
            .describe("Transposes the sub continuously, up to 64 semitones either way."),
    )
    .param(choice(
        SHAPE,
        "Shape",
        &SUB_SHAPES,
        0,
        "The sub's fixed waveform.",
    ))
    .param(
        ParamDescriptor::continuous(n(PHASE), "Phase", (0.0, 360.0), 0.0, ParamUnit::None)
            .describe("Where in the shape a note starts, in degrees."),
    )
    .param(toggle(
        CONTINUE_PHASE,
        "Continue phase",
        false,
        "Whether each note carries on from the last note's phase instead of starting at Phase.",
    ))
    .param(toggle(
        WRAP_PHASE,
        "Wrap phase",
        false,
        "Whether phase modulated past 360° wraps around; off, it stays at 360°.",
    ))
    .param(toggle(
        KEY_TRACK,
        "Key track",
        true,
        "Whether the sub follows the played note. Off, it plays MIDI note 60 (261.63 Hz).",
    ))
}

/// The noise oscillator's module (engine-core section 6).
#[must_use]
pub fn noise() -> ModuleType {
    use noise::*;
    channel_params(ModuleType::new(NOISE, "noise", "Osc"), 25.0, "the noise")
        .param(choice(
            SOURCE,
            "Source",
            &NOISE_SOURCES,
            DEFAULT_NOISE_SOURCE,
            "Which noise plays: Reagent's own White, Pink, Brown or Crackle.",
        ))
        .param(choice(
            LOOP,
            "Loop",
            &["One-shot", "Loop"],
            1,
            "Whether playback wraps at the end of the noise or stops there.",
        ))
        .param(percent(
            START,
            "Start",
            0.0,
            "Where playback starts in the noise.",
        ))
        .param(percent(
            START_RANDOM,
            "Start random",
            0.0,
            "How far each note's start is spread, so chords don't repeat the same noise.",
        ))
        .param(percent(
            RATE,
            "Rate",
            50.0,
            "The playback speed; 50 % is the noise's original speed.",
        ))
        .param(
            ParamDescriptor::continuous(n(FINE), "Fine", (-1.0, 1.0), 0.0, ParamUnit::None)
                .describe("Detunes the playback speed finely, in semitones."),
        )
        .param(toggle(
            KEY_TRACK,
            "Key track",
            false,
            "Whether the playback speed follows the played note.",
        ))
        .param(percent(
            STEREO,
            "Stereo",
            0.0,
            "For the colour noises: mono at 0 %, two uncorrelated channels at 100 %.",
        ))
        .param(percent(
            TONE,
            "Tone",
            50.0,
            "For the colour noises: low-passed at 0 %, flat at 50 %, high-passed at 100 %.",
        ))
}

/// The filter module.
#[must_use]
pub fn filter_module() -> ModuleType {
    use filter::*;
    let middle = (CUTOFF_MIN * CUTOFF_MAX).sqrt();
    ModuleType::new(FILTER, "filter", "Filter")
        .param(toggle(
            ON,
            "On",
            false,
            "Whether the filter processes. Off, it passes its input on to its output unchanged.",
        ))
        .param(choice(
            TYPE,
            "Type",
            &FILTER_TYPES,
            DEFAULT_FILTER_TYPE,
            "The filter's response and slope.",
        ))
        .param(
            ParamDescriptor::continuous(
                n(CUTOFF),
                "Cutoff",
                (CUTOFF_MIN, CUTOFF_MAX),
                middle,
                ParamUnit::Hertz,
            )
            .logarithmic()
            .describe("The cutoff frequency; the control moves evenly in pitch."),
        )
        .param(percent(
            RESONANCE,
            "Resonance",
            10.0,
            "Emphasis at the cutoff; near the top, types that can self-oscillate do.",
        ))
        .param(percent(
            DRIVE,
            "Drive",
            0.0,
            "Gain into the filter, with mild saturation.",
        ))
        .param(toggle(
            CLEAN_DRIVE,
            "Clean drive",
            false,
            "Drives the filter 24 dB lower and makes it up after, for less saturation.",
        ))
        .param(percent(
            CHARACTER,
            "Character",
            0.0,
            "Each type's extra control: for the ladder and state-variable types, saturation in \
             the resonance path; for the others their second cutoff, morph, feedback filter, \
             gain or spacing.",
        ))
        .param(
            ParamDescriptor::continuous(n(STEREO), "Stereo", (0.0, 100.0), 50.0, ParamUnit::None)
                .describe(
                    "Moves the channels' cutoffs apart: 50 does nothing, lower raises the left \
                     and lowers the right, higher the reverse, by up to 12 semitones.",
                ),
        )
        .param(percent(
            MIX,
            "Mix",
            100.0,
            "The filtered signal against the dry; 100 % is fully filtered.",
        ))
        .param(
            ParamDescriptor::gain(n(LEVEL), "Level", gain_to_db(4.0), 0.0)
                .power(2)
                .describe("The filter's output gain."),
        )
        .param(toggle(
            KEY_TRACK,
            "Key track",
            false,
            "Whether the cutoff follows the played note, an octave per octave from middle C.",
        ))
        .param(choice(
            OUTPUT,
            "Output",
            &FILTER_OUTPUTS,
            1,
            "Where the filter's output goes: into the other filter (the two in series), Main, \
             the dry output, or nowhere. Routing a filter into one that already routes back \
             sets the other to Main.",
        ))
        .param(percent(
            SEND_1,
            "Send 1",
            0.0,
            "How much of the filter's output goes to Bus 1.",
        ))
        .param(percent(
            SEND_2,
            "Send 2",
            0.0,
            "How much of the filter's output goes to Bus 2.",
        ))
        .param(toggle(
            AMP_ENV,
            "Amp envelope",
            true,
            "Whether Env 1 shapes the filter's output.",
        ))
        .param(
            ParamDescriptor::continuous(
                n(MORPH_X),
                "Pole-zero X",
                (0.0, 1.0),
                0.0,
                ParamUnit::None,
            )
            .describe("The pole-zero filter's X: morphs between its four states, with Y."),
        )
        .param(
            ParamDescriptor::continuous(
                n(MORPH_Y),
                "Pole-zero Y",
                (0.0, 1.0),
                0.0,
                ParamUnit::None,
            )
            .describe("The pole-zero filter's Y: morphs between its four states, with X."),
        )
}

/// The bus module.
#[must_use]
pub fn bus_module() -> ModuleType {
    ModuleType::new(BUS, "bus", "Bus")
        .param(
            ParamDescriptor::gain(n(bus::LEVEL), "Level", gain_to_db(4.0), 0.0)
                .power(2)
                .describe("The bus's level, after everything sent to it."),
        )
        .param(choice(
            bus::OUTPUT,
            "Output",
            &BUS_OUTPUTS,
            0,
            "Where the bus goes: Main, the dry output, or the other bus. Routing a bus into one \
             that already routes back sets the other to Main.",
        ))
}

fn envelope(kind: ModuleType, amp: bool) -> ModuleType {
    use env::*;
    let attack = if amp { 1.0 } else { 0.5 };
    let kind = kind
        .param(time(
            ATTACK,
            "Attack",
            attack,
            "How long the envelope takes to rise to full at a note's start.",
        ))
        .param(time(HOLD, "Hold", 0.0, "How long it holds at full."))
        .param(time(
            DECAY,
            "Decay",
            1000.0,
            "How long it takes to fall from full to the sustain level.",
        ));
    let kind = if amp {
        kind.param(
            ParamDescriptor::gain(n(SUSTAIN), "Sustain", 0.0, 0.0)
                .describe("The level held while the note lasts."),
        )
    } else {
        kind.param(
            ParamDescriptor::continuous(
                n(SUSTAIN),
                "Sustain",
                (0.0, 100.0),
                100.0,
                ParamUnit::Percent,
            )
            .power(2)
            .describe("The level held while the note lasts, as a percentage of full."),
        )
    };
    let kind = kind
        .param(time(
            RELEASE,
            "Release",
            15.0,
            "How long it takes to fall to its end level once the note ends.",
        ))
        .param(percent(
            ATTACK_CURVE,
            "Attack curve",
            50.0,
            "The attack's shape: 50 % straight, higher fast first and slow into full.",
        ))
        .param(percent(
            DECAY_CURVE,
            "Decay curve",
            66.6,
            "The decay's shape: 50 % straight, higher fast first and slow into the sustain level.",
        ))
        .param(percent(
            RELEASE_CURVE,
            "Release curve",
            66.6,
            "The release's shape: 50 % straight, higher fast first and slow into the end level.",
        ))
        .param(toggle(
            LEGATO_INVERTED,
            "Legato inverted",
            false,
            "Restarts the envelope on every note, even when the patch plays legato.",
        ))
        .param(choice(
            STEAL_RESTART,
            "Steal restart",
            &["From current level", "From start level"],
            usize::from(!amp),
            "Where the envelope restarts when its voice, already sounding, takes a new note.",
        ));
    if amp {
        kind
    } else {
        kind.param(
            ParamDescriptor::gain(n(START), "Start level", 0.0, f64::NEG_INFINITY)
                .describe("The level a note starts from."),
        )
        .param(
            ParamDescriptor::gain(n(END), "End level", 0.0, f64::NEG_INFINITY)
                .describe("The level the release ends at."),
        )
    }
}

/// The LFO module.
#[must_use]
pub fn lfo_module() -> ModuleType {
    use lfo::*;
    ModuleType::new(LFO, "lfo", "LFO")
        .param(choice(
            TYPE,
            "Type",
            &["Drawn", "XY path", "Lorenz", "Rössler", "Sample and hold"],
            0,
            "What the LFO makes: its drawn shape, an XY path, a chaotic attractor, or a new \
             random value each cycle.",
        ))
        .param(choice(
            MODE,
            "Mode",
            &["Free", "Restart", "One-shot"],
            1,
            "How notes start it: Free ignores notes, Restart starts again at each note, \
             One-shot plays one cycle and stops.",
        ))
        .param(toggle(
            SHARED,
            "Shared",
            false,
            "One LFO for every voice, instead of one per voice.",
        ))
        .param(choice(
            DIRECTION,
            "Direction",
            &["Forward", "Reverse", "Ping-pong"],
            0,
            "Which way the shape plays.",
        ))
        .param(toggle(
            SYNC,
            "Sync",
            true,
            "Whether the rate follows the host's tempo (Division) or is in hertz (Rate).",
        ))
        .param(choice(
            DIVISION,
            "Division",
            &DIVISIONS,
            7,
            "The cycle's length when synced to the tempo.",
        ))
        .param(
            ParamDescriptor::continuous(n(RATE), "Rate", (0.0, 100.0), 1.0, ParamUnit::Hertz)
                .power(4)
                .describe("Cycles per second when not synced; ten times faster with Rate ×10."),
        )
        .param(toggle(
            X10,
            "Rate x10",
            false,
            "Multiplies the range of Rate by ten, to 1000 Hz.",
        ))
        .param(choice(
            FEEL,
            "Feel",
            &["Straight", "Triplet", "Dotted"],
            0,
            "Scales a synced division to a triplet or a dotted length.",
        ))
        .param(toggle(
            SONG_LOCK,
            "Song lock",
            true,
            "Locks a synced LFO's phase to the host's song position.",
        ))
        .param(toggle(
            SWING,
            "Swing",
            false,
            "Swings a synced LFO by the host's swing amount.",
        ))
        .param(choice(
            FADE_IN,
            "Fade in",
            &SPANS,
            0,
            "How long the LFO takes to fade in after its delay, in tempo divisions.",
        ))
        .param(choice(
            DELAY,
            "Delay",
            &SPANS,
            0,
            "How long the LFO holds its first value after a note starts, in tempo divisions.",
        ))
        .param(
            ParamDescriptor::continuous(n(SMOOTH), "Smooth", (0.0, 100.0), 0.0, ParamUnit::None)
                .describe("Softens jumps in the LFO's output."),
        )
        .param(
            ParamDescriptor::continuous(n(PHASE), "Phase", (0.0, 360.0), 0.0, ParamUnit::None)
                .describe("Where in the shape the LFO starts, in degrees."),
        )
        .param(toggle(
            PHASE_SNAP,
            "Phase snap",
            false,
            "Snaps Phase to the shape's vertical grid lines.",
        ))
}

/// The global parameters, in the order hosts list them.
fn globals() -> Vec<ParamDescriptor> {
    use global::*;
    let mut all: Vec<ParamDescriptor> = (0..MACROS)
        .map(|m| {
            ParamDescriptor::continuous(
                macro_id(m),
                &format!("Macro {}", m + 1),
                (0.0, 100.0),
                0.0,
                ParamUnit::Percent,
            )
            .describe(
                "A macro: a control the modulation matrix routes to any number of parameters.",
            )
        })
        .collect();
    all.extend([
        ParamDescriptor::gain(MAIN_LEVEL, "Main level", gain_to_db(2f64.sqrt()), -9.0)
            .power(2)
            .describe("The level of everything routed to Main."),
        ParamDescriptor::gain(DRY_LEVEL, "Dry out level", gain_to_db(4.0), 0.0)
            .power(2)
            .describe("The level of the dry output, which skips every effect."),
        ParamDescriptor::gain(AMP, "Amp", gain_to_db(2.0), 0.0)
            .linear()
            .describe("Each voice's gain, after its envelope; a common target for velocity."),
        ParamDescriptor::continuous(TUNING, "Main tuning", (-64.0, 64.0), 0.0, ParamUnit::Semitones)
            .describe("Transposes every oscillator."),
        ParamDescriptor::choice(QUALITY, "Quality", &QUALITIES, DEFAULT_QUALITY).describe(
            "Oversampling of the voice path: Good runs it at the host's rate, High at twice it, \
             Ultra at four times. Higher is cleaner and costs more.",
        ),
        ParamDescriptor::integer(VOICES, "Voices", (1, 32), 8, ParamUnit::None)
            .describe("How many notes can sound at once; unison copies don't count."),
        ParamDescriptor::toggle(MONO, "Mono", false).describe("One note at a time."),
        ParamDescriptor::toggle(LEGATO, "Legato", false).describe(
            "Notes played over a held note don't restart the envelopes: in mono, the voice \
             re-pitches; otherwise the new note joins the held note's envelope.",
        ),
        ParamDescriptor::choice(
            STEAL,
            "Steal priority",
            &["Newest", "Oldest", "Highest", "Lowest", "Velocity"],
            0,
        )
        .describe("Which notes are kept when every voice is taken."),
        ParamDescriptor::choice(SAME_NOTE, "One voice per key", &["Off", "Sustain", "On"], 0)
            .describe("Whether a repeated note takes over its key's voice instead of stacking."),
        ParamDescriptor::continuous(
            GLIDE_TIME,
            "Glide time",
            (0.0, GLIDE_MAX_MS),
            0.0,
            ParamUnit::Milliseconds,
        )
        .power(5)
        .describe("How long the pitch takes to move to a new note."),
        ParamDescriptor::continuous(GLIDE_CURVE, "Glide curve", (-100.0, 100.0), 0.0, ParamUnit::None)
            .describe("The glide's shape: 0 even in semitones, positive leaves quickly, negative arrives quickly."),
        ParamDescriptor::toggle(GLIDE_ALWAYS, "Glide always", false)
            .describe("Glides to every new note, not only while a note is held."),
        ParamDescriptor::toggle(GLIDE_SCALED, "Glide by interval", false)
            .describe("Makes Glide time an octave's, so wider intervals take longer."),
        ParamDescriptor::integer(BEND_UP, "Bend up", (-24, 24), 2, ParamUnit::Semitones)
            .describe("How far full upward pitch bend moves the pitch."),
        ParamDescriptor::integer(BEND_DOWN, "Bend down", (-24, 24), -2, ParamUnit::Semitones)
            .describe("How far full downward pitch bend moves the pitch."),
        ParamDescriptor::toggle(MPE, "MPE", false)
            .describe("Each note takes its own pitch bend and expression from its MIDI channel."),
        ParamDescriptor::integer(MPE_BEND, "MPE bend range", (1, 96), 48, ParamUnit::Semitones)
            .describe("A note's full pitch bend with MPE on."),
        ParamDescriptor::toggle(VELOCITY_LEGATO, "Velocity legato", false)
            .describe("Whether a legato note updates its voice's velocity."),
        ParamDescriptor::toggle(CURVE_GLIDE, "Curve glide", false).describe(
            "Whether a note played over a held one moves the velocity and key curves to its \
             values over the glide time.",
        ),
        ParamDescriptor::integer(
            REFERENCE_PITCH,
            "Reference pitch",
            (430, 450),
            440,
            ParamUnit::Hertz,
        )
        .describe("The frequency of A4, applied to every oscillator and key-tracked filter."),
        ParamDescriptor::continuous(
            ENV_SCALE,
            "Envelope scale",
            (10.0, 1000.0),
            100.0,
            ParamUnit::Percent,
        )
        .describe("Multiplies every envelope's times; 100 % leaves them unchanged."),
        ParamDescriptor::continuous(
            LFO_SCALE,
            "LFO scale",
            (10.0, 1000.0),
            100.0,
            ParamUnit::Percent,
        )
        .describe("Scales every LFO; 100 % leaves them unchanged."),
    ]);
    all
}

/// Reagent's parameter space.
///
/// # Panics
///
/// Never: the space is valid, as the tests check.
#[must_use]
pub fn space() -> ParamSpace {
    let mut builder = ParamSpace::builder();
    for global in globals() {
        builder = builder.global(global);
    }
    builder
        .module(oscillator(), &OSC_NAMES)
        .module(sub(), &["Sub"])
        .module(noise(), &["Noise"])
        .module(filter_module(), &["1", "2"])
        .module(bus_module(), &["1", "2"])
        .module(envelope(ModuleType::new(AMP_ENV, "amp_env", "Env"), true), &["1"])
        .module(envelope(ModuleType::new(MOD_ENV, "env", "Env"), false), &["2", "3", "4"])
        .module(
            lfo_module(),
            &["1", "2", "3", "4", "5", "6", "7", "8", "9", "10"],
        )
        // A new patch plays oscillator A through the filters; B and C are
        // off and go to Main (engine-core R9).
        .instance_default(id(OSC, 0, channel::ON), 1.0)
        .instance_default(id(OSC, 0, channel::OUTPUT), 0.0)
        .build()
        .expect("the parameter space is valid")
}

/// Every parameter, in the order hosts list them.
#[must_use]
pub fn params() -> Vec<ParamDescriptor> {
    space().descriptors()
}

/// Env `e`'s module type and instance (0 for Env 1).
#[must_use]
pub const fn env_address(e: usize) -> (u8, u8) {
    if e == 0 {
        (AMP_ENV, 0)
    } else {
        (MOD_ENV, (e - 1) as u8)
    }
}

/// Env `e`'s parameter `param`.
#[must_use]
pub const fn env_id(e: usize, param: u16) -> ParamId {
    let (module, instance) = env_address(e);
    id(module, instance, param)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lspk_params::db_to_gain;

    fn default(id: ParamId) -> f64 {
        space()
            .descriptors()
            .into_iter()
            .find(|d| d.id == id)
            .map(|d| d.default)
            .expect("a parameter")
    }

    // Spec: engine-core R9 (A on and to the filters, B and C off and to
    // Main), R7 (75 % level), section 6 (noise 25 %).
    #[test]
    fn the_init_patch_plays_oscillator_a_through_the_filters() {
        assert_eq!(default(id(OSC, 0, channel::ON)), 1.0);
        assert_eq!(default(id(OSC, 0, channel::OUTPUT)), 0.0);
        for s in 1..3 {
            assert_eq!(default(id(OSC, s, channel::ON)), 0.0);
            assert_eq!(default(id(OSC, s, channel::OUTPUT)), 1.0);
        }
        assert_eq!(default(id(OSC, 0, channel::LEVEL)), 75.0);
        assert_eq!(default(id(OSC, 0, channel::BALANCE)), -100.0);
        assert_eq!(default(id(SUB, 0, channel::LEVEL)), 75.0);
        assert_eq!(default(id(NOISE, 0, channel::LEVEL)), 25.0);
        // Section 2: Phase 180°, Phase random 100 %.
        assert_eq!(default(id(OSC, 0, osc::PHASE)), 180.0);
        assert_eq!(default(id(OSC, 0, osc::PHASE_RANDOM)), 100.0);
        // Section 4: one copy, Detune 0.25, Blend 75 %, Width 100, Range 2.
        assert_eq!(default(id(OSC, 0, osc::UNISON)), 1.0);
        assert_eq!(default(id(OSC, 0, osc::DETUNE)), 0.25);
        assert_eq!(default(id(OSC, 0, osc::WIDTH)), 100.0);
        assert_eq!(default(id(OSC, 0, osc::DETUNE_RANGE)), 2.0);
    }

    // Spec: engine-core section 7 (off, the 12 dB ladder, 425 Hz,
    // resonance 10 %), section 8 (filters, buses to Main; the amp envelope
    // on for filter channels).
    #[test]
    fn the_init_filters_and_buses_are_as_observed() {
        let f = |param: u16| default(id(FILTER, 0, param));
        assert_eq!(f(filter::ON), 0.0);
        assert_eq!(FILTER_TYPES[f(filter::TYPE) as usize], "Ladder LP 12");
        assert!((f(filter::CUTOFF) - 425.0).abs() < 0.5);
        assert_eq!(f(filter::RESONANCE), 10.0);
        assert_eq!(f(filter::MIX), 100.0);
        assert_eq!(f(filter::LEVEL), 1.0);
        assert_eq!(f(filter::OUTPUT), 1.0);
        assert_eq!(f(filter::AMP_ENV), 1.0);
        assert_eq!(default(id(BUS, 0, bus::OUTPUT)), 0.0);
        assert_eq!(default(id(BUS, 1, bus::LEVEL)), 1.0);
        // R51: the cutoff's travel is even in pitch from MIDI note 0.
        let cutoff = space()
            .descriptors()
            .into_iter()
            .find(|d| d.id == id(FILTER, 0, filter::CUTOFF))
            .expect("the cutoff");
        assert!((cutoff.denormalize(0.5) - 425.0).abs() < 0.5);
        assert!((cutoff.denormalize(0.0) - 8.18).abs() < 0.01);
    }

    // Spec: engine-core R70 (t = 32 s × n⁵), section 9 (Env 1 attack 1 ms,
    // Env 2–4 0.5 ms; decay 1 s; release 15 ms; curves 50, 66.6, 66.6),
    // R79 (Env 1 restarts from its level, the others from their start).
    #[test]
    fn the_init_envelopes_are_as_observed() {
        assert_eq!(default(env_id(0, env::ATTACK)), 1.0);
        assert_eq!(default(env_id(1, env::ATTACK)), 0.5);
        assert_eq!(default(env_id(3, env::DECAY)), 1000.0);
        assert_eq!(default(env_id(0, env::SUSTAIN)), 1.0);
        assert_eq!(default(env_id(2, env::SUSTAIN)), 100.0);
        assert_eq!(default(env_id(0, env::RELEASE)), 15.0);
        assert_eq!(default(env_id(0, env::DECAY_CURVE)), 66.6);
        assert_eq!(default(env_id(0, env::STEAL_RESTART)), 0.0);
        assert_eq!(default(env_id(1, env::STEAL_RESTART)), 1.0);
        assert_eq!(default(env_id(1, env::START)), 0.0);
        let attack = space()
            .descriptors()
            .into_iter()
            .find(|d| d.id == env_id(0, env::ATTACK))
            .expect("an attack");
        assert!((attack.denormalize(0.5) - 1000.0).abs() < 1e-9);
    }

    // Spec: engine-core section 10 (Restart, synced at a quarter note, song
    // lock on), sections 13 to 15 (macros 0, 8 voices, Newest, High
    // quality, Main −9 dB, Amp unity, bend ±2).
    #[test]
    fn the_init_lfos_voicing_and_globals_are_as_observed() {
        assert_eq!(default(id(LFO, 9, lfo::MODE)), 1.0);
        assert_eq!(default(id(LFO, 0, lfo::SYNC)), 1.0);
        assert_eq!(
            DIVISIONS[default(id(LFO, 0, lfo::DIVISION)) as usize],
            "1/4"
        );
        assert_eq!(default(id(LFO, 0, lfo::SONG_LOCK)), 1.0);
        assert_eq!(default(global::macro_id(7)), 0.0);
        assert_eq!(default(global::VOICES), 8.0);
        assert_eq!(default(global::STEAL), 0.0);
        assert_eq!(QUALITIES[default(global::QUALITY) as usize], "High");
        assert!((default(global::MAIN_LEVEL) - db_to_gain(-9.0)).abs() < 1e-12);
        assert!((default(global::AMP) - 1.0).abs() < 1e-12);
        assert_eq!(default(global::BEND_UP), 2.0);
        assert_eq!(default(global::BEND_DOWN), -2.0);
        assert_eq!(default(global::REFERENCE_PITCH), 440.0);
        assert_eq!(default(global::ENV_SCALE), 100.0);
        assert_eq!(default(global::LFO_SCALE), 100.0);
    }

    // Spec: engine-core R80 (envelope scale 10 % to 1000 %, default 100 %),
    // R97 (LFO scale 10 % to 1000 %, default 100 %), R132 (quality Good,
    // High, Ultra; default High), R136 (reference pitch 430 to 450 Hz,
    // default 440).
    #[test]
    fn global_settings_have_specified_ranges_and_defaults() {
        let descriptors = space().descriptors();
        let find = |id: ParamId| descriptors.iter().find(|d| d.id == id).expect("param");

        let quality = find(global::QUALITY);
        assert_eq!(quality.name, "Quality");
        assert_eq!(QUALITIES[quality.default as usize], "High");

        let ref_pitch = find(global::REFERENCE_PITCH);
        assert_eq!(ref_pitch.name, "Reference pitch");
        assert_eq!(ref_pitch.default, 440.0);
        assert_eq!(ref_pitch.min(), 430.0);
        assert_eq!(ref_pitch.max(), 450.0);
        assert!(ref_pitch.is_stepped());
        assert_eq!(ref_pitch.format(440.0), "440 Hz");

        let env_scale = find(global::ENV_SCALE);
        assert_eq!(env_scale.name, "Envelope scale");
        assert_eq!(env_scale.default, 100.0);
        assert_eq!(env_scale.min(), 10.0);
        assert_eq!(env_scale.max(), 1000.0);
        assert_eq!(env_scale.format(100.0), "100 %");

        let lfo_scale = find(global::LFO_SCALE);
        assert_eq!(lfo_scale.name, "LFO scale");
        assert_eq!(lfo_scale.default, 100.0);
        assert_eq!(lfo_scale.min(), 10.0);
        assert_eq!(lfo_scale.max(), 1000.0);
        assert_eq!(lfo_scale.format(100.0), "100 %");
    }

    // Spec: engine-core rows for Level (filters), Bus 1 and Bus 2 Vol,
    // Direct Vol, Main Vol and Amp (task T26).
    fn check_knob_law(param_id: ParamId, law: impl Fn(f64) -> f64, expected_default_gain: f64) {
        let desc = space()
            .descriptors()
            .into_iter()
            .find(|d| d.id == param_id)
            .expect("descriptor");
        assert!(
            (desc.default - expected_default_gain).abs() < 1e-12,
            "default gain unchanged"
        );
        for n in [0.0, 0.25, 0.5, 1.0] {
            let gain = desc.denormalize(n);
            let expected = law(n);
            assert!(
                (gain - expected).abs() < 1e-12,
                "at n={n}, gain {gain} != expected {expected}"
            );
            assert!(
                (desc.normalize(gain) - n).abs() < 1e-12,
                "normalize at n={n}"
            );
        }
        assert_eq!(desc.format(0.0), "-inf dB");
    }

    #[test]
    fn filter_1_level_follows_its_knob_law() {
        check_knob_law(id(FILTER, 0, filter::LEVEL), |n| (2.0 * n).powi(2), 1.0);
    }

    #[test]
    fn filter_2_level_follows_its_knob_law() {
        check_knob_law(id(FILTER, 1, filter::LEVEL), |n| (2.0 * n).powi(2), 1.0);
    }

    #[test]
    fn bus_1_level_follows_its_knob_law() {
        check_knob_law(id(BUS, 0, bus::LEVEL), |n| (2.0 * n).powi(2), 1.0);
    }

    #[test]
    fn bus_2_level_follows_its_knob_law() {
        check_knob_law(id(BUS, 1, bus::LEVEL), |n| (2.0 * n).powi(2), 1.0);
    }

    #[test]
    fn direct_vol_follows_its_knob_law() {
        check_knob_law(global::DRY_LEVEL, |n| (2.0 * n).powi(2), 1.0);
    }

    #[test]
    fn main_vol_follows_its_knob_law() {
        check_knob_law(
            global::MAIN_LEVEL,
            |n| 2f64.sqrt() * n.powi(2),
            db_to_gain(-9.0),
        );
    }

    #[test]
    fn amp_follows_its_knob_law() {
        check_knob_law(global::AMP, |n| 2.0 * n, 1.0);
    }

    #[test]
    fn every_parameter_is_named_by_its_module() {
        let names: Vec<String> = params().into_iter().map(|d| d.name).collect();
        for name in [
            "Osc A On",
            "Osc Sub Level",
            "Osc Noise Pan",
            "Filter 2 Type",
            "Env 3 Start level",
            "LFO 7 Division",
            "Bus 2 Output",
            "Macro 3",
            "Glide time",
        ] {
            assert!(names.iter().any(|n| n == name), "{name}");
        }
    }
}
