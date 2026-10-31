//! Reagent as a plug-in and a standalone app, through the plug-in kit:
//! exported as CLAP natively, with VST3 and, on macOS, Audio Unit v2 made
//! from it, and a standalone app, each with the agent interface
//! (`lspk_plugin_agent::WithAgent`) and the kit's generic editor until
//! Reagent's own interface (R2).
//!
//! The plug-in itself, its parameters, presets and patches, is
//! `reagent-core`'s, which reaches no UI toolkit; this crate re-exports it
//! and adds the editor and the exports.
//!
//! Without the `export` feature (on by default) the crate is the plug-in's
//! types alone, with no editor.

use std::ops::{Deref, DerefMut};

use lspk_params::ParamSpace;
use lspk_plugin::{
    Host, ParamValues, Plugin, PluginError, PluginInfo, ProcessConfig, RegisterError,
    Registry as Commands,
};
use reagent_engine::engine::Engine;

pub use reagent_core::*;

/// Reagent: [`reagent_core::Reagent`] with the generic editor.
#[derive(Debug)]
pub struct Reagent(reagent_core::Reagent);

impl Deref for Reagent {
    type Target = reagent_core::Reagent;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Reagent {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Plugin for Reagent {
    type Processor = Engine;

    const INFO: PluginInfo = reagent_core::Reagent::INFO;

    fn new(host: Host) -> Self {
        Self(reagent_core::Reagent::new(host))
    }

    fn param_space(&self) -> Option<ParamSpace> {
        self.0.param_space()
    }

    fn activate(
        &mut self,
        config: &ProcessConfig,
        values: &ParamValues,
    ) -> Result<Engine, PluginError> {
        self.0.activate(config, values)
    }

    fn deactivate(&mut self, processor: Engine) {
        self.0.deactivate(processor);
    }

    fn save_state(&self) -> Vec<u8> {
        self.0.save_state()
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), PluginError> {
        self.0.load_state(data)
    }

    fn on_main_thread(&mut self) {
        self.0.on_main_thread();
    }

    fn track_info_changed(&mut self) {
        self.0.track_info_changed();
    }

    fn latency(&self) -> u32 {
        self.0.latency()
    }

    fn summary(&self) -> Option<String> {
        self.0.summary()
    }

    fn guide(&self) -> Option<String> {
        self.0.guide()
    }

    #[cfg(feature = "export")]
    fn editor(&mut self) -> Option<Box<dyn lspk_plugin::Editor>> {
        Some(Box::new(lspk_ui::generic_editor_with(
            NAME,
            &params(),
            editor_actions(),
        )))
    }

    fn register_commands(&self, registry: &mut Commands) -> Result<(), RegisterError> {
        self.0.register_commands(registry)
    }

    fn run_command(
        &mut self,
        id: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, PluginError> {
        self.0.run_command(id, args)
    }

    fn run_command_labelled(
        &mut self,
        id: &str,
        args: &serde_json::Value,
        undo_label: &str,
    ) -> Result<serde_json::Value, PluginError> {
        self.0.run_command_labelled(id, args, undo_label)
    }
}

/// The commands the editor shows as buttons: undo, redo and A/B compare,
/// each labelled as its command is.
///
/// # Panics
///
/// If one of the commands isn't registered.
#[cfg(feature = "export")]
#[must_use]
pub fn editor_actions() -> Vec<lspk_ui::Action> {
    let mut registry = Commands::new();
    reagent_core::preset::register(&mut registry).expect("the preset commands register");
    [
        preset::UNDO,
        preset::REDO,
        preset::STORE_A,
        preset::STORE_B,
        preset::COMPARE,
    ]
    .map(|id| {
        let spec = registry.get(id).expect("a registered command");
        lspk_ui::Action::new(id, spec.label.to_string())
    })
    .into()
}

#[cfg(feature = "export")]
lspk_plugin::export_clap!(lspk_plugin_agent::WithAgent<Reagent>);
