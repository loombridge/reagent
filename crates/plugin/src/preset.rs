//! Reagent's preset files: the plug-in kit's preset format with Reagent's
//! own state in it — the modulation matrix and, per oscillator slot, every
//! engine's state — saved and loaded by the `preset.*` commands, with A/B
//! compare and an undo history.
//!
//! A preset is UTF-8 JSON in the kit's format ([`lspk_presets`]), its
//! `format` field `reagent-preset`:
//!
//! ```json
//! {
//!   "format": "reagent-preset",
//!   "schema": 1,
//!   "params": [
//!     { "id": 16777217, "value": 1.0 },
//!     { "id": 16777233, "value": "Dense" }
//!   ],
//!   "state": {
//!     "matrix": { "version": 1, "slots": [], "locked": false },
//!     "oscillators": [
//!       { "wavetable": { "table": "reagent/basic-shapes" } }, {}, {}
//!     ]
//!   }
//! }
//! ```
//!
//! `params` holds only the values that differ from their defaults. `state`
//! is Reagent's own: the matrix as [`SavedMatrix`] saves it, and each
//! oscillator slot's engines' state by engine name. A slot keeps every
//! engine's state, so a state a later engine saved — a granular one, say —
//! is kept as it is and written back when the patch is saved again; the
//! wavetable engine's is the table the slot plays.
//!
//! The commands are `preset.save` and `preset.load` (files outside the
//! project, so agents ask the person first), `preset.store_a` and
//! `preset.store_b` with `preset.compare` for A/B, and `preset.undo` and
//! `preset.redo` for the changes preset commands made.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use lspk_commands::{CommandSpec, Context, Permission, RegisterError, Registry as Commands};
use lspk_mod::{Destinations, Matrix, Sources};
use lspk_plugin::{ParamValues, PluginError};
use lspk_presets::{Preset, Side};
use reagent_engine::modulation::SavedMatrix;
use reagent_engine::params::{OSC_NAMES, OSCILLATORS};
use reagent_engine::{Registry, table};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::Reagent;

/// The `format` field every Reagent preset starts with.
pub const FORMAT: &str = "reagent-preset";

/// The engine name a slot's wavetable state sits under.
pub const WAVETABLE: &str = "wavetable";

/// How many preset changes the undo history keeps.
pub const UNDO_PRESETS: usize = 64;

/// `preset.save`: write the patch to a preset file.
pub const SAVE: &str = "preset.save";
/// `preset.load`: apply a preset file.
pub const LOAD: &str = "preset.load";
/// `preset.store_a`: keep the patch as A/B compare's A side.
pub const STORE_A: &str = "preset.store_a";
/// `preset.store_b`: keep the patch as A/B compare's B side.
pub const STORE_B: &str = "preset.store_b";
/// `preset.compare`: switch A/B compare to the other side.
pub const COMPARE: &str = "preset.compare";
/// `preset.undo`: put the patch back before the last preset change.
pub const UNDO: &str = "preset.undo";
/// `preset.redo`: put the patch back the way undo turned it away from.
pub const REDO: &str = "preset.redo";

/// The wavetable engine's own state: which table a slot plays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Wavetable {
    /// The table's name in the process-wide cache.
    #[serde(default = "default_table")]
    pub table: String,
}

fn default_table() -> String {
    table::DEFAULT_TABLE.to_owned()
}

/// One oscillator slot's engines' state, by engine name: each engine's own
/// state as it saved it. The wavetable engine's state is a [`Wavetable`];
/// anything else is kept as it came, so a state a later engine saved isn't
/// lost when this build saves the patch again.
pub type SlotState = BTreeMap<String, Value>;

/// A new patch's slots: every slot's wavetable engine on Reagent's default
/// table.
#[must_use]
pub fn default_slots() -> Vec<SlotState> {
    (0..OSCILLATORS).map(|_| default_slot()).collect()
}

/// One new slot.
fn default_slot() -> SlotState {
    let wavetable = default_wavetable();
    SlotState::from([(WAVETABLE.to_owned(), wavetable)])
}

/// The wavetable engine's state on Reagent's default table.
fn default_wavetable() -> Value {
    serde_json::to_value(Wavetable {
        table: default_table(),
    })
    .unwrap_or(Value::Null)
}

/// Reagent's own state inside a preset: the modulation matrix and each
/// oscillator slot's engines.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// The modulation matrix as a patch saves it.
    #[serde(default)]
    pub matrix: SavedMatrix,
    /// Each oscillator slot's engines' state, by engine name. Absent, the
    /// slots are new ones.
    #[serde(default)]
    pub oscillators: Vec<SlotState>,
}

impl State {
    /// `matrix` and `slots` as a preset's state.
    #[must_use]
    pub fn of(matrix: &Matrix, sources: &Sources, slots: &[SlotState]) -> Self {
        Self {
            matrix: SavedMatrix::of(matrix, sources),
            oscillators: slots.to_vec(),
        }
    }

    /// The matrix and the slots this state holds, checked for this build.
    /// A slot without a wavetable state takes the default table.
    ///
    /// # Errors
    ///
    /// The matrix is a later layout or names a route Reagent doesn't have,
    /// the state doesn't hold Reagent's three slots, or a slot plays a
    /// table this build doesn't have loaded.
    pub fn load(
        &self,
        sources: &Sources,
        dests: &Destinations,
    ) -> Result<(Matrix, Vec<SlotState>), String> {
        let matrix = self.matrix.matrix(sources, dests)?;
        let mut slots = if self.oscillators.is_empty() {
            default_slots()
        } else if self.oscillators.len() == OSCILLATORS {
            self.oscillators.clone()
        } else {
            return Err(format!(
                "the preset has {} oscillator slots, where Reagent has {OSCILLATORS}",
                self.oscillators.len()
            ));
        };
        for (at, slot) in slots.iter_mut().enumerate() {
            let wavetable = slot
                .entry(WAVETABLE.to_owned())
                .or_insert(default_wavetable());
            let saved: Wavetable = serde_json::from_value(wavetable.clone()).map_err(|error| {
                format!(
                    "oscillator {}'s wavetable state doesn't read: {error}",
                    OSC_NAMES[at]
                )
            })?;
            if saved.table != table::DEFAULT_TABLE {
                return Err(format!(
                    "oscillator {}'s preset plays the wavetable {:?}, which isn't loaded",
                    OSC_NAMES[at], saved.table
                ));
            }
        }
        Ok((matrix, slots))
    }
}

/// The text `values` and `state` save as a Reagent preset.
///
/// # Errors
///
/// A value the kit's format can't write.
pub fn encode(values: &ParamValues, state: &State) -> Result<String, PluginError> {
    let state = serde_json::to_value(state).map_err(|error| PluginError::new(error.to_string()))?;
    lspk_presets::save(FORMAT, values, &state).map_err(|error| PluginError::new(error.to_string()))
}

/// The preset `text` holds, checked against `values` and `registry`, with
/// Reagent's own state read and written back in its canonical form.
///
/// # Errors
///
/// The text isn't a Reagent preset this build can read.
pub fn decode(
    text: &str,
    values: &ParamValues,
    registry: &Registry,
) -> Result<Preset, PluginError> {
    let preset = lspk_presets::load(text, FORMAT, values)
        .map_err(|error| PluginError::new(error.to_string()))?;
    let state: State = serde_json::from_value(preset.state.clone()).map_err(|error| {
        PluginError::new(format!("the preset's state isn't Reagent's: {error}"))
    })?;
    let (matrix, slots) = state
        .load(&registry.sources, &registry.destinations)
        .map_err(PluginError::new)?;
    let state = serde_json::to_value(State::of(&matrix, &registry.sources, &slots))
        .map_err(|error| PluginError::new(error.to_string()))?;
    Ok(Preset {
        values: preset.values,
        state,
    })
}

/// The preset file at `path`.
///
/// # Errors
///
/// The file can't be read, is larger than a preset, or its text isn't one
/// this build can read.
pub fn read(path: &Path, values: &ParamValues, registry: &Registry) -> Result<Preset, PluginError> {
    let io = |error: std::io::Error| PluginError::new(format!("{}: {error}", path.display()));
    let size = fs::metadata(path).map_err(io)?.len();
    if size > lspk_presets::MAX_BYTES as u64 {
        return Err(PluginError::new(format!(
            "{}: the file is {size} bytes, more than a preset holds",
            path.display()
        )));
    }
    let text = fs::read_to_string(path).map_err(io)?;
    decode(&text, values, registry)
        .map_err(|error| PluginError::new(format!("{}: {error}", path.display())))
}

/// Writes `values` and `state` to `path` as a Reagent preset. The file is
/// written beside `path` under a hidden name and renamed into place, so
/// `path` holds a whole preset, old or new, whenever the write stops.
///
/// # Errors
///
/// The file can't be written, or a value can't be saved.
pub fn write(path: &Path, values: &ParamValues, state: &State) -> Result<(), PluginError> {
    use std::io::Write;
    let io = |error: std::io::Error| PluginError::new(format!("{}: {error}", path.display()));
    let text = encode(values, state)?;
    let folder = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let hidden = folder.join(format!(".{name}.saving"));
    let written = (|| {
        let mut file = fs::File::create(&hidden)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&hidden, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&hidden);
    }
    written.map_err(io)
}

/// `args` as `T`, as `run_command` receives them.
fn parse_args<T: DeserializeOwned>(args: &Value) -> Result<T, PluginError> {
    serde_json::from_value(args.clone())
        .map_err(|error| PluginError::new(format!("the arguments don't fit: {error}")))
}

impl Reagent {
    /// The host's parameter values.
    ///
    /// # Errors
    ///
    /// They aren't connected yet.
    fn values(&self) -> Result<&ParamValues, PluginError> {
        self.host
            .values()
            .ok_or_else(|| PluginError::new("the parameter values aren't connected"))
    }

    /// The patch now as a preset: every value that differs from its
    /// default, the matrix, and each oscillator slot's engines' state.
    ///
    /// # Errors
    ///
    /// The parameter values aren't connected.
    fn patch(&self) -> Result<Preset, PluginError> {
        let state =
            serde_json::to_value(State::of(&self.matrix, &self.registry.sources, &self.slots))
                .map_err(|error| PluginError::new(error.to_string()))?;
        Ok(Preset::of(self.values()?, state))
    }

    /// Applies `preset` to the patch: every parameter the preset doesn't
    /// name goes back to its default, as an edit the host records, and the
    /// matrix and the oscillators' state follow. A locked matrix stays as
    /// it is.
    ///
    /// # Errors
    ///
    /// An edit the host refuses, or a state this build can't read.
    fn apply_preset(&mut self, preset: &Preset) -> Result<(), PluginError> {
        let edits = {
            let values = self.values()?;
            let mut edits = Vec::new();
            for descriptor in values.descriptors() {
                let target = preset.value(descriptor.id).unwrap_or(descriptor.default);
                if values.get(descriptor.id) != Some(target) {
                    edits.push((descriptor.id, target));
                }
            }
            edits
        };
        for (id, value) in edits {
            self.host
                .edit(id, value)
                .map_err(|error| PluginError::new(error.to_string()))?;
        }
        let state: State = serde_json::from_value(preset.state.clone()).map_err(|error| {
            PluginError::new(format!("the preset's state isn't Reagent's: {error}"))
        })?;
        let (matrix, slots) = state
            .load(&self.registry.sources, &self.registry.destinations)
            .map_err(PluginError::new)?;
        self.matrix.load(&matrix);
        self.slots = slots;
        self.send_matrix();
        self.recompile();
        self.host.mark_dirty();
        Ok(())
    }

    /// `preset.save`: writes the patch to `path`.
    fn save_preset(&self, path: &str) -> Result<PresetResult, PluginError> {
        let state = State::of(&self.matrix, &self.registry.sources, &self.slots);
        write(Path::new(path), self.values()?, &state)?;
        Ok(PresetResult {
            path: path.to_owned(),
        })
    }

    /// `preset.load`: applies `path`'s preset, keeping the patch before it
    /// for undo.
    fn load_preset(&mut self, path: &str) -> Result<PresetResult, PluginError> {
        let before = self.patch()?;
        let preset = read(Path::new(path), self.values()?, &self.registry)?;
        self.history.record(before);
        self.apply_preset(&preset)?;
        Ok(PresetResult {
            path: path.to_owned(),
        })
    }

    /// `preset.store_a` and `preset.store_b`: keeps the patch now as
    /// `side`'s.
    fn store_side(&mut self, side: Side) -> Result<StoreResult, PluginError> {
        let patch = self.patch()?;
        self.compare.store(side, patch);
        Ok(StoreResult {
            side: side.key().to_owned(),
        })
    }

    /// `preset.compare`: keeps the patch now on the side showing, then
    /// switches to the patch kept on the other side and applies it.
    fn compare_sides(&mut self) -> Result<CompareResult, PluginError> {
        let patch = self.patch()?;
        let showing = self.compare.showing();
        self.compare.store(showing, patch);
        let Some(other) = self.compare.switch().cloned() else {
            return Ok(CompareResult {
                showing: self.compare.showing().key().to_owned(),
                applied: false,
            });
        };
        let before = self.patch()?;
        self.history.record(before);
        self.apply_preset(&other)?;
        Ok(CompareResult {
            showing: self.compare.showing().key().to_owned(),
            applied: true,
        })
    }

    /// `preset.undo`: puts the patch back as it was before the last preset
    /// change.
    fn undo_preset(&mut self) -> Result<UndoResult, PluginError> {
        let current = self.patch()?;
        let Some(past) = self.history.undo(&current) else {
            return Ok(UndoResult { restored: false });
        };
        self.apply_preset(&past)?;
        Ok(UndoResult { restored: true })
    }

    /// `preset.redo`: puts the patch back the way the last undo turned it
    /// away from.
    fn redo_preset(&mut self) -> Result<UndoResult, PluginError> {
        let current = self.patch()?;
        let Some(forward) = self.history.redo(&current) else {
            return Ok(UndoResult { restored: false });
        };
        self.apply_preset(&forward)?;
        Ok(UndoResult { restored: true })
    }
}

/// The `path` argument of `preset.save` and `preset.load`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PathArgs {
    /// The preset file to write, or to read.
    pub path: String,
}

/// What `preset.save` and `preset.load` return.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PresetResult {
    /// The file, as it was given.
    pub path: String,
}

/// What `preset.store_a` and `preset.store_b` return.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StoreResult {
    /// The side the patch was kept as: `a` or `b`.
    pub side: String,
}

/// What `preset.compare` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CompareResult {
    /// The side showing now: `a` or `b`.
    pub showing: String,
    /// Whether the other side's patch was applied. False when nothing was
    /// kept there.
    pub applied: bool,
}

/// What `preset.undo` and `preset.redo` return.
#[derive(Debug, Serialize, JsonSchema)]
pub struct UndoResult {
    /// Whether a patch was put back.
    pub restored: bool,
}

/// Adds Reagent's preset commands to `registry`.
///
/// # Errors
///
/// One of them is already registered.
pub fn register(registry: &mut Commands) -> Result<(), RegisterError> {
    registry.register(
        CommandSpec::new(SAVE, "Save Preset")
            .description(
                "Writes the patch now as a preset file at `path`: every parameter that differs \
                 from its default, the modulation matrix, and each oscillator's state. It writes \
                 a file outside the project, so an agent asks the person first.",
            )
            .permission(Permission::External)
            .context(Context::Global)
            .args::<PathArgs>()
            .result::<PresetResult>(),
    )?;
    registry.register(
        CommandSpec::new(LOAD, "Load Preset")
            .description(
                "Reads the preset file at `path` and applies it: every parameter the preset \
                 doesn't name goes back to its default, the matrix and the oscillators' state \
                 follow, and a locked matrix stays as it is. The patch before the load is kept \
                 for `preset.undo`. It reads a file outside the project, so an agent asks the \
                 person first.",
            )
            .permission(Permission::External)
            .context(Context::Global)
            .args::<PathArgs>()
            .result::<PresetResult>(),
    )?;
    registry.register(
        CommandSpec::new(STORE_A, "Store A")
            .description(
                "Keeps the patch now as the A side of A/B compare. `preset.compare` switches to \
                 the other side and back, to hear one patch against the other.",
            )
            .permission(Permission::Edit)
            .context(Context::Global)
            .args::<lspk_commands::NoArgs>()
            .result::<StoreResult>(),
    )?;
    registry.register(
        CommandSpec::new(STORE_B, "Store B")
            .description(
                "Keeps the patch now as the B side of A/B compare. `preset.compare` switches to \
                 the other side and back, to hear one patch against the other.",
            )
            .permission(Permission::Edit)
            .context(Context::Global)
            .args::<lspk_commands::NoArgs>()
            .result::<StoreResult>(),
    )?;
    registry.register(
        CommandSpec::new(COMPARE, "Compare A and B")
            .description(
                "Keeps the patch now on the side showing and applies the patch kept on the other \
                 side, switching A/B compare to it. With nothing kept on the other side it \
                 changes nothing, and says so. The patch before the switch is kept for \
                 `preset.undo`.",
            )
            .permission(Permission::Edit)
            .context(Context::Global)
            .args::<lspk_commands::NoArgs>()
            .result::<CompareResult>(),
    )?;
    registry.register(
        CommandSpec::new(UNDO, "Undo Preset Change")
            .description(
                "Puts the patch back as it was before the last `preset.load` or `preset.compare`, \
                 and says whether there was one. It leaves the two sides of A/B compare alone.",
            )
            .permission(Permission::Edit)
            .context(Context::Global)
            .args::<lspk_commands::NoArgs>()
            .result::<UndoResult>(),
    )?;
    registry.register(
        CommandSpec::new(REDO, "Redo Preset Change")
            .description(
                "Puts the patch back the way the last `preset.undo` turned it away from, and says \
                 whether there was one.",
            )
            .permission(Permission::Edit)
            .context(Context::Global)
            .args::<lspk_commands::NoArgs>()
            .result::<UndoResult>(),
    )
}

/// Runs the preset command `id` with `args`.
///
/// # Errors
///
/// There's no such command, its arguments don't fit, or it failed.
pub fn run(plugin: &mut Reagent, id: &str, args: &Value) -> Result<Value, PluginError> {
    match id {
        SAVE => Ok(
            serde_json::to_value(plugin.save_preset(&parse_args::<PathArgs>(args)?.path)?)
                .expect("serialisable"),
        ),
        LOAD => Ok(
            serde_json::to_value(plugin.load_preset(&parse_args::<PathArgs>(args)?.path)?)
                .expect("serialisable"),
        ),
        STORE_A => Ok(serde_json::to_value(plugin.store_side(Side::A)?).expect("serialisable")),
        STORE_B => Ok(serde_json::to_value(plugin.store_side(Side::B)?).expect("serialisable")),
        COMPARE => Ok(serde_json::to_value(plugin.compare_sides()?).expect("serialisable")),
        UNDO => Ok(serde_json::to_value(plugin.undo_preset()?).expect("serialisable")),
        REDO => Ok(serde_json::to_value(plugin.redo_preset()?).expect("serialisable")),
        _ => Err(PluginError::new(format!("no command `{id}`"))),
    }
}
