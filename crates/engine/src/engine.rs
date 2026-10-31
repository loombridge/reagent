//! The engine on the audio thread: Reagent's voice architecture assembled
//! from the plug-in kit.
//!
//! # A block
//!
//! The kit's voice manager (`lspk-voice`) takes the block's notes and
//! controllers and calls back for every stretch between events, split at
//! its 32-frame control points, so each stretch lies inside one control
//! period. For each stretch the engine:
//!
//! 1. takes the patch the plug-in's main thread compiled, if one waits, and
//!    applies parameter changes and the voice settings;
//! 2. renders the four envelopes, and the LFOs some route reads, at the
//!    host's rate;
//! 3. runs the modulation matrix (`lspk-mod`) over the stretch;
//! 4. renders the audio path at the quality's rate (1, 2 or 4 times the
//!    host's, engine-core R132): oscillators A to C (`lspk-wavetable`)
//!    into the mixer, the filters (`lspk-dsp`) in series or parallel, Env 1
//!    and Amp where each signal leaves the voice, then Main, Dry out and the
//!    buses, summed over the voices;
//! 5. routes the buses and applies Main and Dry out's levels.
//!
//! After the block, the oversampler's way down brings the output to the
//! host's rate.
//!
//! The sub and noise channels (R1.8) play the kit's sub and noise
//! oscillators — the sub's six fixed shapes, and the noise's playback of
//! Reagent's own colour blocks — through the same routing, level and pan
//! as the main oscillators.
//!
//! # Audio sources in the matrix
//!
//! A route from an oscillator's or a filter's audio reads it one control
//! period (32 frames at the host's rate) late, from a ring of the last two
//! periods, because the matrix runs before the audio it would read. The
//! delay is fixed to the period grid, so renders don't depend on block
//! size. The shapers' inter-oscillator modulation (R1.5) reads audio
//! sample by sample instead.
//!
//! # Real-time safety
//!
//! Everything is allocated in [`Engine::new`]; processing allocates
//! nothing, locks nothing and never blocks. A new modulation program and a
//! new compiled patch arrive through their mailboxes at a block's start,
//! and the program one replaces goes back to its sender to be dropped.

use std::sync::Arc;

use lspk_dsp::Oversampler;
use lspk_dsp::envelope::{EnvelopeBank, EnvelopeParams, StealRestart, TimeUnit, Timing, Trigger};
use lspk_dsp::filter::{FilterBank, FilterType, VoiceParams, pitch_of};
use lspk_dsp::lfo::{
    Direction, Division, Feel, Lfo, LfoBank, LfoType, Mode, Rate as LfoRate,
    Settings as LfoSettings, Span, Transport,
};
use lspk_mod::{
    Config, DestId, Engine as ModEngine, NoteOn, Output, ProgramReceiver, Signals, SourceId, Target,
};
use lspk_params::{ParamAddress, ParamDescriptor, ParamId};
use lspk_plugin::{
    Buffers, Event, Events, Note, ParamValues, ProcessContext, ProcessStatus, Processor,
};
use lspk_voice::{
    CONTROL_INTERVAL, DetuneLaw, Glide, Layout, Mpe, SameNote, Settings, Stack, Start,
    StealPriority, Unison, UnisonLanes, VoiceBlock, VoiceEvent, VoiceEventKind, VoiceHandler,
    Voices,
};
use lspk_wavetable::SharedTable;
use lspk_wavetable::noise::{
    COLOUR_RATE, COLOUR_SECONDS, Colour, Inputs as NoiseInputs, Noise, Samples,
};
use lspk_wavetable::osc::{Inputs, Layout as OscLayout, Onset, Oscillator, PhaseMode};
use lspk_wavetable::sub::{Inputs as SubInputs, Shape as SubShape, Sub};

use crate::modulation::{AUDIO_SOURCES, Needs, Registry, audio_bit};
use crate::params::{
    self, AMP_ENV, BUS, BUSES, ENVELOPES, FILTER, FILTERS, LFO, LFOS, MACROS, MOD_ENV, NOISE, OSC,
    OSCILLATORS, SUB, bus, channel, env, filter, global, lfo, noise, osc, sub,
};
use crate::patch::{CHANNELS, ChannelState, FilterState, Patch, PatchReceiver};
use crate::routing::{self, BusOutput, ChannelOutput, FilterOutput, balance_shares, pan_gains};
use crate::stepping::{Stepper, Stepping};
use crate::telemetry::{self, Frame, Publisher};
use crate::tuning::{Control, Tuner};

/// The most voices (engine-core R117).
pub const MAX_VOICES: usize = lspk_voice::MAX_VOICES;
/// The most unison copies of an oscillator per voice (engine-core
/// section 4).
pub const MAX_COPIES: usize = lspk_voice::unison::MAX_COPIES;
/// Lanes in an oscillator's buffers at the most copies.
const LANES: usize = MAX_VOICES * MAX_COPIES;
/// Lanes per per-voice buffer: the voices, padded to the widest SIMD
/// vector.
const S: usize = MAX_VOICES;
/// The highest oversampling factor (Ultra).
pub const MAX_FACTOR: usize = 4;
/// The most host frames one stretch holds: a control period.
const CHUNK: usize = CONTROL_INTERVAL as usize;
/// The most frames one stretch holds at the highest rate.
const OS_CHUNK: usize = CHUNK * MAX_FACTOR;
/// How many frames after the last voice ends the engine keeps processing,
/// so the oversampler's filter empties, before it sleeps.
const SETTLE_FRAMES: usize = 256;
/// The most note ends and parameter resets one block reports.
const REPORTS: usize = 256;

/// The seed each colour noise block is generated from (R49).
const COLOUR_SEEDS: [u64; 4] = [0x0E5E_0001, 0x0E5E_0002, 0x0E5E_0003, 0x0E5E_0004];

/// The oversampling factor of quality index `quality` (Good, High, Ultra).
#[must_use]
pub fn quality_factor(quality: f64) -> usize {
    match quality.round() as i64 {
        0 => 1,
        2 => 4,
        _ => 2,
    }
}

/// The latency in host frames at quality index `quality`: the way down of
/// the oversampler (engine-core R132).
#[must_use]
pub fn latency(quality: f64) -> u32 {
    let factor = quality_factor(quality);
    Oversampler::new(factor.trailing_zeros(), 1, 1).decimation_latency()
}

/// The kit's filter type for Type choice `index`.
#[must_use]
pub fn filter_type(index: f64) -> FilterType {
    use FilterType::*;
    const TYPES: [FilterType; 18] = [
        Ladder6, Ladder12, Ladder18, Ladder24, Lp6, Lp12, Lp18, Lp24, Hp6, Hp12, Hp18, Hp24, Bp12,
        Bp24, Peak12, Peak24, Notch12, Notch24,
    ];
    let i = index.round().max(0.0) as usize;
    if i < TYPES.len() {
        TYPES[i]
    } else {
        // The Multi, Flanges and Misc groups follow in the kit's order.
        FilterType::ALL[i.min(FilterType::ALL.len() - 1)]
    }
}

/// One oscillator laid out for one unison count: its lanes and the copies'
/// pitch, frame and gains.
struct Unit {
    osc: Oscillator,
    lanes: UnisonLanes,
}

/// Parameter values by index, with each one's descriptor.
#[derive(Debug)]
struct Values {
    /// `(id, index)`, sorted by ID.
    ids: Vec<(u32, u16)>,
    descriptors: Vec<ParamDescriptor>,
    values: Vec<f64>,
    /// Each parameter's destination, if it's modulated.
    dests: Vec<Option<DestId>>,
}

impl Values {
    fn index(&self, id: ParamId) -> Option<usize> {
        self.ids
            .binary_search_by_key(&id.0, |&(i, _)| i)
            .ok()
            .map(|at| usize::from(self.ids[at].1))
    }

    fn get(&self, id: ParamId) -> f64 {
        self.index(id).map_or(0.0, |i| self.values[i])
    }

    fn on(&self, id: ParamId) -> bool {
        self.get(id) >= 0.5
    }

    fn dest(&self, id: ParamId) -> Option<DestId> {
        self.index(id).and_then(|i| self.dests[i])
    }

    /// The real value of parameter `id` at travel `position`.
    fn real(&self, id: ParamId, position: f32) -> f32 {
        self.index(id).map_or(0.0, |i| {
            self.descriptors[i].denormalize(f64::from(position)) as f32
        })
    }
}

/// The modulated values of destination `dest` for the stretch.
fn modulation(modeng: &ModEngine, dest: Option<DestId>) -> Mod<'_> {
    match dest.map(|d| modeng.output(d)) {
        Some(Output::Audio(a)) => Mod::Audio(a),
        Some(Output::Control(c)) => Mod::Control(c),
        _ => Mod::None,
    }
}

/// Fills `buf[..n * S]` with destination `dest`'s value at every host frame
/// of the stretch and voice: an audio-rate destination's own value, a
/// control-rate one's ramp between ticks, so a value the ear follows per
/// frame never steps (engine-core R1.13).
fn fill_ramp(modeng: &ModEngine, buf: &mut [f32], dest: Option<DestId>, n: usize, span: usize) {
    let mut one = [0.0f32; CHUNK];
    match modulation(modeng, dest) {
        Mod::None => {}
        Mod::Audio(a) => {
            for t in 0..n {
                for v in 0..span {
                    buf[t * S + v] = a.at(t, v);
                }
            }
        }
        Mod::Control(c) => {
            for v in 0..span {
                c.ramp(v, &mut one[..n]);
                for t in 0..n {
                    buf[t * S + v] = one[t];
                }
            }
        }
    }
}

/// Writes a source's audio for the matrix into the current period of its
/// ring: `read(j, v)` is voice `v` at frame `j` of the stretch at the
/// quality's rate, and every `factor`th frame is kept.
fn store_ring(
    ring: &mut [f32],
    clock: u64,
    factor: usize,
    n: usize,
    span: usize,
    read: impl Fn(usize, usize) -> f32,
) {
    let phase = (clock % CHUNK as u64) as usize;
    let period = (clock / CHUNK as u64) as usize % 2;
    let ring = &mut ring[period * CHUNK * S..(period + 1) * CHUNK * S];
    for t in 0..n {
        for v in 0..span {
            ring[(phase + t) * S + v] = read(t * factor, v);
        }
    }
}

/// What a bus's parameters were last turned into.
#[derive(Clone, Copy, Debug)]
struct BusState {
    level: f32,
    output: BusOutput,
}

/// Which modules changed since they were last rebuilt, one bit each.
mod dirty {
    pub const OSC: u64 = 1; // bits 0..3
    pub const SUB: u64 = 1 << 3;
    pub const NOISE: u64 = 1 << 4;
    pub const FILTER: u64 = 1 << 5; // bits 5..7
    pub const BUS: u64 = 1 << 7;
    pub const ENV: u64 = 1 << 8; // bits 8..12
    pub const LFO: u64 = 1 << 12; // bits 12..22
    pub const GLOBAL: u64 = 1 << 22;
    pub const VOICE: u64 = 1 << 23;
    pub const ALL: u64 = (1 << 24) - 1;
}

/// The module bit parameter `id` dirties.
fn dirty_bit(id: ParamId) -> u64 {
    let Some(a) = ParamAddress::of(id) else {
        return 0;
    };
    let i = u32::from(a.instance);
    match a.module {
        OSC => dirty::OSC << i,
        SUB => dirty::SUB,
        NOISE => dirty::NOISE,
        FILTER => dirty::FILTER << i,
        BUS => dirty::BUS,
        AMP_ENV => dirty::ENV,
        MOD_ENV => dirty::ENV << (i + 1),
        LFO => dirty::LFO << i,
        0 => {
            if id == global::VOICES || (id.0 >= global::MONO.0 && id.0 <= global::VELOCITY_LEGATO.0)
            {
                dirty::VOICE | dirty::GLOBAL
            } else {
                dirty::GLOBAL
            }
        }
        _ => 0,
    }
}

/// The matrix's source IDs the engine binds or sets.
#[derive(Debug)]
struct SourceIds {
    env: [SourceId; ENVELOPES],
    lfo: [SourceId; LFOS],
    lfo_y: [SourceId; LFOS],
    audio: [SourceId; AUDIO_SOURCES.len()],
    macros: [SourceId; MACROS],
    mod_wheel: SourceId,
    pitch_bend: SourceId,
    channel_pressure: SourceId,
    poly_pressure: SourceId,
    mpe_y: SourceId,
    mpe_z: SourceId,
}

impl SourceIds {
    fn new(registry: &Registry) -> Self {
        let find = |key: &str| {
            registry
                .sources
                .find(key)
                .expect("the standard sources hold it")
        };
        Self {
            env: std::array::from_fn(|e| find(&format!("env{}", e + 1))),
            lfo: std::array::from_fn(|l| find(&format!("lfo{}", l + 1))),
            lfo_y: std::array::from_fn(|l| find(&format!("lfo{}_y", l + 1))),
            audio: std::array::from_fn(|a| find(AUDIO_SOURCES[a].0)),
            macros: std::array::from_fn(|m| find(&format!("macro{}", m + 1))),
            mod_wheel: find("mod_wheel"),
            pitch_bend: find("pitch_bend"),
            channel_pressure: find("channel_pressure"),
            poly_pressure: find("poly_pressure"),
            mpe_y: find("mpe_y"),
            mpe_z: find("mpe_z"),
        }
    }
}

/// A modulated destination's values for one stretch, per voice.
#[derive(Clone, Copy)]
enum Mod<'a> {
    None,
    /// One value per host frame per voice (travel positions).
    Audio(lspk_mod::AudioOut<'a>),
    /// One value per voice for the stretch.
    Control(lspk_mod::ControlOut<'a>),
}

impl Mod<'_> {
    fn is_some(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// Voice `v`'s position at host frame `k` of the stretch.
    #[inline]
    fn at(&self, k: usize, v: usize) -> Option<f32> {
        match self {
            Self::None => None,
            Self::Audio(a) => Some(a.at(k, v)),
            Self::Control(c) => (!c.chunks().is_empty()).then(|| c.value(c.chunks().len() - 1, v)),
        }
    }
}

/// Everything but the voice manager: the [`VoiceHandler`] the voices call.
struct Synth {
    rate: f64,
    factor: usize,
    registry: Arc<Registry>,
    values: Values,
    dirty: u64,
    needs: Arc<Needs>,
    needed: u32,

    table_frames: usize,
    /// Each oscillator laid out for every unison count, 1 to 16 copies, so
    /// changing the count never allocates.
    units: Vec<Vec<Unit>>,
    /// The sub oscillator, one lane per voice (R40).
    sub: Sub,
    /// The noise oscillator, one lane per voice (R45).
    noise: Noise,
    /// Reagent's own colour noise blocks, the noise's sources (R49).
    colours: Vec<Samples>,
    /// Which colour the noise plays.
    noise_source: usize,
    /// Each oscillator's unison count now.
    copies: [usize; OSCILLATORS],
    /// Each oscillator's unison controls.
    unison: [Unison; OSCILLATORS],
    /// Oscillators whose unison count changed: their sounding voices start
    /// again on the new layout.
    restart: [bool; OSCILLATORS],
    osc_params: [lspk_wavetable::osc::Params; OSCILLATORS],
    /// The sub's controls, as last rebuilt.
    sub_params: lspk_wavetable::sub::Params,
    /// The noise's controls, as last rebuilt.
    noise_params: lspk_wavetable::noise::Params,
    channels: [ChannelState; CHANNELS],
    filters: Vec<FilterBank>,
    filter_state: [FilterState; FILTERS],
    buses: [BusState; BUSES],
    envs: Vec<EnvelopeBank>,
    env_params: [EnvelopeParams; ENVELOPES],
    lfos: Vec<LfoBank>,
    transport: Transport,
    modeng: ModEngine,
    src: SourceIds,

    main_level: f32,
    dry_level: f32,
    amp: f32,
    tuning: f32,
    reference: f32,
    /// Where each note's tuning comes from, and what it gave each voice at
    /// its start (or, following an MTS-ESP master, as of this stretch).
    tuner: Tuner,
    tune_off: [f32; S],
    /// Each voice's pitch with its tuning added: the pitch everything that
    /// follows the note reads.
    tuned: [f32; S],
    env_scale: f64,
    /// The tempo the envelopes' timing was built at.
    env_tempo: f64,
    /// The program this thread runs: what is in use, compiled off this
    /// thread by the plug-in's main thread and swapped at a block's start,
    /// or refreshed here when a host's automation changes it.
    patch: Patch,
    /// Where a newly compiled patch arrives, while the processor is active.
    patch_mailbox: Option<PatchReceiver>,

    // Host-rate buffers for one stretch, `CHUNK × S`.
    env_buf: Vec<Vec<f32>>,
    lfo_buf: Vec<Vec<f32>>,
    lfo_y_buf: Vec<Vec<f32>>,
    audio_buf: Vec<Vec<f32>>,
    /// The audio sources' last two periods, `2 × CHUNK × S` each.
    audio_ring: Vec<Vec<f32>>,
    /// The host-rate gains of the stretch, `CHUNK × S`.
    gain_l: Vec<f32>,
    gain_r: Vec<f32>,
    amp_buf: Vec<f32>,
    env_amp: Vec<f32>,
    // Rate-scaled pitch steps and modulation for the oscillators.
    lane_pitch: Vec<f32>,
    pitch_step: Vec<f32>,
    pitch_mod: Vec<f32>,
    frame_mod: Vec<f32>,
    /// One control-rate destination's values at every host frame of the
    /// stretch, per voice: its ramp between ticks (never a step).
    ramp_buf: Vec<f32>,
    /// An oscillator's copies mixed to a stereo pair per voice.
    uni_out: Vec<f32>,
    // Audio-rate buffers for one stretch.
    osc_out: Vec<f32>,
    /// Filter inputs, `OS_CHUNK × 2 × S` each.
    fin: [Vec<f32>; FILTERS],
    cutoff: Vec<f32>,
    /// Per-voice Main, Dry out, Bus 1 and Bus 2, `OS_CHUNK × 2 × S` each.
    vpath: [Vec<f32>; 4],
    /// The same summed over the voices, `OS_CHUNK × 2` each.
    path: [Vec<f32>; 4],
    /// The block's output at the quality's rate, per channel.
    out: [Vec<f32>; 2],

    /// Note ends to report, and parameter resets to report.
    ends: Vec<(u32, Note)>,
    resets: Vec<(u32, ParamId, f64)>,
    /// The frame in the block the current stretch starts at.
    block_frame: usize,
    /// The engine's clock in host frames, for the audio ring's periods.
    clock: u64,
}

/// Reagent's engine: the processor a plug-in format runs.
pub struct Engine {
    voices: Voices,
    synth: Synth,
    oversamplers: [Oversampler; 3],
    max_frames: usize,
    quiet: usize,
    /// Where the interface's live values go, once an editor can watch.
    telemetry: Option<telemetry::State>,
    /// The tuning's other reader: it drops the notes a tuning leaves out
    /// before the voices see them.
    tuner: Tuner,
    stepper: Stepper,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("rate", &self.synth.rate)
            .field("factor", &self.synth.factor)
            .finish_non_exhaustive()
    }
}

/// The modulation engine's configuration at host rate `rate`.
#[must_use]
pub fn mod_config(rate: f64) -> Config {
    Config {
        sample_rate: rate as f32,
        max_frames: CHUNK,
        voices: MAX_VOICES,
    }
}

impl Engine {
    /// An engine at `rate` for blocks of up to `max_frames`, with the
    /// parameters' values `values`, playing `table`, taking modulation
    /// programs from `programs` and reading which sources they need from
    /// `needs`. Allocates everything it will use.
    #[must_use]
    pub fn new(
        rate: f64,
        max_frames: usize,
        values: &ParamValues,
        registry: Arc<Registry>,
        table: SharedTable,
        programs: ProgramReceiver,
        needs: Arc<Needs>,
    ) -> Self {
        let max_frames = max_frames.max(1);
        let descriptors = values.descriptors().to_vec();
        let mut ids: Vec<(u32, u16)> = descriptors
            .iter()
            .enumerate()
            .map(|(i, d)| (d.id.0, i as u16))
            .collect();
        ids.sort_unstable();
        let dests = descriptors
            .iter()
            .map(|d| registry.dest(Target::Param(d.id)))
            .collect();
        let vals = (0..descriptors.len()).map(|i| values.value_at(i)).collect();
        let values = Values {
            ids,
            descriptors,
            values: vals,
            dests,
        };
        let factor = quality_factor(values.get(global::QUALITY));
        let os_rate = (rate * factor as f64) as f32;
        let mut seed = 0x5EED_u64;
        let units = (0..OSCILLATORS)
            .map(|_| {
                (1..=MAX_COPIES)
                    .map(|copies| {
                        seed += 1;
                        Unit {
                            osc: Oscillator::new(
                                table.clone(),
                                OscLayout {
                                    voices: MAX_VOICES,
                                    lanes_per_voice: copies,
                                },
                                os_rate,
                                OS_CHUNK,
                                seed,
                            ),
                            lanes: UnisonLanes::new(MAX_VOICES, copies, seed),
                        }
                    })
                    .collect()
            })
            .collect();
        let filters = (0..FILTERS)
            .map(|_| FilterBank::new(os_rate, MAX_VOICES, 2))
            .collect();
        // Reagent's own colour noises, the noise oscillator's sources,
        // generated in code at 48 kHz (R49).
        let colours: Vec<Samples> = Colour::ALL
            .into_iter()
            .zip(COLOUR_SEEDS)
            .map(|(colour, seed)| Samples::colour(colour, COLOUR_SECONDS, COLOUR_RATE, seed))
            .collect();
        let sub = Sub::new(MAX_VOICES, os_rate, OS_CHUNK);
        let noise = Noise::new(MAX_VOICES, os_rate, OS_CHUNK, 0x5EED);
        let timing = Timing::new(rate);
        let envs = (0..ENVELOPES)
            .map(|e| {
                let p = if e == 0 {
                    EnvelopeParams::AMP
                } else {
                    EnvelopeParams::MODULATION
                };
                EnvelopeBank::new(MAX_VOICES, p, timing)
            })
            .collect();
        let lfos = (0..LFOS)
            .map(|l| {
                LfoBank::new(
                    Lfo::new(LfoSettings::default(), 0xF0 + l as u64),
                    MAX_VOICES,
                )
            })
            .collect();
        let config = mod_config(rate);
        let mut modeng = ModEngine::new(&registry.sources, &registry.destinations, config);
        modeng.attach(programs);
        let src = SourceIds::new(&registry);
        let chunk = || vec![0.0f32; CHUNK * S];
        let table_frames = table.frame_count();
        let mut synth = Synth {
            rate,
            factor,
            registry,
            values,
            dirty: dirty::ALL,
            needs,
            needed: 0,
            table_frames,
            units,
            sub,
            noise,
            colours,
            noise_source: params::DEFAULT_NOISE_SOURCE,
            copies: [1; OSCILLATORS],
            unison: [Unison::default(); OSCILLATORS],
            restart: [false; OSCILLATORS],
            osc_params: [lspk_wavetable::osc::Params::default(); OSCILLATORS],
            sub_params: lspk_wavetable::sub::Params::default(),
            noise_params: lspk_wavetable::noise::Params::default(),
            channels: [ChannelState {
                on: false,
                level: 0.0,
                pan: 0.0,
                output: ChannelOutput::Main,
                balance: -100.0,
                send: [0.0; 2],
                amp_env: true,
            }; CHANNELS],
            filters,
            filter_state: [FilterState {
                on: false,
                cutoff: 60.0,
                voice: VoiceParams::default(),
                clean: false,
                key_track: false,
                output: FilterOutput::Main,
                send: [0.0; 2],
                amp_env: true,
            }; FILTERS],
            buses: [BusState {
                level: 1.0,
                output: BusOutput::Main,
            }; BUSES],
            envs,
            env_params: [EnvelopeParams::AMP; ENVELOPES],
            lfos,
            transport: Transport::new(rate),
            modeng,
            src,
            main_level: 1.0,
            dry_level: 1.0,
            amp: 1.0,
            tuning: 0.0,
            reference: 440.0,
            tuner: Tuner::new(Control::new(), None),
            tune_off: [0.0; S],
            tuned: [0.0; S],
            env_scale: 1.0,
            env_tempo: 120.0,
            patch: Patch::default(),
            patch_mailbox: None,
            env_buf: (0..ENVELOPES).map(|_| chunk()).collect(),
            lfo_buf: (0..LFOS).map(|_| chunk()).collect(),
            lfo_y_buf: (0..LFOS).map(|_| chunk()).collect(),
            audio_buf: (0..AUDIO_SOURCES.len()).map(|_| chunk()).collect(),
            audio_ring: (0..AUDIO_SOURCES.len())
                .map(|_| vec![0.0; 2 * CHUNK * S])
                .collect(),
            gain_l: chunk(),
            gain_r: chunk(),
            amp_buf: chunk(),
            env_amp: chunk(),
            lane_pitch: vec![0.0; LANES],
            pitch_step: vec![0.0; LANES],
            pitch_mod: vec![0.0; OS_CHUNK * LANES],
            frame_mod: vec![0.0; OS_CHUNK * LANES],
            ramp_buf: vec![0.0; CHUNK * S],
            osc_out: vec![0.0; OS_CHUNK * LANES],
            uni_out: vec![0.0; OS_CHUNK * 2 * S],
            fin: std::array::from_fn(|_| vec![0.0; OS_CHUNK * 2 * S]),
            cutoff: vec![0.0; OS_CHUNK * S],
            vpath: std::array::from_fn(|_| vec![0.0; OS_CHUNK * 2 * S]),
            path: std::array::from_fn(|_| vec![0.0; OS_CHUNK * 2]),
            out: std::array::from_fn(|_| vec![0.0; max_frames * MAX_FACTOR]),
            ends: Vec::with_capacity(REPORTS),
            resets: Vec::with_capacity(REPORTS),
            block_frame: 0,
            clock: 0,
        };
        let mut voices = Voices::new(
            Layout {
                voices: MAX_VOICES,
                lanes_per_voice: 1,
            },
            rate as f32,
        );
        // Every modulated parameter's base, and the macros' values.
        for i in 0..synth.values.values.len() {
            let id = synth.values.descriptors[i].id;
            let value = synth.values.values[i];
            if let Some(dest) = synth.values.dests[i] {
                let position = synth.values.descriptors[i].normalize(value) as f32;
                synth.modeng.set_base(dest, position);
            }
            if let Some(m) = (0..MACROS).find(|&m| global::macro_id(m) == id) {
                synth
                    .modeng
                    .set_held(synth.src.macros[m], (value / 100.0) as f32);
            }
        }
        synth.rebuild(voices.settings_mut());
        let oversamplers = [
            Oversampler::new(0, 2, max_frames),
            Oversampler::new(1, 2, max_frames),
            Oversampler::new(2, 2, max_frames),
        ];
        Self {
            voices,
            synth,
            oversamplers,
            max_frames,
            quiet: SETTLE_FRAMES,
            telemetry: None,
            tuner: Tuner::new(Control::new(), None),
            stepper: Stepper::new(Stepping::new()),
        }
    }

    /// The oversampling factor it renders at now.
    #[must_use]
    pub fn factor(&self) -> usize {
        self.synth.factor
    }

    /// How many voices are sounding.
    #[must_use]
    pub fn active_voices(&self) -> usize {
        self.voices.block().active_count()
    }

    /// Takes compiled patches from `patches` at each block's start, as the
    /// plug-in does at activation. Without one, the engine still refreshes
    /// its own program when a host's automation changes what is in use.
    pub fn attach_patches(&mut self, patches: PatchReceiver) {
        self.synth.patch_mailbox = Some(patches);
    }

    /// Publishes the interface's live values through `publisher`, while its
    /// watcher says an editor is open; otherwise it costs one atomic load
    /// per block. Allocates, so call it off the audio thread.
    pub fn attach_telemetry(&mut self, publisher: Publisher<Frame>) {
        self.telemetry = Some(telemetry::State::new(publisher, self.max_frames));
    }

    /// Takes the publisher back, as the plug-in does when it deactivates, to
    /// hand it to the next engine.
    pub fn take_telemetry(&mut self) -> Option<Publisher<Frame>> {
        self.telemetry.take().map(|t| t.publisher)
    }

    /// Tunes notes by `tuner`'s sources, as the plug-in does at activation.
    pub fn attach_tuner(&mut self, tuner: Tuner) {
        self.synth.tuner = tuner.clone();
        self.tuner = tuner;
    }

    /// Steps presets on the controllers or keys `stepping` assigns.
    pub fn attach_stepping(&mut self, stepping: Arc<Stepping>) {
        self.stepper = Stepper::new(stepping);
    }

    fn render_block(
        &mut self,
        audio: &mut Buffers<'_>,
        events: &Events<'_>,
        cx: &mut ProcessContext<'_>,
    ) -> ProcessStatus {
        let frames = audio.frames();
        self.synth.begin_block();
        self.synth.modeng.begin_block();
        self.synth.needed = self.synth.needs.get();
        let factor = self.synth.factor;
        for out in &mut self.synth.out {
            out[..frames * factor].fill(0.0);
        }
        self.synth.block_frame = 0;
        // The notes that step a preset and the keys a tuning leaves out
        // never reach the voices.
        let (stepper, tuner) = (&mut self.stepper, &self.tuner);
        let events = events.iter().filter(|event| {
            if stepper.takes(event) {
                return false;
            }
            !matches!(event, Event::NoteOn { note, .. }
                if note.key.is_some_and(|key| tuner.offset(key).is_none()))
        });
        self.voices.process(events, frames, &mut self.synth);

        // Down to the host's rate.
        let stage = factor.trailing_zeros() as usize;
        let oversampler = &mut self.oversamplers[stage];
        let channels = audio.channels();
        for c in 0..channels.min(2) {
            let high = &mut self.synth.out[c][..frames * factor];
            let out = audio.channel_mut(c);
            oversampler.decimate(c, high, &mut out[..frames]);
        }
        self.report(cx, frames);
        let sleeping =
            self.voices.block().active_count() == 0 && self.quiet + frames >= SETTLE_FRAMES;
        self.publish_telemetry(audio, frames, sleeping);
        if self.synth.factor != factor {
            // The quality changed in this block: the new rate's way down
            // starts clean.
            let stage = self.synth.factor.trailing_zeros() as usize;
            self.oversamplers[stage].reset();
        }
        if self.voices.block().active_count() > 0 {
            self.quiet = 0;
            ProcessStatus::Continue
        } else {
            self.quiet += frames;
            if self.quiet >= SETTLE_FRAMES {
                ProcessStatus::Sleep
            } else {
                ProcessStatus::Continue
            }
        }
    }

    /// Takes the block's output for the scope, the spectrum and the meters,
    /// and publishes a frame when one is due, or when the engine is about to
    /// sleep, so the interface shows the silence it fell into.
    fn publish_telemetry(&mut self, audio: &Buffers<'_>, frames: usize, sleeping: bool) {
        let Some(t) = self.telemetry.as_mut() else {
            return;
        };
        if !t.publisher.watched() {
            return;
        }
        let metered = audio.channels().min(telemetry::METERED);
        let mono = &mut t.mono[..frames];
        mono.fill(0.0);
        for c in 0..metered {
            for (m, &s) in mono.iter_mut().zip(audio.channel(c)) {
                *m += s / metered as f32;
                t.peaks[c] = t.peaks[c].max(s.abs());
            }
        }
        t.analyser.push(mono);
        if !t.publisher.due(frames) && !sleeping {
            return;
        }
        let (synth, voices) = (&self.synth, &self.voices);
        t.publisher.publish(|frame| {
            debug_assert_eq!(frame.layout, telemetry::layout(&synth.registry));
            frame.clock = synth.clock;
            frame.set_modulated(&synth.modeng);
            frame.active.fill(false);
            for v in voices.block().active_voices() {
                frame.active[v] = true;
            }
            let osc = &synth.units[0][synth.copies[0] - 1].osc;
            for (v, phase) in frame.voice_phase.iter_mut().enumerate() {
                *phase =
                    (f64::from(osc.phase(osc.lanes_of(v).start)) / lspk_wavetable::CYCLE) as f32;
            }
            for (l, phases) in frame
                .lfo_phase
                .as_chunks_mut::<MAX_VOICES>()
                .0
                .iter_mut()
                .enumerate()
            {
                for (v, phase) in phases.iter_mut().enumerate() {
                    // An LFO nothing reads isn't running.
                    *phase = if synth.needed & (1 << l) != 0 {
                        synth.lfos[l].phase(v, &synth.transport)
                    } else {
                        0.0
                    };
                }
            }
            for (e, stages) in frame
                .env_stage
                .as_chunks_mut::<MAX_VOICES>()
                .0
                .iter_mut()
                .enumerate()
            {
                for (v, stage) in stages.iter_mut().enumerate() {
                    *stage = synth.envs[e].stage(v);
                }
            }
            frame.levels[..telemetry::METERED].copy_from_slice(&t.peaks);
            t.analyser.scope(&mut frame.scope);
            t.analyser.spectrum(&mut frame.spectrum);
        });
        t.peaks = [0.0; telemetry::METERED];
    }

    /// Reports the block's note ends and routing resets to the host, in
    /// time order, each inside the block's `frames` frames.
    fn report(&mut self, cx: &mut ProcessContext<'_>, frames: usize) {
        let last = frames.saturating_sub(1) as u32;
        self.synth.ends.sort_by_key(|(t, _)| *t);
        self.synth.resets.sort_by_key(|(t, _, _)| *t);
        let (mut e, mut r) = (0, 0);
        let (ends, resets) = (&self.synth.ends, &self.synth.resets);
        while e < ends.len() || r < resets.len() {
            let reset_first = r < resets.len() && (e >= ends.len() || resets[r].0 <= ends[e].0);
            if reset_first {
                let (time, id, value) = resets[r];
                let time = time.min(last);
                cx.begin_gesture(time, id);
                cx.set_param(time, id, value);
                cx.end_gesture(time, id);
                r += 1;
            } else {
                let (time, note) = ends[e];
                cx.note_end(time.min(last), note);
                e += 1;
            }
        }
        self.synth.ends.clear();
        self.synth.resets.clear();
    }
}

impl Processor for Engine {
    fn process(
        &mut self,
        audio: &mut Buffers<'_>,
        events: &Events<'_>,
        cx: &mut ProcessContext<'_>,
    ) -> ProcessStatus {
        let frames = audio.frames();
        if frames > self.max_frames {
            // Never asked for by a host that keeps to its maximum; split.
            audio.silence();
            return ProcessStatus::Continue;
        }
        self.synth.set_host_transport(cx.transport());
        let has_events = events.iter().next().is_some();
        if !has_events && self.quiet >= SETTLE_FRAMES && self.voices.block().active_count() == 0 {
            // Asleep: nothing sounds and nothing arrives.
            audio.silence();
            self.synth.advance_idle(frames);
            return ProcessStatus::Sleep;
        }
        self.render_block(audio, events, cx)
    }

    fn flush(&mut self, events: &Events<'_>, _cx: &mut ProcessContext<'_>) {
        for event in events.iter() {
            if let Event::Param { id, value, .. } = event {
                self.synth.set_param(id, value, 0);
            }
        }
        self.synth.rebuild(self.voices.settings_mut());
    }

    fn reset(&mut self) {
        self.voices.reset();
        for e in &mut self.synth.envs {
            for v in 0..MAX_VOICES {
                e.reset(v);
            }
        }
        for f in &mut self.synth.filters {
            f.reset();
        }
        self.synth.sub.reset();
        self.synth.noise.reset();
        for o in &mut self.oversamplers {
            o.reset();
        }
        for v in 0..MAX_VOICES {
            self.synth.modeng.voice_ended(v);
        }
        for ring in &mut self.synth.audio_ring {
            ring.fill(0.0);
        }
        self.quiet = SETTLE_FRAMES;
    }
}

/// Gives every envelope the timing it follows now: this engine's rate, the
/// tempo of the host or of the engine's own clock, and the global envelope
/// scale (engine-core R75, R80).
fn retime_envelopes(envs: &mut [EnvelopeBank], rate: f64, tempo: f64, scale: f64) {
    let timing = Timing::new(rate).with_tempo(tempo).with_scale(scale);
    for env in envs {
        env.set_timing(timing);
    }
}

impl Synth {
    /// Takes the patch the plug-in's main thread compiled, if one waits,
    /// and makes it the program this thread runs. The one it replaces goes
    /// back to the sender, which drops it: the audio thread never frees.
    /// The modules' state — the voices, the phases, the filters' and the
    /// effects' tails, the envelopes and the LFOs — is untouched, so the
    /// swap is inaudible.
    fn begin_block(&mut self) {
        let Some(mailbox) = self.patch_mailbox.as_mut() else {
            return;
        };
        if let Some(patch) = mailbox.take() {
            self.patch = patch;
        }
    }

    /// Keeps time while asleep.
    fn advance_idle(&mut self, frames: usize) {
        self.transport.advance(frames);
    }

    /// Takes the host's transport for this block: a synced LFO follows its
    /// tempo (engine-core R85), a song-locked one its position (R84), and a
    /// tempo-synced envelope its tempo (R75). Without a host transport the
    /// engine keeps its own clock.
    fn set_host_transport(&mut self, host: Option<&lspk_plugin::Transport>) {
        let Some(host) = host else {
            return;
        };
        let tempo = host.tempo;
        self.transport.tempo = tempo;
        self.transport.beats_per_bar = host.beats_per_bar;
        self.transport.playing = host.playing;
        self.transport.beats = host.beats;
        self.transport.seconds = host.seconds;
        if tempo != self.env_tempo {
            self.env_tempo = tempo;
            retime_envelopes(&mut self.envs, self.rate, tempo, self.env_scale);
        }
    }

    /// Takes parameter `id`'s new value at frame `time` of the block.
    fn set_param(&mut self, id: ParamId, value: f64, time: u32) {
        let Some(i) = self.values.index(id) else {
            return;
        };
        self.values.values[i] = value;
        self.dirty |= dirty_bit(id);
        if let Some(dest) = self.values.dests[i] {
            let position = self.values.descriptors[i].normalize(value) as f32;
            self.modeng.set_base(dest, position);
        }
        if let Some(m) = (0..MACROS).find(|&m| global::macro_id(m) == id) {
            self.modeng
                .set_held(self.src.macros[m], (value / 100.0) as f32);
        }
        self.resolve_loops(id, time);
    }

    /// Keeps filters and buses from routing into each other: the newer
    /// route wins and the earlier goes to Main (engine-core R65, R68).
    fn resolve_loops(&mut self, id: ParamId, time: u32) {
        let Some(a) = ParamAddress::of(id) else {
            return;
        };
        let (module, param, other_value, main) = match (a.module, a.param) {
            (FILTER, filter::OUTPUT) => (
                FILTER,
                filter::OUTPUT,
                routing::FILTER_OTHER,
                routing::FILTER_MAIN,
            ),
            (BUS, bus::OUTPUT) => (BUS, bus::OUTPUT, routing::BUS_OTHER, routing::BUS_MAIN),
            _ => return,
        };
        let ids = [params::id(module, 0, param), params::id(module, 1, param)];
        let points = [
            self.values.get(ids[0]).round() == other_value,
            self.values.get(ids[1]).round() == other_value,
        ];
        if let Some(reset) = routing::resolve_loop(points, usize::from(a.instance)) {
            let reset_id = ids[reset];
            if let Some(i) = self.values.index(reset_id) {
                self.values.values[i] = main;
                self.dirty |= dirty_bit(reset_id);
            }
            if self.resets.len() < REPORTS {
                self.resets.push((time, reset_id, main));
            }
        }
    }

    /// Rebuilds every module whose parameters changed.
    fn rebuild(&mut self, settings: &mut Settings) {
        if self.dirty == 0 {
            return;
        }
        let dirty = std::mem::take(&mut self.dirty);
        let v = &self.values;
        let f32_of = |id: ParamId| v.get(id) as f32;
        let mut any_channel = false;
        for s in 0..CHANNELS {
            let bit = match s {
                0..=2 => dirty::OSC << s,
                3 => dirty::SUB,
                _ => dirty::NOISE,
            };
            if dirty & bit == 0 {
                continue;
            }
            any_channel = true;
            self.channels[s] = ChannelState::of(s, &|id| v.get(id));
            let (module, instance) = match s {
                0..=2 => (OSC, s as u8),
                3 => (SUB, 0),
                _ => (NOISE, 0),
            };
            let p = |param: u16| params::id(module, instance, param);
            if s < OSCILLATORS {
                let frames = self.table_frames.max(1) as f32;
                let params = lspk_wavetable::osc::Params {
                    octave: v.get(p(osc::OCTAVE)).round() as i32,
                    semitone: v.get(p(osc::SEMITONE)).round() as i32,
                    fine: f32_of(p(osc::FINE)),
                    coarse: f32_of(p(osc::SWEEP)),
                    frame: 1.0 + f32_of(p(osc::FRAME)) / 100.0 * (frames - 1.0),
                    smooth: v.on(p(osc::SMOOTH)),
                    phase: f32_of(p(osc::PHASE)),
                    phase_random: f32_of(p(osc::PHASE_RANDOM)),
                    phase_mode: match v.get(p(osc::PHASE_MODE)).round() as i64 {
                        1 => PhaseMode::Continue,
                        2 => PhaseMode::PerVoice,
                        _ => PhaseMode::NoteStart,
                    },
                    phase_legato: v.on(p(osc::PHASE_LEGATO)),
                    key_track: v.on(p(osc::KEY_TRACK)),
                    // The oscillator's own level fades without clicks; a
                    // modulated level is applied in the mixer instead.
                    level: f32_of(p(channel::LEVEL)),
                    reference: self.reference,
                };
                self.osc_params[s] = params;
                let copies = (v.get(p(osc::UNISON)).round() as usize).clamp(1, MAX_COPIES);
                let unison = Unison {
                    copies,
                    detune: f32_of(p(osc::DETUNE)),
                    blend: f32_of(p(osc::UNISON_MIX)),
                    law: DetuneLaw::ALL[(v.get(p(osc::DETUNE_LAW)).round().max(0.0) as usize)
                        .min(DetuneLaw::ALL.len() - 1)],
                    stack: Stack::ALL[(v.get(p(osc::STACK)).round().max(0.0) as usize)
                        .min(Stack::ALL.len() - 1)],
                    width: f32_of(p(osc::WIDTH)),
                    range: f32_of(p(osc::DETUNE_RANGE)),
                    frame_spread: f32_of(p(osc::FRAME_SPREAD)),
                    ..Unison::default()
                };
                if copies != self.copies[s] {
                    self.copies[s] = copies;
                    self.restart[s] = true;
                }
                self.unison[s] = unison;
                let unit = &mut self.units[s][copies - 1];
                *unit.osc.params_mut() = params;
                unit.lanes.set(unison);
                let count = unit.osc.lane_count();
                unit.osc
                    .set_unison(0..count, unit.lanes.pitch(), unit.lanes.frame());
            } else if s == OSCILLATORS {
                let shape = match v.get(p(sub::SHAPE)).round().max(0.0) as usize {
                    1 => SubShape::RoundedSquare,
                    2 => SubShape::Triangle,
                    3 => SubShape::Saw,
                    4 => SubShape::Square,
                    5 => SubShape::Pulse,
                    _ => SubShape::Sine,
                };
                self.sub_params = lspk_wavetable::sub::Params {
                    octave: v.get(p(sub::OCTAVE)).round() as i32,
                    sweep: f32_of(p(sub::SWEEP)),
                    shape,
                    phase: f32_of(p(sub::PHASE)),
                    continue_phase: v.on(p(sub::CONTINUE_PHASE)),
                    wrap_phase: v.on(p(sub::WRAP_PHASE)),
                    key_track: v.on(p(sub::KEY_TRACK)),
                    level: f32_of(p(channel::LEVEL)),
                    reference: self.reference,
                };
                *self.sub.params_mut() = self.sub_params;
            } else {
                self.noise_params = lspk_wavetable::noise::Params {
                    looped: v.get(p(noise::LOOP)).round() >= 1.0,
                    start: f32_of(p(noise::START)),
                    start_random: f32_of(p(noise::START_RANDOM)),
                    rate: f32_of(p(noise::RATE)),
                    fine: f32_of(p(noise::FINE)),
                    key_track: v.on(p(noise::KEY_TRACK)),
                    stereo: f32_of(p(noise::STEREO)),
                    tone: f32_of(p(noise::TONE)),
                    level: f32_of(p(channel::LEVEL)),
                    reference: self.reference,
                };
                *self.noise.params_mut() = self.noise_params;
                self.noise_source = (v.get(p(noise::SOURCE)).round().max(0.0) as usize)
                    .min(params::NOISE_SOURCES.len() - 1);
            }
        }
        for f in 0..FILTERS {
            if dirty & (dirty::FILTER << f) == 0 {
                continue;
            }
            any_channel = true;
            let p = |param: u16| params::id(FILTER, f as u8, param);
            let kind = filter_type(v.get(p(filter::TYPE)));
            let state = FilterState::of(f, &|id| v.get(id));
            let bank = &mut self.filters[f];
            bank.set_type(kind);
            bank.set_clean_drive(state.clean);
            for voice in 0..MAX_VOICES {
                bank.set_params(voice, &state.voice);
            }
            self.filter_state[f] = state;
        }
        if dirty & dirty::BUS != 0 {
            for b in 0..BUSES {
                let p = |param: u16| params::id(BUS, b as u8, param);
                self.buses[b] = BusState {
                    level: f32_of(p(bus::LEVEL)),
                    output: BusOutput::from_index(v.get(p(bus::OUTPUT))),
                };
            }
        }
        for e in 0..ENVELOPES {
            if dirty & (dirty::ENV << e) == 0 {
                continue;
            }
            let p = |param: u16| params::env_id(e, param);
            let sustain = if e == 0 {
                f32_of(p(env::SUSTAIN))
            } else {
                f32_of(p(env::SUSTAIN)) / 100.0
            };
            let params = EnvelopeParams {
                unit: TimeUnit::Milliseconds,
                delay: 0.0,
                attack: f32_of(p(env::ATTACK)),
                hold: f32_of(p(env::HOLD)),
                decay: f32_of(p(env::DECAY)),
                sustain,
                release: f32_of(p(env::RELEASE)),
                attack_curve: f32_of(p(env::ATTACK_CURVE)) / 100.0,
                decay_curve: f32_of(p(env::DECAY_CURVE)) / 100.0,
                release_curve: f32_of(p(env::RELEASE_CURVE)) / 100.0,
                start: if e == 0 { 0.0 } else { f32_of(p(env::START)) },
                end: if e == 0 { 0.0 } else { f32_of(p(env::END)) },
                legato_inverted: v.on(p(env::LEGATO_INVERTED)),
                steal_restart: if v.get(p(env::STEAL_RESTART)).round() >= 1.0 {
                    StealRestart::StartLevel
                } else {
                    StealRestart::CurrentLevel
                },
            };
            self.env_params[e] = params;
            self.envs[e].set_all_params(params);
        }
        for l in 0..LFOS {
            if dirty & (dirty::LFO << l) == 0 {
                continue;
            }
            let p = |param: u16| params::id(LFO, l as u8, param);
            let x10 = v.on(p(lfo::X10));
            let settings = LfoSettings {
                kind: match v.get(p(lfo::TYPE)).round() as i64 {
                    1 => LfoType::Path,
                    2 => LfoType::Lorenz,
                    3 => LfoType::Rossler,
                    4 => LfoType::SampleAndHold,
                    _ => LfoType::Drawn,
                },
                mode: match v.get(p(lfo::MODE)).round() as i64 {
                    0 => Mode::Free,
                    2 => Mode::OneShot,
                    _ => Mode::Restart,
                },
                shared: v.on(p(lfo::SHARED)),
                direction: match v.get(p(lfo::DIRECTION)).round() as i64 {
                    1 => Direction::Reverse,
                    2 => Direction::PingPong,
                    _ => Direction::Forward,
                },
                rate: if v.on(p(lfo::SYNC)) {
                    LfoRate::Synced {
                        division: Division::new(v.get(p(lfo::DIVISION)).round() as u8)
                            .unwrap_or(Division::QUARTER),
                        feel: match v.get(p(lfo::FEEL)).round() as i64 {
                            1 => Feel::Triplet,
                            2 => Feel::Dotted,
                            _ => Feel::Straight,
                        },
                    }
                } else {
                    LfoRate::Hz {
                        hz: f32_of(p(lfo::RATE)) * if x10 { 10.0 } else { 1.0 },
                        x10,
                    }
                },
                song_lock: v.on(p(lfo::SONG_LOCK)),
                swing: v.on(p(lfo::SWING)),
                fade_in: Span::from_index(v.get(p(lfo::FADE_IN)).round() as u8),
                delay: Span::from_index(v.get(p(lfo::DELAY)).round() as u8),
                smooth: f32_of(p(lfo::SMOOTH)),
                phase: f32_of(p(lfo::PHASE)),
                phase_snap: v.on(p(lfo::PHASE_SNAP)),
                grid_x: 8,
                grid_y: 8,
            };
            self.lfos[l].lfo.settings = settings;
        }
        if dirty & dirty::GLOBAL != 0 {
            self.main_level = f32_of(global::MAIN_LEVEL);
            self.dry_level = f32_of(global::DRY_LEVEL);
            self.amp = f32_of(global::AMP);
            self.tuning = f32_of(global::TUNING);
            let factor = quality_factor(v.get(global::QUALITY));
            if factor != self.factor {
                self.factor = factor;
                let os_rate = (self.rate * factor as f64) as f32;
                for unit in self.units.iter_mut().flatten() {
                    unit.osc.set_sample_rate(os_rate);
                }
                for f in &mut self.filters {
                    f.set_sample_rate(os_rate);
                }
                self.sub.set_sample_rate(os_rate);
                self.noise.set_sample_rate(os_rate);
            }
            let reference = f32_of(global::REFERENCE_PITCH);
            if reference != self.reference {
                self.reference = reference;
                for s in 0..OSCILLATORS {
                    self.osc_params[s].reference = reference;
                }
                for unit in self.units.iter_mut().flatten() {
                    unit.osc.params_mut().reference = reference;
                }
                self.sub_params.reference = reference;
                self.noise_params.reference = reference;
                self.sub.params_mut().reference = reference;
                self.noise.params_mut().reference = reference;
            }
            let env_scale = f64::from(f32_of(global::ENV_SCALE)) / 100.0;
            if env_scale != self.env_scale {
                self.env_scale = env_scale;
                self.env_tempo = self.transport.tempo;
                retime_envelopes(&mut self.envs, self.rate, self.transport.tempo, env_scale);
            }
            let lfo_scale = f32_of(global::LFO_SCALE);
            self.transport.scale = lspk_dsp::lfo::Scale::Percent(lfo_scale);
        }
        if dirty & dirty::VOICE != 0 {
            let steal = match v.get(global::STEAL).round() as i64 {
                1 => StealPriority::Oldest,
                2 => StealPriority::Highest,
                3 => StealPriority::Lowest,
                4 => StealPriority::Velocity,
                _ => StealPriority::Newest,
            };
            *settings = Settings {
                voices: (v.get(global::VOICES).round() as usize).clamp(1, MAX_VOICES),
                steal,
                same_note: match v.get(global::SAME_NOTE).round() as i64 {
                    1 => SameNote::Sustain,
                    2 => SameNote::On,
                    _ => SameNote::Off,
                },
                mono: v.on(global::MONO),
                legato: v.on(global::LEGATO),
                velocity_legato: v.on(global::VELOCITY_LEGATO),
                glide: Glide {
                    time: f32_of(global::GLIDE_TIME) / 1000.0,
                    curve: f32_of(global::GLIDE_CURVE) / 100.0,
                    always: v.on(global::GLIDE_ALWAYS),
                    scaled: v.on(global::GLIDE_SCALED),
                },
                bend_up: f32_of(global::BEND_UP),
                bend_down: f32_of(global::BEND_DOWN),
                mpe: Mpe {
                    enabled: v.on(global::MPE),
                    bend_range: f32_of(global::MPE_BEND),
                },
                ..*settings
            };
            self.modeng.set_voice_limit(settings.voices);
        }
        if any_channel {
            // What changed here can change what is in use: refresh the
            // program in place. It allocates nothing, and the modules' state
            // stays where it is, so no voice, phase or tail is disturbed.
            self.patch = Patch::compile(&self.channels, &self.filter_state);
        }
    }

    /// Renders the envelopes, the LFOs that are needed and the audio
    /// sources' delayed copies for host frames `0..n` of the stretch, and
    /// runs the matrix over them.
    fn render_modulation(&mut self, voices: &VoiceBlock, n: usize) {
        for (e, bank) in self.envs.iter_mut().enumerate() {
            bank.process(&mut self.env_buf[e][..n * S]);
        }
        for l in 0..LFOS {
            if self.needed & (1 << l) != 0 {
                self.lfos[l].process(
                    &self.transport,
                    n,
                    &mut self.lfo_buf[l][..n * S],
                    Some(&mut self.lfo_y_buf[l][..n * S]),
                );
            }
        }
        // The audio sources, one period late.
        let phase = (self.clock % CHUNK as u64) as usize;
        let period = (self.clock / CHUNK as u64) as usize;
        let prev = (period + 1) % 2;
        for a in 0..AUDIO_SOURCES.len() {
            if self.needed & audio_bit(a) == 0 {
                continue;
            }
            let ring = &self.audio_ring[a][prev * CHUNK * S..(prev + 1) * CHUNK * S];
            self.audio_buf[a][..n * S].copy_from_slice(&ring[phase * S..(phase + n) * S]);
        }
        let c = voices.controllers();
        let src = &self.src;
        self.modeng.set_held(src.mod_wheel, c.cc(1));
        self.modeng.set_held(src.pitch_bend, c.pitch_bend);
        self.modeng
            .set_held(src.channel_pressure, c.channel_pressure);
        for v in voices.active_voices() {
            let pressure = voices.pressure()[v];
            self.modeng.set_voice_held(src.poly_pressure, v, pressure);
            self.modeng.set_voice_held(src.mpe_z, v, pressure);
            self.modeng.set_voice_held(src.mpe_y, v, voices.timbre()[v]);
        }
        let mut signals = Signals::new();
        for e in 0..ENVELOPES {
            signals.bind(src.env[e], &self.env_buf[e][..n * S], S);
        }
        for l in 0..LFOS {
            if self.needed & (1 << l) != 0 {
                signals.bind(src.lfo[l], &self.lfo_buf[l][..n * S], S);
                signals.bind(src.lfo_y[l], &self.lfo_y_buf[l][..n * S], S);
            }
        }
        for a in 0..AUDIO_SOURCES.len() {
            if self.needed & audio_bit(a) != 0 {
                signals.bind(src.audio[a], &self.audio_buf[a][..n * S], S);
            }
        }
        self.modeng.process(&signals, 0..n);
    }

    /// Applies the matrix's control-rate results to the modules for the
    /// stretch: filter settings, envelope times, LFO rates, point buses
    /// and glide.
    fn apply_control(&mut self, voices: &mut VoiceBlock) {
        // Filters.
        for f in 0..FILTERS {
            let p = |param: u16| params::id(FILTER, f as u8, param);
            let mods = [
                filter::RESONANCE,
                filter::DRIVE,
                filter::CHARACTER,
                filter::STEREO,
                filter::MIX,
                filter::LEVEL,
                filter::MORPH_X,
                filter::MORPH_Y,
            ];
            let any = mods
                .iter()
                .any(|&m| modulation(&self.modeng, self.values.dest(p(m))).is_some());
            if !any {
                continue;
            }
            let base = self.filter_state[f].voice;
            for v in voices.active_voices() {
                let get = |m: u16, base: f32, scale: f32| {
                    modulation(&self.modeng, self.values.dest(p(m)))
                        .at(0, v)
                        .map_or(base, |pos| self.values.real(p(m), pos) * scale)
                };
                let params = VoiceParams {
                    resonance: get(filter::RESONANCE, base.resonance, 0.01),
                    drive: get(filter::DRIVE, base.drive, 0.01),
                    saturation: get(filter::CHARACTER, base.saturation, 0.01),
                    character: get(filter::CHARACTER, base.character, 0.01),
                    stereo_offset: modulation(&self.modeng, self.values.dest(p(filter::STEREO)))
                        .at(0, v)
                        .map_or(base.stereo_offset, |pos| {
                            (self.values.real(p(filter::STEREO), pos) - 50.0) / 50.0 * 12.0
                        }),
                    mix: get(filter::MIX, base.mix, 0.01),
                    level: get(filter::LEVEL, base.level, 1.0),
                    morph_x: get(filter::MORPH_X, base.morph_x, 1.0),
                    morph_y: get(filter::MORPH_Y, base.morph_y, 1.0),
                };
                self.filters[f].set_params(v, &params);
            }
        }
        // Envelopes.
        for e in 0..ENVELOPES {
            let p = |param: u16| params::env_id(e, param);
            let times = [
                env::ATTACK,
                env::HOLD,
                env::DECAY,
                env::SUSTAIN,
                env::RELEASE,
                env::ATTACK_CURVE,
                env::DECAY_CURVE,
                env::RELEASE_CURVE,
                env::START,
                env::END,
            ];
            let modulated = times
                .iter()
                .any(|&m| modulation(&self.modeng, self.values.dest(p(m))).is_some());
            if !modulated {
                continue;
            }
            let base = self.env_params[e];
            for v in voices.active_voices() {
                let get = |m: u16, base: f32, scale: f32| {
                    modulation(&self.modeng, self.values.dest(p(m)))
                        .at(0, v)
                        .map_or(base, |pos| self.values.real(p(m), pos) * scale)
                };
                let sustain_scale = if e == 0 { 1.0 } else { 0.01 };
                let params = EnvelopeParams {
                    attack: get(env::ATTACK, base.attack, 1.0),
                    hold: get(env::HOLD, base.hold, 1.0),
                    decay: get(env::DECAY, base.decay, 1.0),
                    sustain: get(env::SUSTAIN, base.sustain, sustain_scale),
                    release: get(env::RELEASE, base.release, 1.0),
                    attack_curve: get(env::ATTACK_CURVE, base.attack_curve, 0.01),
                    decay_curve: get(env::DECAY_CURVE, base.decay_curve, 0.01),
                    release_curve: get(env::RELEASE_CURVE, base.release_curve, 0.01),
                    start: if e == 0 {
                        0.0
                    } else {
                        get(env::START, base.start, 1.0)
                    },
                    end: if e == 0 {
                        0.0
                    } else {
                        get(env::END, base.end, 1.0)
                    },
                    ..base
                };
                if *self.envs[e].params(v) != params {
                    self.envs[e].set_params(v, params);
                }
            }
        }
        // LFO rates, smoothing and phase, shared by the LFO's voices.
        for l in 0..LFOS {
            if self.needed & (1 << l) == 0 {
                continue;
            }
            let p = |param: u16| params::id(LFO, l as u8, param);
            let mut settings = self.lfos[l].lfo.settings;
            let base_rate = self.values.get(p(lfo::RATE)) as f32;
            if let Some(pos) = modulation(&self.modeng, self.values.dest(p(lfo::RATE))).at(0, 0)
                && let LfoRate::Hz { x10, .. } = settings.rate
            {
                let hz = self.values.real(p(lfo::RATE), pos);
                settings.rate = LfoRate::Hz {
                    hz: hz * if x10 { 10.0 } else { 1.0 },
                    x10,
                };
            } else if let LfoRate::Hz { x10, .. } = settings.rate {
                settings.rate = LfoRate::Hz {
                    hz: base_rate * if x10 { 10.0 } else { 1.0 },
                    x10,
                };
            }
            settings.smooth = modulation(&self.modeng, self.values.dest(p(lfo::SMOOTH)))
                .at(0, 0)
                .map_or(self.values.get(p(lfo::SMOOTH)) as f32, |pos| {
                    self.values.real(p(lfo::SMOOTH), pos)
                });
            settings.phase = modulation(&self.modeng, self.values.dest(p(lfo::PHASE)))
                .at(0, 0)
                .map_or(self.values.get(p(lfo::PHASE)) as f32, |pos| {
                    self.values.real(p(lfo::PHASE), pos)
                });
            self.lfos[l].lfo.settings = settings;
            // Point buses.
            for b in 0..lspk_dsp::lfo::POINT_BUSES {
                let dest = self.registry.dest(Target::LfoBus {
                    lfo: l as u8,
                    bus: b as u8,
                });
                let m = modulation(&self.modeng, dest);
                if !m.is_some() {
                    continue;
                }
                let values: [Option<f32>; S] = std::array::from_fn(|v| m.at(0, v));
                for v in voices.active_voices() {
                    if let Some(value) = values[v] {
                        self.lfos[l].voice_mut(v).set_bus(b, value);
                    }
                }
            }
        }
        // Unison, shared by each oscillator's voices.
        for s in 0..OSCILLATORS {
            let p = |param: u16| params::id(OSC, s as u8, param);
            let controls = [
                osc::DETUNE,
                osc::UNISON_MIX,
                osc::WIDTH,
                osc::DETUNE_RANGE,
                osc::FRAME_SPREAD,
            ];
            let mods = controls.map(|c| modulation(&self.modeng, self.values.dest(p(c))).at(0, 0));
            if mods.iter().all(Option::is_none) {
                continue;
            }
            let value = |i: usize, base: f32| {
                mods[i].map_or(base, |pos| self.values.real(p(controls[i]), pos))
            };
            let base = self.unison[s];
            let unison = Unison {
                detune: value(0, base.detune),
                blend: value(1, base.blend),
                width: value(2, base.width),
                range: value(3, base.range),
                frame_spread: value(4, base.frame_spread),
                ..base
            };
            let unit = &mut self.units[s][self.copies[s] - 1];
            if *unit.lanes.unison() != unison {
                unit.lanes.set(unison);
                let count = unit.osc.lane_count();
                unit.osc
                    .set_unison(0..count, unit.lanes.pitch(), unit.lanes.frame());
            }
        }
        // Glide.
        let time = modulation(&self.modeng, self.values.dest(global::GLIDE_TIME)).at(0, 0);
        let curve = modulation(&self.modeng, self.values.dest(global::GLIDE_CURVE)).at(0, 0);
        if time.is_some() || curve.is_some() {
            let glide = &mut voices.settings_mut().glide;
            if let Some(pos) = time {
                glide.time = self.values.real(global::GLIDE_TIME, pos) / 1000.0;
            }
            if let Some(pos) = curve {
                glide.curve = self.values.real(global::GLIDE_CURVE, pos) / 100.0;
            }
        }
    }

    /// Renders the audio path for host frames `0..n` of the stretch at the
    /// quality's rate, into the block's output from frame `at`.
    // The loops index several per-voice buffers in step, which reads more
    // plainly by index than zipped.
    #[allow(clippy::too_many_lines, clippy::needless_range_loop)]
    fn render_audio(&mut self, voices: &VoiceBlock, n: usize) {
        let k = self.factor;
        let os_n = n * k;
        let span = voices.span(1);
        let at = self.block_frame;
        for vp in &mut self.vpath {
            vp[..os_n * 2 * S].fill(0.0);
        }
        for fin in &mut self.fin {
            fin[..os_n * 2 * S].fill(0.0);
        }
        let mut used = [false; 4];
        let active = voices.active();

        // Each voice's Amp, per host frame.
        let amp_mod = modulation(&self.modeng, self.values.dest(global::AMP));
        let tuning_mod = modulation(&self.modeng, self.values.dest(global::TUNING));
        for t in 0..n {
            for v in 0..span {
                let amp = amp_mod
                    .at(t, v)
                    .map_or(self.amp, |pos| self.values.real(global::AMP, pos));
                self.amp_buf[t * S + v] = amp * active[v];
            }
        }

        // What this stretch runs: the program's modules, in its order.
        let patch = self.patch;
        for &s in patch.channels() {
            let s = usize::from(s);
            let ch = self.channels[s];
            if span == 0 {
                continue;
            }
            let p = |param: u16| match s {
                0..=2 => params::id(OSC, s as u8, param),
                3 => params::id(SUB, 0, param),
                _ => params::id(NOISE, 0, param),
            };
            let level_mod = modulation(&self.modeng, self.values.dest(p(channel::LEVEL)));
            let pan_mod = modulation(&self.modeng, self.values.dest(p(channel::PAN)));
            if s < OSCILLATORS {
                let copies = self.copies[s];
                let lane_count = self.units[s][copies - 1].osc.lane_count();
                let lanes = span * copies;
                // Each copy's lane takes its voice's pitch, and glide's step at
                // the quality's rate.
                let pitch = self.tuned;
                let step = voices.pitch_step();
                for v in 0..span {
                    for i in 0..copies {
                        self.lane_pitch[v * copies + i] = pitch[v];
                        self.pitch_step[v * copies + i] = step[v] / k as f32;
                    }
                }
                // Audio-rate modulation of the oscillator's tuning and of main
                // tuning, in semitones.
                let pitch_mods = [osc::OCTAVE, osc::SEMITONE, osc::FINE, osc::SWEEP]
                    .map(|m| (m, modulation(&self.modeng, self.values.dest(p(m)))));
                let any_pitch = pitch_mods.iter().any(|(_, m)| m.is_some())
                    || tuning_mod.is_some()
                    || self.tuning != 0.0;
                if any_pitch {
                    let base = self.osc_params[s].tune();
                    for t in 0..n {
                        for v in 0..span {
                            let mut offset = tuning_mod.at(t, v).map_or(self.tuning, |pos| {
                                // Main tuning is unbounded (R137).
                                -64.0 + 128.0 * pos
                            });
                            let mut tune = base;
                            for &(m, ref modu) in &pitch_mods {
                                if let Some(pos) = modu.at(t, v) {
                                    let real = self.values.real(p(m), pos);
                                    let base_real = self.values.get(p(m)) as f32;
                                    let (real, scale) = match m {
                                        osc::OCTAVE => (real.round(), 12.0),
                                        osc::SEMITONE => (real.round(), 1.0),
                                        osc::FINE => (real, 0.01),
                                        _ => (real, 1.0),
                                    };
                                    tune += (real - base_real) * scale;
                                }
                            }
                            offset += tune - base;
                            for i in 0..k {
                                let row = (t * k + i) * lane_count + v * copies;
                                self.pitch_mod[row..row + copies].fill(offset);
                            }
                        }
                    }
                }
                // Frame, read once a host frame: at control rate its route's
                // ramp between ticks, so the frame never steps.
                let frame_dest = self.values.dest(p(osc::FRAME));
                let has_frame = modulation(&self.modeng, frame_dest).is_some();
                if has_frame {
                    fill_ramp(&self.modeng, &mut self.ramp_buf, frame_dest, n, span);
                    let base = self.values.get(p(osc::FRAME)) as f32;
                    let frames = self.table_frames.max(1) as f32 - 1.0;
                    for j in 0..os_n {
                        for v in 0..span {
                            let pos = self.ramp_buf[(j / k) * S + v];
                            let real = self.values.real(p(osc::FRAME), pos);
                            let row = j * lane_count + v * copies;
                            self.frame_mod[row..row + copies].fill((real - base) / 100.0 * frames);
                        }
                    }
                }
                let inputs = Inputs {
                    lanes,
                    pitch: &self.lane_pitch,
                    pitch_step: Some(&self.pitch_step),
                    pitch_mod: any_pitch.then_some(&self.pitch_mod[..os_n * lane_count]),
                    frame_mod: has_frame.then_some(&self.frame_mod[..os_n * lane_count]),
                };
                {
                    let unit = &mut self.units[s][copies - 1];
                    // The oscillator's own level fades without clicks; a
                    // modulated level is applied below instead.
                    unit.osc.params_mut().level =
                        if level_mod.is_some() { 100.0 } else { ch.level };
                    unit.osc
                    // Always the SIMD kernel: the oscillator's scalar path for
                    // a few lanes reads the table differently, and which path a
                    // lane count takes depends on the CPU's vector width, so
                    // renders would differ between platforms.
                    .process_at(
                        lspk_dsp::simd::level(),
                        os_n,
                        &inputs,
                        &mut self.osc_out[..os_n * lane_count],
                    );
                    self.uni_out[..os_n * 2 * S].fill(0.0);
                    unit.lanes.mix(
                        os_n,
                        span,
                        &self.osc_out[..os_n * lane_count],
                        &mut self.uni_out[..os_n * 2 * S],
                    );
                }
            } else if s == OSCILLATORS {
                // The sub: one lane per voice (R40), its pitch and phase
                // modulated at audio rate (R42).
                let pitch = self.tuned;
                let step = voices.pitch_step();
                for v in 0..span {
                    self.lane_pitch[v] = pitch[v];
                    self.pitch_step[v] = step[v] / k as f32;
                }
                let pitch_mods = [sub::OCTAVE, sub::SWEEP]
                    .map(|m| (m, modulation(&self.modeng, self.values.dest(p(m)))));
                let phase_dest = self.values.dest(p(sub::PHASE));
                let has_phase = modulation(&self.modeng, phase_dest).is_some();
                let any_pitch = pitch_mods.iter().any(|(_, m)| m.is_some())
                    || tuning_mod.is_some()
                    || self.tuning != 0.0;
                if any_pitch {
                    let base = self.sub_params.tune();
                    for t in 0..n {
                        for v in 0..span {
                            let mut offset = tuning_mod.at(t, v).map_or(self.tuning, |pos| {
                                // Main tuning is unbounded (R137).
                                -64.0 + 128.0 * pos
                            });
                            let mut tune = base;
                            for &(m, ref modu) in &pitch_mods {
                                if let Some(pos) = modu.at(t, v) {
                                    let real = self.values.real(p(m), pos);
                                    let base_real = self.values.get(p(m)) as f32;
                                    let (real, scale) = if m == sub::OCTAVE {
                                        (real.round(), 12.0)
                                    } else {
                                        (real, 1.0)
                                    };
                                    tune += (real - base_real) * scale;
                                }
                            }
                            offset += tune - base;
                            for i in 0..k {
                                self.pitch_mod[(t * k + i) * S + v] = offset;
                            }
                        }
                    }
                }
                if has_phase {
                    fill_ramp(&self.modeng, &mut self.ramp_buf, phase_dest, n, span);
                    let base = self.values.get(p(sub::PHASE)) as f32;
                    for t in 0..n {
                        for v in 0..span {
                            let offset =
                                (self.values.real(p(sub::PHASE), self.ramp_buf[t * S + v]) - base)
                                    / 360.0;
                            for i in 0..k {
                                self.frame_mod[(t * k + i) * S + v] = offset;
                            }
                        }
                    }
                }
                let inputs = SubInputs {
                    lanes: span,
                    pitch: &self.lane_pitch,
                    pitch_step: Some(&self.pitch_step),
                    pitch_mod: any_pitch.then_some(&self.pitch_mod[..os_n * S]),
                    phase_mod: has_phase.then_some(&self.frame_mod[..os_n * S]),
                };
                let mut params = self.sub_params;
                params.level = if level_mod.is_some() { 100.0 } else { ch.level };
                let sub = &mut self.sub;
                *sub.params_mut() = params;
                sub.process(os_n, &inputs, &mut self.osc_out[..os_n * S]);
                for j in 0..os_n {
                    for v in 0..span {
                        let x = self.osc_out[j * S + v];
                        self.uni_out[j * 2 * S + v] = x;
                        self.uni_out[j * 2 * S + S + v] = x;
                    }
                }
            } else {
                // The noise: a sample player over one of Reagent's own
                // colour blocks (R45 to R49).
                let pitch = self.tuned;
                let step = voices.pitch_step();
                for v in 0..span {
                    self.lane_pitch[v] = pitch[v];
                    self.pitch_step[v] = step[v] / k as f32;
                }
                // Rate, Fine, Stereo and Tone are shared controls: a route
                // to one moves every voice together.
                let shared = |param: u16, base: f32, scale: f32| {
                    modulation(&self.modeng, self.values.dest(p(param)))
                        .at(0, 0)
                        .map_or(base, |pos| self.values.real(p(param), pos) * scale)
                };
                let start_dest = self.values.dest(p(noise::START));
                let has_start = modulation(&self.modeng, start_dest).is_some();
                if has_start {
                    fill_ramp(&self.modeng, &mut self.ramp_buf, start_dest, n, span);
                    let base = self.values.get(p(noise::START)) as f32;
                    for t in 0..n {
                        for v in 0..span {
                            let offset =
                                self.values.real(p(noise::START), self.ramp_buf[t * S + v]) - base;
                            for i in 0..k {
                                self.frame_mod[(t * k + i) * S + v] = offset;
                            }
                        }
                    }
                }
                let inputs = NoiseInputs {
                    lanes: span,
                    pitch: &self.lane_pitch,
                    pitch_step: Some(&self.pitch_step),
                    start_mod: has_start.then_some(&self.frame_mod[..os_n * S]),
                };
                let base = self.noise_params;
                let params = lspk_wavetable::noise::Params {
                    rate: shared(noise::RATE, base.rate, 1.0),
                    fine: shared(noise::FINE, base.fine, 1.0),
                    stereo: shared(noise::STEREO, base.stereo, 1.0),
                    tone: shared(noise::TONE, base.tone, 1.0),
                    level: if level_mod.is_some() {
                        100.0
                    } else {
                        base.level
                    },
                    ..base
                };
                let source = &self.colours[self.noise_source];
                let noise = &mut self.noise;
                *noise.params_mut() = params;
                noise.process(os_n, &inputs, source, &mut self.uni_out[..os_n * 2 * S]);
            }
            // The source's audio for the matrix, at the host's rate.
            if self.needed & audio_bit(s) != 0 {
                let out = &self.uni_out;
                store_ring(&mut self.audio_ring[s], self.clock, k, n, span, |j, v| {
                    0.5 * (out[j * 2 * S + v] + out[j * 2 * S + S + v])
                });
            }
            // Per host frame and voice: the level and pan gains.
            let (pl, pr) = pan_gains(ch.pan);
            for t in 0..n {
                for v in 0..span {
                    let level = level_mod.at(t, v).map_or(1.0, |pos| {
                        let n = self.values.real(p(channel::LEVEL), pos) / 100.0;
                        n * n
                    });
                    let (l, r) = pan_mod.at(t, v).map_or((pl, pr), |pos| {
                        pan_gains(self.values.real(p(channel::PAN), pos))
                    });
                    let g = level * active[v];
                    self.gain_l[t * S + v] = g * l;
                    self.gain_r[t * S + v] = g * r;
                }
            }
            let env1 = &self.env_buf[0];
            for t in 0..n {
                for v in 0..span {
                    let e = if ch.amp_env { env1[t * S + v] } else { 1.0 };
                    self.env_amp[t * S + v] = e * self.amp_buf[t * S + v];
                }
            }
            let balance_mod = modulation(&self.modeng, self.values.dest(p(channel::BALANCE)));
            let send_mods = [
                modulation(&self.modeng, self.values.dest(p(channel::SEND_1))),
                modulation(&self.modeng, self.values.dest(p(channel::SEND_2))),
            ];
            let mut shares = [[0.0f32; S]; 2];
            let mut sends = [[0.0f32; S]; 2];
            for v in 0..span {
                let balance = balance_mod
                    .at(0, v)
                    .map_or(ch.balance, |pos| self.values.real(p(channel::BALANCE), pos));
                let (a, b) = balance_shares(balance);
                shares[0][v] = a;
                shares[1][v] = b;
                for i in 0..2 {
                    let id = p(if i == 0 {
                        channel::SEND_1
                    } else {
                        channel::SEND_2
                    });
                    sends[i][v] = send_mods[i]
                        .at(0, v)
                        .map_or(ch.send[i], |pos| self.values.real(id, pos))
                        / 100.0;
                }
            }
            let send_on = [
                sends[0][..span].iter().any(|s| *s > 0.0),
                sends[1][..span].iter().any(|s| *s > 0.0),
            ];
            let out = &self.uni_out;
            match ch.output {
                ChannelOutput::Filter => {
                    for (f, fin) in self.fin.iter_mut().enumerate() {
                        if shares[f][..span].iter().all(|s| *s == 0.0) {
                            continue;
                        }
                        for j in 0..os_n {
                            let t = j / k;
                            let (row_l, row_r) = fin[j * 2 * S..(j + 1) * 2 * S].split_at_mut(S);
                            for v in 0..span {
                                let share = shares[f][v];
                                row_l[v] += out[j * 2 * S + v] * share * self.gain_l[t * S + v];
                                row_r[v] += out[j * 2 * S + S + v] * share * self.gain_r[t * S + v];
                            }
                        }
                    }
                }
                ChannelOutput::Main | ChannelOutput::Dry => {
                    let path = if ch.output == ChannelOutput::Main {
                        0
                    } else {
                        1
                    };
                    used[path] = true;
                    let vp = &mut self.vpath[path];
                    for j in 0..os_n {
                        let t = j / k;
                        let (row_l, row_r) = vp[j * 2 * S..(j + 1) * 2 * S].split_at_mut(S);
                        for v in 0..span {
                            let e = self.env_amp[t * S + v];
                            row_l[v] += out[j * 2 * S + v] * e * self.gain_l[t * S + v];
                            row_r[v] += out[j * 2 * S + S + v] * e * self.gain_r[t * S + v];
                        }
                    }
                }
                ChannelOutput::None => {}
            }
            // Sends, after Level (R11), with Env 1 if the channel takes it.
            for i in 0..2 {
                if !send_on[i] {
                    continue;
                }
                used[2 + i] = true;
                let vp = &mut self.vpath[2 + i];
                for j in 0..os_n {
                    let t = j / k;
                    let (row_l, row_r) = vp[j * 2 * S..(j + 1) * 2 * S].split_at_mut(S);
                    for v in 0..span {
                        let e = self.env_amp[t * S + v] * sends[i][v];
                        row_l[v] += out[j * 2 * S + v] * e * self.gain_l[t * S + v];
                        row_r[v] += out[j * 2 * S + S + v] * e * self.gain_r[t * S + v];
                    }
                }
            }
        }

        // The filters, in the program's order: the ones in use, feeding
        // first (engine-core R59, R65).
        for &f in patch.filters() {
            let f = usize::from(f);
            if span == 0 {
                continue;
            }
            let st = self.filter_state[f];
            let p = |param: u16| params::id(FILTER, f as u8, param);
            if st.on {
                let cutoff_mod = modulation(&self.modeng, self.values.dest(p(filter::CUTOFF)));
                let pitch = self.tuned;
                let step = voices.pitch_step();
                let ref_offset = 12.0 * (self.reference / 440.0).log2();
                // The first of A, B and C with key tracking on, glide
                // included; if none tracks, it follows the played note (R52).
                let tracking_osc = self
                    .osc_params
                    .iter()
                    .enumerate()
                    .find(|(_, p)| p.key_track);
                let tracking_tune = tracking_osc.map_or(0.0, |(_, p)| p.tune());
                let tracking_pitch_mods = tracking_osc.map(|(s, _)| {
                    let p_osc = |param: u16| params::id(OSC, s as u8, param);
                    [osc::OCTAVE, osc::SEMITONE, osc::FINE, osc::SWEEP]
                        .map(|m| (m, modulation(&self.modeng, self.values.dest(p_osc(m)))))
                });
                let tracking_any_pitch = tracking_pitch_mods
                    .as_ref()
                    .is_some_and(|mods| mods.iter().any(|(_, m)| m.is_some()))
                    || (tracking_osc.is_some() && (tuning_mod.is_some() || self.tuning != 0.0));
                for j in 0..os_n {
                    let t = j / k;
                    for v in 0..span {
                        let base = cutoff_mod.at(t, v).map_or(st.cutoff, |pos| {
                            // The travel is even in pitch: MIDI note 0 at
                            // the bottom (R51).
                            pitch_of(self.values.real(p(filter::CUTOFF), pos), 440.0)
                        });
                        let key = if st.key_track {
                            let osc_offset = if tracking_any_pitch {
                                let mut offset = tuning_mod.at(t, v).map_or(self.tuning, |pos| {
                                    // Main tuning is unbounded (R137).
                                    -64.0 + 128.0 * pos
                                });
                                let mut tune = tracking_tune;
                                if let (Some((s, _)), Some(mods)) =
                                    (tracking_osc, &tracking_pitch_mods)
                                {
                                    let p_osc = |param: u16| params::id(OSC, s as u8, param);
                                    for &(m, ref modu) in mods {
                                        if let Some(pos) = modu.at(t, v) {
                                            let real = self.values.real(p_osc(m), pos);
                                            let base_real = self.values.get(p_osc(m)) as f32;
                                            let (real, scale) = match m {
                                                osc::OCTAVE => (real.round(), 12.0),
                                                osc::SEMITONE => (real.round(), 1.0),
                                                osc::FINE => (real, 0.01),
                                                _ => (real, 1.0),
                                            };
                                            tune += (real - base_real) * scale;
                                        }
                                    }
                                }
                                offset += tune - tracking_tune;
                                tracking_tune + offset
                            } else {
                                tracking_tune
                            };
                            pitch[v] + step[v] * t as f32 + osc_offset - 60.0 + ref_offset
                        } else {
                            0.0
                        };
                        self.cutoff[j * S + v] = base + key;
                    }
                }
                self.filters[f].process_voices(
                    span,
                    &self.cutoff[..os_n * S],
                    &mut self.fin[f][..os_n * 2 * S],
                );
            }
            if self.needed & audio_bit(5 + f) != 0 {
                let fin = &self.fin[f];
                store_ring(
                    &mut self.audio_ring[5 + f],
                    self.clock,
                    k,
                    n,
                    span,
                    |j, v| 0.5 * (fin[j * 2 * S + v] + fin[j * 2 * S + S + v]),
                );
            }
            let env1 = &self.env_buf[0];
            for t in 0..n {
                for v in 0..span {
                    let e = if st.amp_env { env1[t * S + v] } else { 1.0 };
                    self.env_amp[t * S + v] = e * self.amp_buf[t * S + v];
                }
            }
            let mut sends = [[0.0f32; S]; 2];
            for i in 0..2 {
                let id = p(if i == 0 {
                    filter::SEND_1
                } else {
                    filter::SEND_2
                });
                let m = modulation(&self.modeng, self.values.dest(id));
                for v in 0..span {
                    sends[i][v] = m
                        .at(0, v)
                        .map_or(st.send[i], |pos| self.values.real(id, pos))
                        / 100.0;
                }
            }
            let (this, other) = if f == 0 {
                let (a, b) = self.fin.split_at_mut(1);
                (&a[0], &mut b[0])
            } else {
                let (a, b) = self.fin.split_at_mut(1);
                (&b[0], &mut a[0])
            };
            match st.output {
                FilterOutput::Other => {
                    // In series: the other filter takes this one's output as
                    // it is; Env 1 and Amp apply where the signal leaves
                    // the voice.
                    for i in 0..os_n * 2 * S {
                        other[i] += this[i];
                    }
                }
                FilterOutput::Main | FilterOutput::Dry => {
                    let path = if st.output == FilterOutput::Main {
                        0
                    } else {
                        1
                    };
                    used[path] = true;
                    let vp = &mut self.vpath[path];
                    for j in 0..os_n {
                        let t = j / k;
                        for c in 0..2 {
                            let row = (j * 2 + c) * S;
                            for v in 0..span {
                                vp[row + v] += this[row + v] * self.env_amp[t * S + v];
                            }
                        }
                    }
                }
                FilterOutput::None => {}
            }
            for i in 0..2 {
                if sends[i][..span].iter().all(|s| *s == 0.0) {
                    continue;
                }
                used[2 + i] = true;
                let vp = &mut self.vpath[2 + i];
                for j in 0..os_n {
                    let t = j / k;
                    for c in 0..2 {
                        let row = (j * 2 + c) * S;
                        for v in 0..span {
                            vp[row + v] += this[row + v] * self.env_amp[t * S + v] * sends[i][v];
                        }
                    }
                }
            }
        }

        // Sum the voices.
        for p in 0..4 {
            let path = &mut self.path[p];
            path[..os_n * 2].fill(0.0);
            if !used[p] {
                continue;
            }
            let vp = &self.vpath[p];
            for j in 0..os_n * 2 {
                let row = &vp[j * S..j * S + span];
                path[j] = row.iter().sum();
            }
        }

        // The buses (no effects yet: R4) into Main, Dry out or each other.
        let bus_level = |b: usize, s: &Self| {
            let id = params::id(BUS, b as u8, bus::LEVEL);
            modulation(&s.modeng, s.values.dest(id))
                .at(0, 0)
                .map_or(s.buses[b].level, |pos| s.values.real(id, pos))
        };
        let levels = [bus_level(0, self), bus_level(1, self)];
        let order = if self.buses[1].output == BusOutput::Other
            && self.buses[0].output != BusOutput::Other
        {
            [1, 0]
        } else {
            [0, 1]
        };
        for b in order {
            if !used[2 + b] {
                continue;
            }
            let target = match self.buses[b].output {
                BusOutput::Main => 0,
                BusOutput::Dry => 1,
                BusOutput::Other => 3 - b,
            };
            used[target] = true;
            let (from, to) = if 2 + b < target {
                let (a, c) = self.path.split_at_mut(target);
                (&a[2 + b], &mut c[0])
            } else {
                let (a, c) = self.path.split_at_mut(2 + b);
                (&c[0], &mut a[target])
            };
            for j in 0..os_n * 2 {
                to[j] += from[j] * levels[b];
            }
        }

        // Main and Dry out, into the block's output.
        let main = modulation(&self.modeng, self.values.dest(global::MAIN_LEVEL))
            .at(0, 0)
            .map_or(self.main_level, |pos| {
                self.values.real(global::MAIN_LEVEL, pos)
            });
        let dry = modulation(&self.modeng, self.values.dest(global::DRY_LEVEL))
            .at(0, 0)
            .map_or(self.dry_level, |pos| {
                self.values.real(global::DRY_LEVEL, pos)
            });
        let start = at * k;
        for c in 0..2 {
            let out = &mut self.out[c][start..start + os_n];
            for (j, sample) in out.iter_mut().enumerate() {
                *sample = self.path[0][j * 2 + c] * main + self.path[1][j * 2 + c] * dry;
            }
        }
    }

    // shortcut: a voice's tuning is one offset for its note, so glide between
    // two retuned keys steps the offset at the new note rather than gliding it;
    // ramp it with the glide if the step is heard.
    /// Adds each voice's tuning to its pitch. A voice's tuning is read at
    /// its start; with an MTS-ESP master and Note-on only off, it is read
    /// again for every stretch, so the master can retune a held note.
    fn retune(&mut self, voices: &VoiceBlock) {
        let pitch = voices.pitch();
        let follow = self.tuner.follows_master();
        for (v, &pitch) in pitch.iter().enumerate().take(S) {
            if follow && voices.active_mask() & (1 << v) != 0 {
                self.tune_off[v] = self
                    .tuner
                    .offset(voices.info(v).key)
                    .unwrap_or(self.tune_off[v]);
            }
            self.tuned[v] = pitch + self.tune_off[v];
        }
    }

    /// Starts the sounding voices again on each oscillator whose unison
    /// count changed, as fresh notes on the new layout.
    fn restart_units(&mut self, voices: &VoiceBlock) {
        for s in 0..OSCILLATORS {
            if !std::mem::take(&mut self.restart[s]) {
                continue;
            }
            let unit = &mut self.units[s][self.copies[s] - 1];
            for v in voices.active_voices() {
                unit.lanes.start(v);
                let lanes = unit.osc.lanes_of(v);
                unit.osc
                    .set_unison(lanes, unit.lanes.pitch(), unit.lanes.frame());
                unit.osc.start(v, self.tuned[v], Onset::Fresh);
            }
        }
    }

    /// Ends the released voices whose envelopes have finished.
    fn end_finished(&mut self, voices: &mut VoiceBlock) {
        let gate = voices.gate();
        let all_envs = self.patch.all_envs();
        let ended: [bool; S] = std::array::from_fn(|v| {
            v < MAX_VOICES
                && voices.active_mask() & (1 << v) != 0
                && gate[v] == 0.0
                && self.envs[0].is_idle(v)
                && (!all_envs || (1..ENVELOPES).all(|e| self.envs[e].is_idle(v)))
        });
        for (v, end) in ended.iter().enumerate() {
            if *end {
                voices.end(v);
            }
        }
    }
}

impl VoiceHandler for Synth {
    fn voice_event(&mut self, voices: &mut VoiceBlock, event: VoiceEvent) {
        // A run of parameter changes at one time is rebuilt once, here,
        // before the first thing that reads the modules — a voice event or
        // a render. Rebuilding after each parameter made a preset load,
        // hundreds of values in one block, cost hundreds of rebuilds.
        self.rebuild(voices.settings_mut());
        let v = event.voice;
        let time = event.time;
        match event.kind {
            VoiceEventKind::Start { start, replaced } => {
                if let Some(note) = replaced
                    && self.ends.len() < REPORTS
                {
                    self.ends.push((time, note));
                }
                self.tune_off[v] = self.tuner.offset(voices.info(v).key).unwrap_or(0.0);
                self.tuned[v] = voices.pitch()[v] + self.tune_off[v];
                let key = self.tuned[v];
                let velocity = voices.velocity()[v];
                let onset = match start {
                    Start::Fresh => Onset::Fresh,
                    Start::Steal => Onset::Steal,
                    Start::Retrigger => Onset::Retrigger,
                    Start::Legato => Onset::Legato,
                    Start::Join { .. } => Onset::Join,
                };
                for (s, units) in self.units.iter_mut().enumerate() {
                    let unit = &mut units[self.copies[s] - 1];
                    unit.lanes.start(v);
                    let lanes = unit.osc.lanes_of(v);
                    unit.osc
                        .set_unison(lanes, unit.lanes.pitch(), unit.lanes.frame());
                    unit.osc.start(v, key, onset);
                }
                self.sub.start(v, key, onset);
                self.noise.start(v, onset);
                match start {
                    Start::Fresh => {
                        for e in &mut self.envs {
                            e.trigger(v, Trigger::New);
                        }
                        for (f, bank) in self.filters.iter_mut().enumerate() {
                            bank.reset_voice(v, &self.filter_state[f].voice);
                        }
                    }
                    Start::Steal | Start::Retrigger => {
                        for e in &mut self.envs {
                            e.trigger(v, Trigger::Retrigger);
                        }
                    }
                    Start::Legato => {
                        for e in &mut self.envs {
                            e.trigger(v, Trigger::Legato);
                        }
                    }
                    Start::Join { leader } => {
                        for e in &mut self.envs {
                            e.join(v, leader);
                        }
                        for (f, bank) in self.filters.iter_mut().enumerate() {
                            bank.reset_voice(v, &self.filter_state[f].voice);
                        }
                    }
                }
                if !matches!(start, Start::Legato) {
                    for l in &mut self.lfos {
                        l.trigger(v, 0);
                    }
                }
                let legato = matches!(start, Start::Legato | Start::Join { .. });
                self.modeng.note_on(
                    v,
                    NoteOn {
                        key: f32::from(voices.info(v).key),
                        velocity,
                        legato,
                        glide_seconds: voices.settings().glide.time,
                    },
                );
            }
            VoiceEventKind::Release => {
                for e in &mut self.envs {
                    e.release(v);
                }
                self.modeng.note_off(v, voices.release_velocity()[v]);
            }
            VoiceEventKind::End { note } => {
                if self.ends.len() < REPORTS {
                    self.ends.push((time, note));
                }
                for e in &mut self.envs {
                    e.reset(v);
                }
                self.modeng.voice_ended(v);
            }
        }
    }

    fn render(&mut self, voices: &mut VoiceBlock, frames: std::ops::Range<usize>) {
        let n = frames.len();
        if n == 0 {
            return;
        }
        self.block_frame = frames.start;
        self.rebuild(voices.settings_mut());
        self.retune(voices);
        self.restart_units(voices);
        self.render_modulation(voices, n);
        self.apply_control(voices);
        self.render_audio(voices, n);
        self.transport.advance(n);
        self.clock += n as u64;
        // Voices end on the control grid, whatever splits the renders, so
        // which voices sound (and so which path renders them) never depends
        // on block size.
        if self.clock.is_multiple_of(CHUNK as u64) {
            self.end_finished(voices);
        }
    }

    fn event(&mut self, voices: &mut VoiceBlock, event: Event) {
        // Only marks what changed: the modules are rebuilt once, at the
        // start of the next render or voice event, so a preset load — every
        // value the host sends in one block — costs one rebuild, not one
        // per value. A voicing setting cannot wait that long: the voice
        // manager reads it before it calls back, so a note at this time
        // would miss the change, and it rebuilds now.
        if let Event::Param { id, value, .. } = event {
            let time = self.block_frame as u32;
            self.set_param(id, value, time);
            if dirty_bit(id) & dirty::VOICE != 0 {
                self.rebuild(voices.settings_mut());
            }
        }
    }
}
