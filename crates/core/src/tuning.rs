//! Tuning and preset stepping, on the main thread: the `tuning.*` commands,
//! the tuning a preset carries, and the lock that keeps the tuning through
//! a preset load.
//!
//! The engine does the retuning (`reagent_engine::tuning`); this holds what
//! the person set. The tuning file, how MTS-ESP is used and the lock belong
//! to the instance and are saved in its state; the file also goes into a
//! preset, and with the lock on a preset loads without it and without its
//! reference pitch. The controllers or keys that step presets are saved with
//! the instance.

use std::fs;

use lspk_commands::{
    CommandSpec, Context, NoArgs, Permission, RegisterError, Registry as Commands,
};
use lspk_plugin::PluginError;
use reagent_engine::stepping::Mode;
use reagent_engine::tuning::Table;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Reagent;

/// `tuning.load`: tune this instance by a file.
pub const LOAD: &str = "tuning.load";
/// `tuning.clear`: drop this instance's tuning file.
pub const CLEAR: &str = "tuning.clear";
/// `tuning.set_mts`: how the MTS-ESP client is used.
pub const SET_MTS: &str = "tuning.set_mts";
/// `tuning.lock`: keep the tuning through a preset load.
pub const LOCK: &str = "tuning.lock";
/// `preset.set_stepping`: assign the controllers or keys that step presets.
pub const SET_STEPPING: &str = "preset.set_stepping";

/// A tuning file: a `.tun`, or a Scala `.scl` with its keyboard mapping.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TuningFile {
    /// The `.tun` or `.scl` file.
    pub path: String,
    /// The Scala keyboard mapping (`.kbm`) for a `.scl`; without one the
    /// scale starts at middle C and A4 is 440 Hz.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keymap: Option<String>,
}

impl TuningFile {
    /// The table the file tunes.
    ///
    /// # Errors
    ///
    /// A file can't be read or isn't a tuning.
    pub fn read(&self) -> Result<Table, String> {
        let text = |path: &str| fs::read_to_string(path).map_err(|e| format!("{path}: {e}"));
        let tun = self.path.to_ascii_lowercase().ends_with(".tun");
        let scl = self.path.to_ascii_lowercase().ends_with(".scl");
        match (tun, scl, &self.keymap) {
            (true, _, None) => Table::from_tun(&text(&self.path)?),
            (_, true, keymap) => {
                let keymap = keymap.as_deref().map(text).transpose()?;
                Table::from_scala(&text(&self.path)?, keymap.as_deref())
            }
            (true, _, Some(_)) => Err("a keyboard mapping goes with a .scl, not a .tun".to_owned()),
            _ => Err(format!("{}: a tuning file ends in .tun or .scl", self.path)),
        }
        .map_err(|e| format!("{}: {e}", self.path))
    }
}

/// This instance's tuning, as its state saves it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Setup {
    /// The file that tunes it; it takes precedence over MTS-ESP.
    #[serde(default)]
    pub file: Option<TuningFile>,
    /// Whether the MTS-ESP client is on (it is by default).
    #[serde(default = "yes")]
    pub mts: bool,
    /// Whether MTS-ESP is read at a note's start only, so a held note keeps
    /// its pitch when the master retunes.
    #[serde(default)]
    pub note_on_only: bool,
    /// Whether a preset load leaves the tuning file and reference pitch as
    /// they are.
    #[serde(default)]
    pub locked: bool,
}

fn yes() -> bool {
    true
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            file: None,
            mts: true,
            note_on_only: false,
            locked: false,
        }
    }
}

/// What assigns the preset steps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StepMode {
    /// Nothing steps presets.
    #[default]
    Off,
    /// Two MIDI controller numbers.
    Controller,
    /// Two MIDI keys.
    Note,
}

/// The controllers or keys that step presets, as the instance's state saves
/// them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StepSetup {
    /// What assigns the steps.
    #[serde(default)]
    pub mode: StepMode,
    /// The controller number or key (0 to 127) that steps forward.
    #[serde(default)]
    pub forward: Option<u8>,
    /// The controller number or key (0 to 127) that steps back.
    #[serde(default)]
    pub backward: Option<u8>,
}

impl StepSetup {
    /// The engine's side of it.
    pub(crate) fn mode(self) -> Mode {
        match self.mode {
            StepMode::Off => Mode::Off,
            StepMode::Controller => Mode::Controller,
            StepMode::Note => Mode::Note,
        }
    }
}

/// The arguments of `tuning.load`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LoadArgs {
    /// The `.tun` or `.scl` file.
    pub path: String,
    /// The `.kbm` keyboard mapping for a `.scl`.
    #[serde(default)]
    pub keymap: Option<String>,
}

/// The arguments of `tuning.set_mts`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MtsArgs {
    /// Whether to follow an MTS-ESP master.
    pub enabled: bool,
    /// Whether to read the master at a note's start only.
    #[serde(default)]
    pub note_on_only: bool,
}

/// The arguments of `tuning.lock`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LockArgs {
    /// Whether a preset load leaves the tuning alone.
    pub locked: bool,
}

/// Adds the tuning commands, and the one that assigns preset stepping, to
/// `registry`.
///
/// # Errors
///
/// One of them is already registered.
pub fn register(registry: &mut Commands) -> Result<(), RegisterError> {
    registry.register(
        CommandSpec::new(LOAD, "Load Tuning")
            .description(
                "Tunes this instance by a tuning file: a `.tun` file, or a Scala `.scl` scale \
                 with its `.kbm` keyboard mapping in `keymap`. The file takes precedence over \
                 MTS-ESP, and it is saved with the instance and in the presets saved after. \
                 Reference pitch and Main tuning still apply on top. Keys the file leaves out \
                 play nothing. It reads files outside the project, so an agent asks the \
                 person first.",
            )
            .permission(Permission::External)
            .context(Context::Global)
            .args::<LoadArgs>()
            .result::<()>(),
    )?;
    registry.register(
        CommandSpec::new(CLEAR, "Clear Tuning")
            .description(
                "Drops this instance's tuning file: the instance follows an MTS-ESP master if \
                 one is connected, else equal temperament.",
            )
            .permission(Permission::Edit)
            .context(Context::Global)
            .args::<NoArgs>()
            .result::<()>(),
    )?;
    registry.register(
        CommandSpec::new(SET_MTS, "Set MTS-ESP")
            .description(
                "Turns the MTS-ESP client on or off (it is on by default), and chooses whether \
                 it reads the master at a note's start only, so a held note keeps its pitch \
                 when the master retunes. A loaded tuning file overrides MTS-ESP either way.",
            )
            .permission(Permission::Edit)
            .context(Context::Global)
            .args::<MtsArgs>()
            .result::<()>(),
    )?;
    registry.register(
        CommandSpec::new(LOCK, "Lock Tuning")
            .description(
                "Locks the tuning configuration: while it is on, a preset load leaves this \
                 instance's tuning file and Reference pitch as they are.",
            )
            .permission(Permission::Edit)
            .context(Context::Global)
            .args::<LockArgs>()
            .result::<()>(),
    )?;
    registry.register(
        CommandSpec::new(SET_STEPPING, "Set Preset Stepping")
            .description(
                "Assigns the controllers or keys that step through presets. With `mode` \
                 `controller`, a message of 64 or more on the `forward` or `backward` \
                 controller number steps to the next or previous preset (it must fall below 64 \
                 before it steps again); with `note`, a note-on of the `forward` or `backward` \
                 key steps, and the note doesn't sound. `off` stops stepping. The presets \
                 stepped through are the files beside the preset last loaded or saved.",
            )
            .permission(Permission::Edit)
            .context(Context::Global)
            .args::<StepSetup>()
            .result::<()>(),
    )
}

/// Runs the tuning command `id` with `args`.
///
/// # Errors
///
/// There's no such command, its arguments don't fit, or it failed.
pub fn run(plugin: &mut Reagent, id: &str, args: &Value) -> Result<Value, PluginError> {
    fn parse<T: serde::de::DeserializeOwned>(args: &Value) -> Result<T, PluginError> {
        serde_json::from_value(args.clone())
            .map_err(|e| PluginError::new(format!("the arguments don't fit: {e}")))
    }
    match id {
        LOAD => {
            let args: LoadArgs = parse(args)?;
            let file = TuningFile {
                path: args.path,
                keymap: args.keymap,
            };
            let table = file.read().map_err(PluginError::new)?;
            plugin.control.set_file(Some(&table));
            plugin.tuning.file = Some(file);
        }
        CLEAR => {
            plugin.tuning.file = None;
            plugin.control.set_file(None);
        }
        SET_MTS => {
            let args: MtsArgs = parse(args)?;
            plugin.tuning.mts = args.enabled;
            plugin.tuning.note_on_only = args.note_on_only;
            plugin.control.set_mts(args.enabled, args.note_on_only);
        }
        LOCK => plugin.tuning.locked = parse::<LockArgs>(args)?.locked,
        SET_STEPPING => {
            let setup: StepSetup = parse(args)?;
            if [setup.forward, setup.backward]
                .into_iter()
                .flatten()
                .any(|n| n > 127)
            {
                return Err(PluginError::new("controller numbers and keys are 0 to 127"));
            }
            plugin
                .stepping
                .set(setup.mode(), setup.forward, setup.backward);
            plugin.step_setup = setup;
        }
        _ => return Err(PluginError::new(format!("no command `{id}`"))),
    }
    plugin.host.mark_dirty();
    Ok(Value::Null)
}

impl Reagent {
    /// Puts the tuning the setup names into the engine's reach: the file's
    /// table, or none; and how MTS-ESP is read.
    pub(crate) fn apply_tuning(&mut self) -> Result<(), String> {
        self.control
            .set_mts(self.tuning.mts, self.tuning.note_on_only);
        let table = self
            .tuning
            .file
            .as_ref()
            .map(TuningFile::read)
            .transpose()?;
        self.control.set_file(table.as_ref());
        Ok(())
    }
}
