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
//!
//! The velocity and key curves live with the matrix, the kit's
//! [`NoteCurves`](lspk_mod::NoteCurves): the drawn shapes a note's velocity
//! and number pass through, their glide and velocity legato (engine-core
//! section 11). [`set_note_curve`] redraws one; [`save`] and [`load`] keep
//! them beside the slots.
//!
//! [`apply_macros`] and [`swap_macros`] are the macros' two operations
//! (R115, R116). Applying bakes every macro route's current offset into the
//! parameter it drives and then removes the routes; swapping exchanges two
//! macros' routes.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use lspk_dsp::lfo::{Point, Shape};
use lspk_mod::commands::SlotView;
use lspk_mod::curve::{CurvePoint, diagonal};
use lspk_mod::{
    Config, Curve, DestId, DestinationInfo, Destinations, Matrix, Polarity, Program, SourceId,
    Sources, Standard, Target,
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

/// Which of the two note curves (engine-core section 11).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteCurve {
    /// Maps note-on velocity onto the velocity source (R99).
    Velocity,
    /// Maps the note number onto the key source (R99).
    Key,
}

/// `shape` as the points a patch saves.
fn shape_points(shape: &Shape) -> Vec<CurvePoint> {
    shape
        .points()
        .iter()
        .map(|p| CurvePoint {
            x: p.x,
            y: p.y,
            curve: p.curve,
        })
        .collect()
}

/// The shape `points` draw; no points is the diagonal, the init curve.
fn shape_of(points: &[CurvePoint]) -> Shape {
    if points.is_empty() {
        return diagonal();
    }
    let points: Vec<Point> = points
        .iter()
        .map(|p| Point::new(p.x, p.y).curve(p.curve))
        .collect();
    Shape::new(&points)
}

/// Sets `matrix`'s velocity or key curve to a drawn shape through `points`
/// (engine-core R99), as the curve's editor draws it; no points leave the
/// diagonal (engine-core R99, Init resets either to the diagonal).
pub fn set_note_curve(matrix: &mut Matrix, which: NoteCurve, points: &[CurvePoint]) {
    let shape = shape_of(points);
    match which {
        NoteCurve::Velocity => matrix.note_curves.velocity = shape,
        NoteCurve::Key => matrix.note_curves.key = shape,
    }
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

/// Source `m` (0 to 7) of the macros, if it exists.
fn macro_source(sources: &Sources, m: usize) -> Option<SourceId> {
    (m < MACROS)
        .then(|| sources.find(&format!("macro{}", m + 1)))
        .flatten()
}

/// The macro number of `id`, if it's a macro (0 to 7).
fn macro_number(sources: &Sources, id: SourceId) -> Option<usize> {
    (0..MACROS).find(|&m| macro_source(sources, m) == Some(id))
}

/// What applying the macros changed (engine-core R115).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MacroApply {
    /// Each parameter a macro drove, with its new value in its own units.
    pub params: Vec<(ParamId, f64)>,
    /// How many routes were removed.
    pub removed: usize,
}

/// Applies and deletes the macros (engine-core R115): adds each macro
/// route's current offset into the parameter it drives, then removes every
/// route whose source is a macro. `macro_value` is a macro's knob, 0 to 1,
/// and `value_of` reads a parameter's value in its own units.
///
/// A route's offset is the macro's own contribution: its source curve at the
/// macro's value, through the route's polarity, amount and output. A route's
/// auxiliary source isn't baked, and a destination that isn't a parameter
/// (an LFO point bus) has no value to add into; either way the route is
/// still removed.
pub fn apply_macros(
    matrix: &mut Matrix,
    space: &ParamSpace,
    sources: &Sources,
    macro_value: &dyn Fn(usize) -> f32,
    value_of: &dyn Fn(ParamId) -> Option<f64>,
) -> MacroApply {
    // What every macro route adds now, added up by destination (engine-core
    // R108: routes to one destination add).
    let mut offsets: Vec<(Target, f32)> = Vec::new();
    let mut routes: Vec<usize> = Vec::new();
    for (n, slot) in matrix.slots().iter().enumerate() {
        let Some(source) = slot.source else { continue };
        let Some(m) = macro_number(sources, source) else {
            continue;
        };
        routes.push(n);
        if slot.bypass {
            continue;
        }
        let Some(destination) = slot.destination else {
            continue;
        };
        let value = slot.source_curve.value(macro_value(m));
        let value = if slot.polarity == Polarity::Bipolar {
            2.0 * value - 1.0
        } else {
            value
        };
        let offset = value * slot.amount * slot.output;
        match offsets.iter_mut().find(|(t, _)| *t == destination) {
            Some((_, sum)) => *sum += offset,
            None => offsets.push((destination, offset)),
        }
    }
    // Every new value, from the state before the routes go. A slot amount or
    // output is part of the matrix; everything else is a parameter.
    let descriptors = space.descriptors();
    let mut params: Vec<(ParamId, f64)> = Vec::new();
    let mut fields: Vec<(usize, bool, f32)> = Vec::new();
    for (destination, offset) in offsets {
        if offset == 0.0 {
            continue;
        }
        let Target::Param(id) = destination else {
            continue;
        };
        if let Some(address) = ParamAddress::of(id)
            && address.module == MATRIX
            && let Ok(slot) = matrix.slot(usize::from(address.instance))
        {
            let output = address.param == 1;
            let value = if output { slot.output } else { slot.amount } + offset;
            let value = if output {
                value.clamp(0.0, 1.0)
            } else {
                value.clamp(-1.0, 1.0)
            };
            fields.push((usize::from(address.instance), output, value));
            continue;
        }
        let Some(descriptor) = descriptors.iter().find(|d| d.id == id) else {
            continue;
        };
        let Some(current) = value_of(id) else {
            continue;
        };
        let position = (descriptor.normalize(current) + f64::from(offset)).clamp(0.0, 1.0);
        params.push((id, descriptor.denormalize(position)));
    }
    let removed = routes.len();
    for n in routes {
        let _ = matrix.remove(n);
    }
    for (slot, output, value) in fields {
        if let Ok(slot) = matrix.slot_mut(slot) {
            if output {
                slot.output = value;
            } else {
                slot.amount = value;
            }
        }
    }
    MacroApply { params, removed }
}

/// Swaps two macros' routes (engine-core R116): every route whose source or
/// aux is macro `a` (0 to 7) takes macro `b` and the other way round, the
/// macros' own values staying put. Returns how many slots changed.
#[must_use]
pub fn swap_macros(matrix: &mut Matrix, sources: &Sources, a: usize, b: usize) -> usize {
    let (Some(a), Some(b)) = (macro_source(sources, a), macro_source(sources, b)) else {
        return 0;
    };
    let swap = |source: Option<SourceId>| match source {
        Some(s) if s == a => Some(b),
        Some(s) if s == b => Some(a),
        other => other,
    };
    let mut changed = 0;
    for n in 0..lspk_mod::SLOTS {
        let Ok(slot) = matrix.slot_mut(n) else {
            continue;
        };
        let (source, aux) = (slot.source, slot.aux);
        slot.source = swap(source);
        slot.aux = swap(aux);
        changed += usize::from(slot.source != source || slot.aux != aux);
    }
    changed
}

/// The velocity and key curves as a patch saves them (engine-core section
/// 11): the points each draws, empty for the diagonal.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedNoteCurves {
    /// The velocity curve's points.
    #[serde(default)]
    pub velocity: Vec<CurvePoint>,
    /// The key curve's points.
    #[serde(default)]
    pub key: Vec<CurvePoint>,
}

/// The matrix as a patch saves it: version 1 of Reagent's own state, the
/// slots in use as the kit's `mod.list` describes them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedMatrix {
    /// The layout's version, 1.
    pub version: u32,
    /// The slots in use.
    pub slots: Vec<SlotView>,
    /// The velocity and key curves.
    #[serde(default)]
    pub note_curves: SavedNoteCurves,
    /// Whether the matrix is locked.
    #[serde(default)]
    pub locked: bool,
}

impl Default for SavedMatrix {
    /// An empty matrix's state.
    fn default() -> Self {
        Self {
            version: 1,
            slots: Vec::new(),
            note_curves: SavedNoteCurves::default(),
            locked: false,
        }
    }
}

impl SavedMatrix {
    /// `matrix` as a patch saves it.
    #[must_use]
    pub fn of(matrix: &Matrix, sources: &Sources) -> Self {
        Self {
            version: 1,
            slots: matrix.describe(sources),
            note_curves: SavedNoteCurves {
                velocity: shape_points(&matrix.note_curves.velocity),
                key: shape_points(&matrix.note_curves.key),
            },
            locked: matrix.locked(),
        }
    }

    /// The matrix this state holds.
    ///
    /// # Errors
    ///
    /// The state is a later layout, or names a source, destination or slot
    /// Reagent doesn't have.
    pub fn matrix(&self, sources: &Sources, dests: &Destinations) -> Result<Matrix, String> {
        if self.version != 1 {
            return Err(format!(
                "a patch state in layout {}, which this version can't read",
                self.version
            ));
        }
        let mut matrix = Matrix::new();
        let source = |key: &Option<String>| -> Result<Option<SourceId>, String> {
            key.as_deref()
                .map(|k| {
                    sources
                        .find(k)
                        .ok_or_else(|| format!("no modulation source {k:?}"))
                })
                .transpose()
        };
        for view in &self.slots {
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
        matrix.note_curves.velocity = shape_of(&self.note_curves.velocity);
        matrix.note_curves.key = shape_of(&self.note_curves.key);
        matrix.set_locked(self.locked);
        Ok(matrix)
    }
}

/// `matrix` as JSON bytes.
#[must_use]
pub fn save(matrix: &Matrix, sources: &Sources) -> Vec<u8> {
    serde_json::to_vec(&SavedMatrix::of(matrix, sources)).unwrap_or_default()
}

/// The matrix `bytes` hold, as [`save`] wrote it; empty bytes are an
/// empty matrix.
///
/// # Errors
///
/// The bytes aren't a saved matrix, or name a source, destination or slot
/// Reagent doesn't have.
pub fn load(bytes: &[u8], sources: &Sources, dests: &Destinations) -> Result<Matrix, String> {
    if bytes.is_empty() {
        return Ok(Matrix::new());
    }
    let saved: SavedMatrix =
        serde_json::from_slice(bytes).map_err(|e| format!("not a Reagent patch state: {e}"))?;
    saved.matrix(sources, dests)
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
            "release_velocity",
            "channel_pressure",
            "poly_pressure",
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
        // Spec: engine-core R114 (a macro is a source), R99 (a drawn
        // velocity curve); both stay in the patch.
        matrix
            .add(
                src("macro1"),
                Target::Param(params::id(OSC, 0, channel::LEVEL)),
                0.25,
                Polarity::Unipolar,
            )
            .unwrap();
        set_note_curve(
            &mut matrix,
            NoteCurve::Velocity,
            &[
                CurvePoint {
                    x: 0.0,
                    y: 0.0,
                    curve: 0.0,
                },
                CurvePoint {
                    x: 1.0,
                    y: 0.5,
                    curve: 0.0,
                },
            ],
        );
        let bytes = save(&matrix, &registry.sources);
        let loaded = load(&bytes, &registry.sources, &registry.destinations).unwrap();
        assert_eq!(loaded, matrix);
        assert_eq!(
            loaded.note_curves.velocity.value(1.0),
            0.5,
            "the drawn velocity curve stays"
        );
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

    // Spec: engine-core R99 (the velocity and key curves are drawn graphs;
    // Init resets them to the diagonal).
    #[test]
    fn the_note_curves_draw_and_reset() {
        let mut matrix = Matrix::new();
        assert_eq!(matrix.note_curves.velocity.value(0.5), 0.5);
        let points = [
            CurvePoint {
                x: 0.0,
                y: 0.0,
                curve: 0.0,
            },
            CurvePoint {
                x: 1.0,
                y: 0.25,
                curve: 0.0,
            },
        ];
        set_note_curve(&mut matrix, NoteCurve::Key, &points);
        assert_eq!(matrix.note_curves.key.value(1.0), 0.25);
        assert!(matrix.note_curves != Matrix::new().note_curves);
        // No points draw the diagonal again, and Init makes a new matrix
        // with it too.
        set_note_curve(&mut matrix, NoteCurve::Key, &[]);
        assert_eq!(matrix.note_curves, Matrix::new().note_curves);
    }

    // Spec: engine-core R114 (a macro is a source, a destination and an aux
    // source), R115 (Apply and Delete adds each macro's current offset into
    // the parameters it drives, then removes every macro route).
    #[test]
    fn applying_the_macros_bakes_their_offsets() {
        let registry = Registry::new();
        let src = |k: &str| registry.sources.find(k).unwrap();
        let level = params::id(OSC, 0, channel::LEVEL);
        let macro2 = global::macro_id(1);
        let cutoff = params::id(FILTER, 0, filter::CUTOFF);
        let mut matrix = Matrix::new();
        // Macro 1 at half its travel moves the level a quarter of the way,
        // Macro 2 at full half the way: together three quarters.
        matrix
            .add(src("macro1"), Target::Param(level), 0.5, Polarity::Unipolar)
            .unwrap();
        matrix
            .add(src("macro2"), Target::Param(level), 0.5, Polarity::Unipolar)
            .unwrap();
        // The macros are destinations too, and a slot's amount is one.
        matrix
            .add(
                src("macro1"),
                Target::Param(macro2),
                0.5,
                Polarity::Unipolar,
            )
            .unwrap();
        matrix
            .add(
                src("macro1"),
                Target::Param(slot_amount(9)),
                0.5,
                Polarity::Unipolar,
            )
            .unwrap();
        // A two-way route adds nothing at the middle, and a route from
        // another source stays.
        matrix
            .add(src("macro1"), Target::Param(cutoff), 0.5, Polarity::Bipolar)
            .unwrap();
        matrix
            .add(src("lfo1"), Target::Param(cutoff), 0.5, Polarity::Unipolar)
            .unwrap();
        let value_of = |id: ParamId| {
            Some(match id {
                x if x == macro2 => 50.0,
                x if x == cutoff => 425.0,
                x if x == level => 0.0,
                _ => 0.0,
            })
        };
        let apply = apply_macros(
            &mut matrix,
            &registry.space,
            &registry.sources,
            &|m| if m == 0 { 0.5 } else { 1.0 },
            &value_of,
        );
        assert_eq!(apply.removed, 5, "every macro route goes");
        assert_eq!(apply.params, vec![(level, 75.0), (macro2, 75.0)]);
        assert_eq!(
            matrix.slot(9).unwrap().amount,
            0.25,
            "the slot's amount is baked"
        );
        let routes = matrix.describe(&registry.sources);
        assert_eq!(routes.len(), 1, "the LFO's route stays: {routes:?}");
        assert_eq!(routes[0].source.as_deref(), Some("lfo1"));
    }

    // Spec: engine-core R116 (swapping two macros swaps their routes).
    #[test]
    fn swapping_two_macros_moves_their_routes() {
        let registry = Registry::new();
        let src = |k: &str| registry.sources.find(k).unwrap();
        let level = Target::Param(params::id(OSC, 0, channel::LEVEL));
        let cutoff = Target::Param(params::id(FILTER, 0, filter::CUTOFF));
        let mut matrix = Matrix::new();
        let n = matrix
            .add(src("macro1"), level, 0.5, Polarity::Unipolar)
            .unwrap();
        matrix
            .add(src("macro2"), cutoff, 0.5, Polarity::Unipolar)
            .unwrap();
        matrix
            .add(src("lfo1"), level, 0.5, Polarity::Unipolar)
            .unwrap();
        // A macro used as an aux source swaps too: two slots change.
        matrix.set_aux(n, Some(src("macro2")), false).unwrap();
        assert_eq!(swap_macros(&mut matrix, &registry.sources, 0, 1), 2);
        assert_eq!(matrix.slot(0).unwrap().source, Some(src("macro2")));
        assert_eq!(matrix.slot(0).unwrap().aux, Some(src("macro1")));
        assert_eq!(matrix.slot(1).unwrap().source, Some(src("macro1")));
        assert_eq!(matrix.slot(2).unwrap().source, Some(src("lfo1")));
    }
}
