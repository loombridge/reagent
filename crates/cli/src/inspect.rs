//! `reagent-cli inspect`: read a preset with `lspk-presets`' own reader and
//! print what it holds — its schema version, the modules it sets, each
//! non-default value in its real unit and its choice by name, and the
//! wavetables its oscillators name.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use lspk_params::ParamId;
use lspk_test::render::Instance;
use reagent::Reagent;
use reagent::params;
use reagent::preset;

use crate::Outcome;

/// What to inspect.
#[derive(Debug, Parser)]
pub struct InspectArgs {
    /// The preset file to read.
    #[arg(long)]
    pub preset: PathBuf,
}

/// What a preset holds, as `inspect` prints it.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    /// The preset's `format` field.
    pub format: String,
    /// The schema version it was written in.
    pub schema: u32,
    /// The modules with a value set, in name order.
    pub modules: Vec<String>,
    /// Each non-default value: its path and how it reads.
    pub values: Vec<(String, String)>,
    /// Each oscillator's wavetable, by oscillator name.
    pub wavetables: Vec<(String, String)>,
}

impl Report {
    /// The report as its text.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = format!(
            "format: {}\nschema: {} (this build reads up to {})\n",
            self.format,
            self.schema,
            lspk_presets::SCHEMA
        );
        out.push_str(&format!(
            "modules: {}\n",
            if self.modules.is_empty() {
                "none".to_owned()
            } else {
                self.modules.join(", ")
            }
        ));
        out.push_str("values:\n");
        if self.values.is_empty() {
            out.push_str("  (every value at its default)\n");
        }
        for (path, text) in &self.values {
            out.push_str(&format!("  {path} = {text}\n"));
        }
        out.push_str("wavetables:\n");
        for (oscillator, table) in &self.wavetables {
            out.push_str(&format!("  {oscillator}: {table}\n"));
        }
        out
    }
}

/// Reads `path` and prints what it holds.
pub fn run(args: &InspectArgs) -> Outcome {
    print!("{}", inspect(&args.preset)?.text());
    Ok(ExitCode::SUCCESS)
}

/// What the preset file at `path` holds.
///
/// # Errors
///
/// The file can't be read, or isn't a Reagent preset this build reads.
pub fn inspect(path: &Path) -> Result<Report, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let raw: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| format!("{}: it isn't JSON: {error}", path.display()))?;
    let instance = Instance::<Reagent>::new()?;
    let values = instance.values();
    let preset = preset::read(path, values, instance.plugin().registry())
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let space = params::space();
    let value_of = |id: ParamId| {
        preset
            .value(id)
            .or_else(|| values.descriptor(id).map(|d| d.default))
    };

    let mut modules = Vec::new();
    let mut rows = Vec::new();
    for (id, value) in &preset.values {
        let view = space
            .view(*id, &value_of)
            .ok_or_else(|| format!("{}: parameter {} isn't Reagent's", path.display(), id.0))?;
        if !view.module.is_empty() && !modules.contains(&view.module) {
            modules.push(view.module.clone());
        }
        rows.push((view.path.to_string(), view.format(*value)));
    }
    modules.sort();

    let mut wavetables = Vec::new();
    if let Some(oscillators) = preset.state.get("oscillators").and_then(|v| v.as_array()) {
        for (at, slot) in oscillators.iter().enumerate() {
            let table = slot
                .get("wavetable")
                .and_then(|w| w.get("table"))
                .and_then(|t| t.as_str());
            if let (Some(name), Some(table)) = (params::OSC_NAMES.get(at), table) {
                wavetables.push((format!("Osc {name}"), table.to_owned()));
            }
        }
    }

    Ok(Report {
        format: raw
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or(preset::FORMAT)
            .to_owned(),
        schema: raw
            .get("schema")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(u64::from(lspk_presets::SCHEMA)) as u32,
        modules,
        values: rows,
        wavetables,
    })
}
