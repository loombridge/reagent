//! Reagent's plug-in without a format export or an editor: the
//! `lspk_plugin::Plugin` implementation, the preset files and the hand-built
//! patches. `reagent` adds the formats, the editor and the agent interface;
//! `reagent-cli` and a host that compiles Reagent in, as Beatlayer's native
//! devices do, use this crate and reach no UI toolkit.
//!
//! The sound is `reagent-engine`'s. This crate holds the main thread's
//! half: the parameter space, the modulation matrix (edited by the kit's
//! `mod.*` commands, compiled here and handed to the audio thread), the
//! patch compiled into the flat program the audio thread runs (the
//! `reagent_engine::patch` compiler, handed over the same way), the patch's
//! own state, preset files (the `preset.*` commands), the summary and the
//! guide agents read.

use std::sync::Arc;

use lspk_commands::{CommandSpec, Context, NoArgs, Permission};
use lspk_mod::curve::CurvePoint;
use lspk_mod::{Matrix, ProgramSender, mailbox};
use lspk_params::{ParamDescriptor, ParamId, ParamSpace};
use lspk_plugin::{
    AudioLayout, Host, NotePorts, ParamValues, Plugin, PluginError, PluginInfo, ProcessConfig,
    RegisterError, Registry as Commands, features,
};
use lspk_presets::{Compare, History};
use reagent_engine::engine::{self, Engine, mod_config};
use reagent_engine::modulation::{self, Needs, NoteCurve, Registry};
use reagent_engine::params::{FILTERS, global};
use reagent_engine::patch::{self, ChannelState, FilterState, Patch, PatchSender};
use reagent_engine::{describe, table};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod patches;
pub mod preset;

use preset::SlotState;

pub use reagent_engine::params;

/// The plug-in's ID.
pub const ID: &str = "com.loombridge.reagent";
/// The plug-in's name.
pub const NAME: &str = "Reagent";
/// The command that puts the Init patch back: every parameter at its
/// default and the modulation matrix empty.
pub const INIT_PATCH: &str = "reagent.init_patch";
/// The command that redraws the velocity or the key curve.
pub const SET_NOTE_CURVE: &str = "reagent.set_note_curve";
/// The command that swaps two macros' routes.
pub const SWAP_MACROS: &str = "reagent.swap_macros";
/// The command that applies and deletes the macros.
pub const APPLY_MACROS: &str = "reagent.apply_macros";

/// The arguments of [`SET_NOTE_CURVE`].
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetNoteCurveArgs {
    /// Which curve, `velocity` or `key`.
    which: WhichCurve,
    /// Its points, x and y from 0 to 1 in order, each with the bend of the
    /// segment after it, −1 to 1 (0 straight).
    points: Vec<CurvePoint>,
}

/// One of the two note curves.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum WhichCurve {
    /// The curve note-on velocity is mapped through (engine-core R99).
    Velocity,
    /// The key curve the note number is mapped through.
    Key,
}

impl From<WhichCurve> for NoteCurve {
    fn from(which: WhichCurve) -> Self {
        match which {
            WhichCurve::Velocity => Self::Velocity,
            WhichCurve::Key => Self::Key,
        }
    }
}

/// The arguments of [`SWAP_MACROS`].
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SwapMacrosArgs {
    /// The first macro, 1 to 8.
    first: usize,
    /// The second.
    second: usize,
}

/// How many slots [`SWAP_MACROS`] moved.
#[derive(Serialize, JsonSchema)]
pub struct SwapMacrosResult {
    /// The slots whose source or aux changed macro.
    swapped: usize,
}

/// What [`APPLY_MACROS`] changed.
#[derive(Serialize, JsonSchema)]
pub struct ApplyMacrosResult {
    /// How many parameters the macros' offsets moved.
    parameters: usize,
    /// How many macro routes were removed.
    removed: usize,
}

/// Every parameter, in the order hosts list them.
#[must_use]
pub fn params() -> Vec<ParamDescriptor> {
    reagent_engine::params::params()
}

/// Reagent's parameter space.
#[must_use]
pub fn space() -> ParamSpace {
    reagent_engine::params::space()
}

/// Reagent on the main thread.
pub struct Reagent {
    host: Host,
    registry: Arc<Registry>,
    matrix: Matrix,
    /// Where compiled matrices go, while the processor is active, and the
    /// sample rate they're compiled for.
    sender: Option<(ProgramSender, f64)>,
    /// Where compiled patches go, while the processor is active.
    patches: Option<PatchSender>,
    needs: Arc<Needs>,
    /// What the last program sent reads.
    sent: u32,
    /// Each oscillator slot's engines' state: kept from a preset that
    /// loaded, and written into the next preset saved.
    slots: Vec<SlotState>,
    /// The two patches A/B compare holds.
    compare: Compare,
    /// The patches before the preset changes, for undo and redo.
    history: History,
    /// What the interface named the command now running, for the undo
    /// history to record the step under ([`Plugin::run_command_labelled`]).
    undo_label: Option<String>,
}

impl std::fmt::Debug for Reagent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reagent")
            .field(
                "routes",
                &self.matrix.describe(&self.registry.sources).len(),
            )
            .finish_non_exhaustive()
    }
}

impl Reagent {
    /// The modulation matrix.
    #[must_use]
    pub fn matrix(&self) -> &Matrix {
        &self.matrix
    }

    /// The registries the matrix names its sources and destinations from.
    #[must_use]
    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// Copies the note curves' switches from the parameters into the matrix,
    /// which compiles them: true when one moved (engine-core R100, R101).
    fn sync_note_curves(&mut self) -> bool {
        let on = |id| self.host.param(id).unwrap_or(0.0) >= 0.5;
        let (glide, legato) = (on(global::CURVE_GLIDE), on(global::VELOCITY_LEGATO));
        let curves = &mut self.matrix.note_curves;
        let changed = curves.glide != glide || curves.velocity_legato != legato;
        curves.glide = glide;
        curves.velocity_legato = legato;
        changed
    }

    /// Compiles the matrix and hands it to the audio thread, if it runs.
    fn send_matrix(&mut self) {
        self.sync_note_curves();
        let Some((sender, rate)) = self.sender.as_mut() else {
            return;
        };
        sender.collect();
        let program = self.registry.compile(&self.matrix, mod_config(*rate));
        let needs = modulation::needs_of(&self.matrix, &self.registry.sources);
        // The union with what the program the audio thread may still be
        // running reads, so it never skips a source that program needs: the
        // last one sent if the audio thread has taken it, or anything still
        // marked if not.
        let running = if sender.pending() {
            self.needs.get()
        } else {
            self.sent
        };
        self.needs.set(running | needs);
        sender.send(program);
        self.sent = needs;
    }

    /// The patch compiled into a flat program: the oscillators, the sub and
    /// the noise that are on, the filters a signal reaches, and whether a
    /// voice waits for every envelope (engine-core R59, R65, R10). Allocates
    /// nothing.
    #[must_use]
    pub fn compile_patch(&self) -> Patch {
        let get = |id: ParamId| self.host.param(id).unwrap_or(0.0);
        let channels: [ChannelState; patch::CHANNELS] =
            std::array::from_fn(|s| ChannelState::of(s, &get));
        let filters: [FilterState; FILTERS] = std::array::from_fn(|f| FilterState::of(f, &get));
        Patch::compile(&channels, &filters)
    }

    /// Compiles the patch and hands it to the audio thread, which takes it
    /// at its next block's start: what runs is what the program says, and
    /// the engine's state — the voices, the oscillators' phases, the
    /// filters' tails, the envelopes and the LFOs — moves across untouched.
    /// Returns whether the processor was active to take it. The plug-in's
    /// commands call this when a patch changes.
    pub fn recompile(&mut self) -> bool {
        let patch = self.compile_patch();
        let Some(patches) = self.patches.as_mut() else {
            return false;
        };
        patches.send(patch);
        true
    }
}

impl Plugin for Reagent {
    type Processor = Engine;

    const INFO: PluginInfo = PluginInfo::new(ID, NAME)
        .vendor("Loombridge")
        .version(env!("CARGO_PKG_VERSION"))
        .url("https://github.com/loombridge/reagent")
        .description("A wavetable synthesiser.")
        .features(&[
            features::INSTRUMENT,
            features::SYNTHESIZER,
            features::STEREO,
        ])
        .audio(AudioLayout::instrument(2))
        .notes(NotePorts::INPUT);

    fn new(host: Host) -> Self {
        Self {
            host,
            registry: Registry::new(),
            matrix: Matrix::new(),
            sender: None,
            patches: None,
            needs: Arc::new(Needs::default()),
            sent: 0,
            slots: preset::default_slots(),
            compare: Compare::new(),
            history: History::new(preset::UNDO_PRESETS),
            undo_label: None,
        }
    }

    fn param_space(&self) -> Option<ParamSpace> {
        Some(self.registry.space.clone())
    }

    fn activate(
        &mut self,
        config: &ProcessConfig,
        values: &ParamValues,
    ) -> Result<Engine, PluginError> {
        let (mut sender, receiver) = mailbox();
        let (mut patches, patch_receiver) = patch::mailbox();
        self.sync_note_curves();
        let program = self
            .registry
            .compile(&self.matrix, mod_config(config.sample_rate));
        self.sent = modulation::needs_of(&self.matrix, &self.registry.sources);
        self.needs.set(self.sent);
        sender.send(program);
        patches.send(self.compile_patch());
        let mut engine = Engine::new(
            config.sample_rate,
            config.max_frames as usize,
            values,
            self.registry.clone(),
            table::default_table(),
            receiver,
            self.needs.clone(),
        );
        engine.attach_patches(patch_receiver);
        self.sender = Some((sender, config.sample_rate));
        self.patches = Some(patches);
        Ok(engine)
    }

    fn deactivate(&mut self, processor: Engine) {
        drop(processor);
        self.sender = None;
        self.patches = None;
    }

    fn save_state(&self) -> Vec<u8> {
        modulation::save(&self.matrix, &self.registry.sources)
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), PluginError> {
        self.matrix = modulation::load(data, &self.registry.sources, &self.registry.destinations)
            .map_err(PluginError::new)?;
        self.send_matrix();
        Ok(())
    }

    fn on_main_thread(&mut self) {
        // The note curves' switches live in the matrix, which the main
        // thread compiles; a host's change to them takes a new program.
        if self.sync_note_curves() {
            self.send_matrix();
        }
    }

    fn latency(&self) -> u32 {
        let quality = self
            .host
            .param(global::QUALITY)
            .unwrap_or(reagent_engine::params::DEFAULT_QUALITY as f64);
        engine::latency(quality)
    }

    fn summary(&self) -> Option<String> {
        let routes = self.matrix.describe(&self.registry.sources);
        Some(describe::summary(
            &self.registry.space,
            &self.registry.sources,
            &routes,
            &|id| self.host.param(id),
        ))
    }

    fn guide(&self) -> Option<String> {
        Some(describe::GUIDE.to_owned())
    }

    fn register_commands(&self, registry: &mut Commands) -> Result<(), RegisterError> {
        lspk_mod::register(registry)?;
        preset::register(registry)?;
        for spec in [
            CommandSpec::new(INIT_PATCH, "Init Patch")
                .description(
                    "Puts the Init patch back: every parameter at its default (oscillator A, a \
                     saw, through Filter 1, which is off), the modulation matrix empty and the \
                     velocity and key curves back to the diagonal. The host records each \
                     parameter change like a person's.",
                )
                .permission(Permission::Edit)
                .context(Context::Global)
                .args::<NoArgs>()
                .result::<()>(),
            CommandSpec::new(SET_NOTE_CURVE, "Set Note Curve")
                .description(
                    "Redraws the velocity or the key curve, the graph a note's velocity or note \
                     number is mapped through before it reaches the matrix (`velocity` and `key` \
                     sources). It changes the patch. Points left out leave the curve as `Init \
                     Patch` sets it, the diagonal.",
                )
                .permission(Permission::Edit)
                .context(Context::Global)
                .args::<SetNoteCurveArgs>()
                .result::<()>(),
            CommandSpec::new(SWAP_MACROS, "Swap Macros")
                .description(
                    "Swaps two macros' routes (the macros are numbered 1 to 8, as they are \
                     named): every route whose source or aux is one macro takes the other, and \
                     the other way round. The macros' own values stay where they are. It changes \
                     the patch, and returns how many slots moved.",
                )
                .permission(Permission::Edit)
                .context(Context::Global)
                .args::<SwapMacrosArgs>()
                .result::<SwapMacrosResult>(),
            CommandSpec::new(APPLY_MACROS, "Apply and Delete Macros")
                .description(
                    "Bakes the macros into the patch: adds each macro route's current offset \
                     into the parameter it drives, then removes every route whose source is a \
                     macro. A route's offset is the macro's own contribution, its curve at the \
                     macro's value through its polarity, amount and output. The host records \
                     each parameter change like a person's.",
                )
                .permission(Permission::Edit)
                .context(Context::Global)
                .args::<NoArgs>()
                .result::<ApplyMacrosResult>(),
        ] {
            registry.register(spec)?;
        }
        Ok(())
    }

    fn run_command(
        &mut self,
        id: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, PluginError> {
        if id == INIT_PATCH {
            for param in self.registry.space.descriptors() {
                self.host
                    .edit(param.id, param.default)
                    .map_err(|e| PluginError::new(e.to_string()))?;
            }
            self.matrix = Matrix::new();
            self.send_matrix();
            self.recompile();
            self.host.mark_dirty();
            return Ok(serde_json::Value::Null);
        }
        if id == SET_NOTE_CURVE {
            let args: SetNoteCurveArgs = arguments(args)?;
            modulation::set_note_curve(&mut self.matrix, args.which.into(), &args.points);
            self.send_matrix();
            self.host.mark_dirty();
            return Ok(serde_json::Value::Null);
        }
        if id == SWAP_MACROS {
            let args: SwapMacrosArgs = arguments(args)?;
            if !(1..=params::MACROS).contains(&args.first)
                || !(1..=params::MACROS).contains(&args.second)
            {
                return Err(PluginError::new(format!(
                    "macros are numbered 1 to {}",
                    params::MACROS
                )));
            }
            if args.first == args.second {
                return Err(PluginError::new(
                    "swapping a macro with itself moves nothing",
                ));
            }
            let swapped = modulation::swap_macros(
                &mut self.matrix,
                &self.registry.sources,
                args.first - 1,
                args.second - 1,
            );
            self.send_matrix();
            self.host.mark_dirty();
            return Ok(
                serde_json::to_value(SwapMacrosResult { swapped }).expect("a serialisable result")
            );
        }
        if id == APPLY_MACROS {
            let host = &self.host;
            let apply = modulation::apply_macros(
                &mut self.matrix,
                &self.registry.space,
                &self.registry.sources,
                &|m| host.param(global::macro_id(m)).unwrap_or(0.0) as f32 / 100.0,
                &|param| host.param(param),
            );
            for (param, value) in &apply.params {
                self.host
                    .edit(*param, *value)
                    .map_err(|e| PluginError::new(e.to_string()))?;
            }
            self.send_matrix();
            self.recompile();
            self.host.mark_dirty();
            return Ok(serde_json::to_value(ApplyMacrosResult {
                parameters: apply.params.len(),
                removed: apply.removed,
            })
            .expect("a serialisable result"));
        }
        if id.starts_with("mod.") {
            let registry = self.registry.clone();
            let result = self
                .matrix
                .run(id, args.clone(), &registry.sources, &registry.destinations)
                .map_err(|e| PluginError::new(e.to_string()))?;
            if id != lspk_mod::commands::LIST {
                self.send_matrix();
                self.host.mark_dirty();
            }
            return Ok(result);
        }
        if id.starts_with("preset.") {
            return preset::run(self, id, args);
        }
        Err(PluginError::new(format!("no command `{id}`")))
    }

    fn run_command_labelled(
        &mut self,
        id: &str,
        args: &serde_json::Value,
        undo_label: &str,
    ) -> Result<serde_json::Value, PluginError> {
        self.undo_label = Some(undo_label.to_owned());
        let result = self.run_command(id, args);
        self.undo_label = None;
        result
    }
}

/// The arguments of a command, or why they don't fit.
fn arguments<T: serde::de::DeserializeOwned>(args: &serde_json::Value) -> Result<T, PluginError> {
    serde_json::from_value(args.clone())
        .map_err(|e| PluginError::new(format!("the arguments don't fit: {e}")))
}
