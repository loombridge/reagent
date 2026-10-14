//! Reagent's modulation matrix: every source and destination registered
//! with the plug-in kit's modulation engine (`lspk-mod`), the matrix saved
//! with the patch, and what the audio thread needs to know about the
//! compiled program.
//!
//! Sources are the kit's standard set for a wavetable synthesiser
//! (engine-core section 12): four envelopes, ten LFOs with their Y
//! outputs, the velocity and key curves, pressure, note-on randoms and
//! alternates, eight macros, the mod wheel, pitch bend, MPE, release
//! velocity, the constant, the audio of each oscillator, the sub, the noise
//! and each filter, voice load and the voice sequencer's values.
//!
//! Destinations are every continuous and whole-number parameter of every
//! module, each LFO's sixteen point-modulation buses, the macros (whose
//! routes feed back into the matrix) and every slot's amount and output
//! (engine-core R111). Switches and choices aren't modulated.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use lspk_mod::commands::SlotView;
use lspk_mod::{
    Config, Curve, DestId, DestinationInfo, Destinations, Matrix, Program, SourceId, Sources,
    Standard, Target,
};
use lspk_params::{ParamAddress, ParamDescriptor, ParamId, ParamKind, ParamSpace};
use serde::{Deserialize, Serialize};

use crate::params::{
    self, AMP_ENV, FILTER, LFOS, MACROS, MATRIX, MOD_ENV, NOISE, OSC, SUB, channel, filter, global,
    osc,
};

/// The audio sources, in registry order: each main oscillator, the sub,
/// the noise and each filter.
pub const AUDIO_SOURCES: [(&str, &str); 7] = [
    ("osc_a", "A audio"),
    ("osc_b", "B audio"),
    ("osc_c", "C audio"),
    ("sub", "Sub audio"),
    ("noise", "Noise audio"),
    ("filter1", "Filter 1 audio"),
    ("filter2", "Filter 2 audio"),
];

/// The modulation sources.
#[must_use]
pub fn sources() -> Sources {
    Sources::standard(&Standard {
        envelopes: params::ENVELOPES,
        lfos: LFOS,
        macros: MACROS,
        sequence_values: 2,
        audio: &AUDIO_SOURCES,
    })
}

/// Slot `slot`'s amount, as a destination: a modulation target only, never
/// a host parameter.
#[must_use]
pub const fn slot_amount(slot: u8) -> ParamId {
    ParamAddress::new(MATRIX, slot, 0).id()
}

/// Slot `slot`'s output scaler, as a destination.
#[must_use]
pub const fn slot_output(slot: u8) -> ParamId {
    ParamAddress::new(MATRIX, slot, 1).id()
}

/// Whether parameter `id` is modulated per voice, and at audio rate.
fn scope_and_rate(id: ParamId) -> (bool, Rate) {
    let Some(address) = ParamAddress::of(id) else {
        return (false, Rate::Control);
    };
    match (address.module, address.param) {
        // Pitch, amplitude and pan are followed sample by sample.
        (
            OSC,
            osc::OCTAVE | osc::SEMITONE | osc::FINE | osc::SWEEP | channel::LEVEL | channel::PAN,
        )
        | (SUB | NOISE, channel::LEVEL | channel::PAN) => (true, Rate::Audio),
        (FILTER, filter::CUTOFF) => (true, Rate::Adaptive),
        // Unison's controls are the oscillator's, shared by its voices.
        (
            OSC,
            osc::DETUNE | osc::UNISON_MIX | osc::WIDTH | osc::DETUNE_RANGE | osc::FRAME_SPREAD,
        ) => (false, Rate::Control),
        (OSC | SUB | NOISE | FILTER | AMP_ENV | MOD_ENV, _) => (true, Rate::Control),
        (0, _) if id == global::AMP || id == global::TUNING => (true, Rate::Audio),
        _ => (false, Rate::Control),
    }
}

#[derive(Clone, Copy)]
enum Rate {
    Audio,
    Control,
    Adaptive,
}

/// Whether a parameter can be modulated: continuous values, gains and
/// whole numbers, but not switches or choices, nor the voice count, the
/// bend ranges or the quality, which change how the instrument is built.
fn modulatable(descriptor: &ParamDescriptor) -> bool {
    let fixed = [
        global::VOICES,
        global::BEND_UP,
        global::BEND_DOWN,
        global::MPE_BEND,
        global::REFERENCE_PITCH,
        global::ENV_SCALE,
        global::LFO_SCALE,
    ];
    matches!(
        descriptor.kind,
        ParamKind::Continuous { .. } | ParamKind::Gain { .. } | ParamKind::Integer { .. }
    ) && !fixed.contains(&descriptor.id)
        && !is_macro(descriptor.id)
        && !matches!(
            ParamAddress::of(descriptor.id),
            // The unison count changes the oscillator's layout; phase and
            // phase random take effect only as a note starts.
            Some(ParamAddress {
                module: OSC,
                param: osc::UNISON | osc::PHASE | osc::PHASE_RANDOM,
                ..
            })
        )
}

fn is_macro(id: ParamId) -> bool {
    (0..MACROS).any(|m| global::macro_id(m) == id)
}

/// The modulation destinations of `space` with the `sources` registry.
///
/// # Panics
///
/// Never: every destination is unique, as the tests check.
#[must_use]
pub fn destinations(space: &ParamSpace, sources: &Sources) -> Destinations {
    let mut dests = Destinations::new();
    let mut add = |info: DestinationInfo| {
        dests.add(info).expect("every destination is unique");
    };
    for descriptor in space.descriptors() {
        if !modulatable(&descriptor) {
            continue;
        }
        let (voice, rate) = scope_and_rate(descriptor.id);
        let mut info = DestinationInfo::param(descriptor.id, &descriptor.name);
        if voice {
            info = info.voice();
        }
        info = match rate {
            Rate::Audio => info.audio(),
            Rate::Control => info,
            Rate::Adaptive => info.adaptive(),
        };
        if descriptor.id == global::TUNING {
            // Modulation may take main tuning past its control's range
            // (engine-core R137).
            info = info.unbounded();
        }
        add(info);
    }
    for m in 0..MACROS {
        let source = sources
            .find(&format!("macro{}", m + 1))
            .expect("the standard sources hold every macro");
        add(DestinationInfo::macro_control(
            global::macro_id(m),
            &format!("Macro {}", m + 1),
            source,
        ));
    }
    for l in 0..LFOS as u8 {
        for bus in 0..lspk_dsp::lfo::POINT_BUSES as u8 {
            add(DestinationInfo::lfo_bus(l, bus));
        }
    }
    for slot in 0..lspk_mod::SLOTS as u8 {
        add(DestinationInfo::slot_amount(
            slot_amount(slot),
            &format!("Slot {} amount", slot + 1),
            slot,
        ));
        add(DestinationInfo::slot_output(
            slot_output(slot),
            &format!("Slot {} output", slot + 1),
            slot,
        ));
    }
    dests
}

/// What the audio thread renders only when a route reads it, as bits: LFO
/// `l` is bit `l`, audio source `a` is bit `16 + a`. The main thread stores
/// the union of the old and new programs' needs before it sends a new
/// program, so the audio thread never skips a source the running program
/// reads.
#[derive(Debug, Default)]
pub struct Needs(AtomicU32);

impl Needs {
    /// The bits now.
    #[must_use]
    pub fn get(&self) -> u32 {
        self.0.load(Ordering::Acquire)
    }

    /// Stores `bits`.
    pub fn set(&self, bits: u32) {
        self.0.store(bits, Ordering::Release);
    }
}

/// The bit of the needs for audio source `index`.
#[must_use]
pub const fn audio_bit(index: usize) -> u32 {
    1 << (16 + index)
}

/// What `matrix` reads that the audio thread renders only on demand.
#[must_use]
pub fn needs_of(matrix: &Matrix, sources: &Sources) -> u32 {
    let mut bits = 0;
    let mut mark = |id: Option<SourceId>| {
        let Some(info) = id.and_then(|id| sources.get(id)) else {
            return;
        };
        let key = info.key.as_str();
        for l in 0..LFOS {
            let name = format!("lfo{}", l + 1);
            if key == name || key == format!("{name}_y") {
                bits |= 1 << l;
            }
        }
        if let Some(a) = AUDIO_SOURCES.iter().position(|(k, _)| *k == key) {
            bits |= audio_bit(a);
        }
    };
    for slot in matrix.slots() {
        if slot.bypass || slot.source.is_none() || slot.destination.is_none() {
            continue;
        }
        mark(slot.source);
        mark(slot.aux);
    }
    bits
}

/// The registries and the compiled program's configuration, shared by the
/// plug-in's main thread and its audio thread.
#[derive(Debug)]
pub struct Registry {
    /// The parameter space.
    pub space: ParamSpace,
    /// The sources.
    pub sources: Sources,
    /// The destinations.
    pub destinations: Destinations,
}

impl Registry {
    /// Reagent's registries. Allocates; make it off the audio thread.
    #[must_use]
    pub fn new() -> Arc<Self> {
        let space = params::space();
        let sources = sources();
        let destinations = destinations(&space, &sources);
        Arc::new(Self {
            space,
            sources,
            destinations,
        })
    }

    /// Parameter `id`'s destination, if it can be modulated.
    #[must_use]
    pub fn dest(&self, target: Target) -> Option<DestId> {
        self.destinations.find(target)
    }

    /// `matrix` compiled for `config`.
    #[must_use]
    pub fn compile(&self, matrix: &Matrix, config: Config) -> Program {
        Program::compile(matrix, &self.sources, &self.destinations, config)
    }
}

/// The matrix as a patch saves it: version 1 of Reagent's own state, the
/// slots in use as the kit's `mod.list` describes them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedMatrix {
    /// The layout's version, 1.
    pub version: u32,
    /// The slots in use.
    pub slots: Vec<SlotView>,
    /// Whether the matrix is locked.
    #[serde(default)]
    pub locked: bool,
}

/// `matrix` as JSON bytes.
#[must_use]
pub fn save(matrix: &Matrix, sources: &Sources) -> Vec<u8> {
    let saved = SavedMatrix {
        version: 1,
        slots: matrix.describe(sources),
        locked: matrix.locked(),
    };
    serde_json::to_vec(&saved).unwrap_or_default()
}

/// The matrix `bytes` hold, as [`save`] wrote it; empty bytes are an
/// empty matrix.
///
/// # Errors
///
/// The bytes aren't a saved matrix, or name a source, destination or slot
/// Reagent doesn't have.
pub fn load(bytes: &[u8], sources: &Sources, dests: &Destinations) -> Result<Matrix, String> {
    let mut matrix = Matrix::new();
    if bytes.is_empty() {
        return Ok(matrix);
    }
    let saved: SavedMatrix =
        serde_json::from_slice(bytes).map_err(|e| format!("not a Reagent patch state: {e}"))?;
    if saved.version != 1 {
        return Err(format!(
            "a patch state in layout {}, which this version can't read",
            saved.version
        ));
    }
    let source = |key: &Option<String>| -> Result<Option<SourceId>, String> {
        key.as_deref()
            .map(|k| {
                sources
                    .find(k)
                    .ok_or_else(|| format!("no modulation source {k:?}"))
            })
            .transpose()
    };
    for view in &saved.slots {
        if let Some(target) = view.destination
            && dests.find(target).is_none()
        {
            return Err(format!("{target:?} can't be modulated"));
        }
        let slot = matrix
            .slot_mut(usize::from(view.slot))
            .map_err(|e| e.to_string())?;
        slot.source = source(&view.source)?;
        slot.source_curve = Curve::from(&view.source_curve);
        slot.amount = view.amount.clamp(-1.0, 1.0);
        slot.polarity = view.polarity;
        slot.destination = view.destination;
        slot.aux = source(&view.aux)?;
        slot.aux_invert = view.aux_invert;
        slot.aux_curve = Curve::from(&view.aux_curve);
        slot.output = view.output.clamp(0.0, 1.0);
        slot.bypass = view.bypass;
    }
    matrix.set_locked(saved.locked);
    Ok(matrix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lspk_mod::Polarity;

    // Spec: engine-core R102, R111 (64 slots; macros, buses and slot
    // amounts are destinations).
    #[test]
    fn every_source_and_destination_is_registered() {
        let registry = Registry::new();
        let sources = &registry.sources;
        for key in [
            "env1",
            "env4",
            "lfo1",
            "lfo10_y",
            "velocity",
            "key",
            "macro8",
            "mod_wheel",
            "pitch_bend",
            "mpe_z",
            "constant",
            "osc_a",
            "filter2",
            "voice_load",
            "random_route",
        ] {
            assert!(sources.find(key).is_some(), "{key}");
        }
        let dest = |t: Target| registry.dest(t).is_some();
        let cutoff = params::id(FILTER, 1, filter::CUTOFF);
        assert!(dest(Target::Param(cutoff)));
        assert!(dest(Target::Param(params::id(OSC, 2, osc::SWEEP))));
        assert!(dest(Target::Param(global::macro_id(7))));
        assert!(dest(Target::LfoBus { lfo: 9, bus: 15 }));
        assert!(dest(Target::Param(slot_amount(63))));
        assert!(
            !dest(Target::Param(params::id(OSC, 0, channel::ON))),
            "a switch"
        );
        assert!(!dest(Target::Param(global::QUALITY)), "a choice");
        assert!(
            !dest(Target::Param(global::REFERENCE_PITCH)),
            "reference pitch"
        );
        assert!(!dest(Target::Param(global::ENV_SCALE)), "envelope scale");
        assert!(!dest(Target::Param(global::LFO_SCALE)), "lfo scale");
        let modulatable = registry
            .space
            .descriptors()
            .iter()
            .filter(|d| modulatable(d))
            .count();
        assert_eq!(
            registry.destinations.len(),
            modulatable + MACROS + LFOS * 16 + 2 * lspk_mod::SLOTS
        );
    }

    #[test]
    fn the_matrix_saves_and_loads() {
        let registry = Registry::new();
        let src = |k: &str| registry.sources.find(k).unwrap();
        let cutoff = Target::Param(params::id(FILTER, 0, filter::CUTOFF));
        let mut matrix = Matrix::new();
        matrix
            .add(src("lfo2"), cutoff, 0.5, Polarity::Bipolar)
            .unwrap();
        let n = matrix
            .add(
                src("velocity"),
                Target::Param(global::AMP),
                -0.25,
                Polarity::Unipolar,
            )
            .unwrap();
        matrix.set_aux(n, Some(src("mod_wheel")), true).unwrap();
        let bytes = save(&matrix, &registry.sources);
        let loaded = load(&bytes, &registry.sources, &registry.destinations).unwrap();
        assert_eq!(loaded, matrix);
        assert_eq!(
            needs_of(&loaded, &registry.sources),
            1 << 1,
            "LFO 2 alone renders on demand"
        );
        assert!(load(b"{", &registry.sources, &registry.destinations).is_err());
        assert_eq!(
            load(&[], &registry.sources, &registry.destinations).unwrap(),
            Matrix::new()
        );
    }
}
