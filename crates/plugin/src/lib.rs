//! Reagent as a plug-in and a standalone app, through the plug-in kit:
//! exported as CLAP natively, with VST3 and, on macOS, Audio Unit v2 made
//! from it, and a standalone app, each with the agent interface
//! (`lspk_plugin_agent::WithAgent`) and the kit's generic editor until
//! Reagent's own interface (R2).
//!
//! The sound is `reagent-engine`'s. This crate holds the main thread's
//! half: the parameter space, the modulation matrix (edited by the kit's
//! `mod.*` commands, compiled here and handed to the audio thread), the
//! patch's own state, the summary and the guide agents read.
//!
//! Without the `export` feature (on by default) the crate is the plug-in's
//! types alone, for a host that compiles it in, as Beatlayer's native
//! devices do.

use std::sync::Arc;

use lspk_commands::{CommandSpec, Context, NoArgs, Permission};
use lspk_mod::{Matrix, ProgramSender, mailbox};
use lspk_params::{ParamDescriptor, ParamSpace};
use lspk_plugin::{
    AudioLayout, Host, NotePorts, ParamValues, Plugin, PluginError, PluginInfo, ProcessConfig,
    RegisterError, Registry as Commands, features,
};
use reagent_engine::engine::{self, Engine, mod_config};
use reagent_engine::modulation::{self, Needs, Registry};
use reagent_engine::params::global;
use reagent_engine::{describe, table};

pub mod patches;

pub use reagent_engine::params;

/// The plug-in's ID.
pub const ID: &str = "com.loombridge.reagent";
/// The plug-in's name.
pub const NAME: &str = "Reagent";
/// The command that puts the Init patch back: every parameter at its
/// default and the modulation matrix empty.
pub const INIT_PATCH: &str = "reagent.init_patch";

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
    needs: Arc<Needs>,
    /// What the last program sent reads.
    sent: u32,
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

    /// Compiles the matrix and hands it to the audio thread, if it runs.
    fn send_matrix(&mut self) {
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
            needs: Arc::new(Needs::default()),
            sent: 0,
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
        let program = self
            .registry
            .compile(&self.matrix, mod_config(config.sample_rate));
        self.sent = modulation::needs_of(&self.matrix, &self.registry.sources);
        self.needs.set(self.sent);
        sender.send(program);
        let engine = Engine::new(
            config.sample_rate,
            config.max_frames as usize,
            values,
            self.registry.clone(),
            table::default_table(),
            receiver,
            self.needs.clone(),
        );
        self.sender = Some((sender, config.sample_rate));
        Ok(engine)
    }

    fn deactivate(&mut self, processor: Engine) {
        drop(processor);
        self.sender = None;
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

    #[cfg(feature = "export")]
    fn editor(&mut self) -> Option<Box<dyn lspk_plugin::Editor>> {
        Some(Box::new(lspk_ui::generic_editor(NAME, &params())))
    }

    fn register_commands(&self, registry: &mut Commands) -> Result<(), RegisterError> {
        lspk_mod::register(registry)?;
        registry.register(
            CommandSpec::new(INIT_PATCH, "Init Patch")
                .description(
                    "Puts the Init patch back: every parameter at its default (oscillator A, a \
                     saw, through Filter 1, which is off) and the modulation matrix empty. The \
                     host records each parameter change like a person's.",
                )
                .permission(Permission::Edit)
                .context(Context::Global)
                .args::<NoArgs>()
                .result::<()>(),
        )
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
            self.host.mark_dirty();
            return Ok(serde_json::Value::Null);
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
        Err(PluginError::new(format!("no command `{id}`")))
    }
}

#[cfg(feature = "export")]
lspk_plugin::export_clap!(lspk_plugin_agent::WithAgent<Reagent>);
