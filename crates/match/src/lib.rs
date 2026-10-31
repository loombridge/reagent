//! The matching harness (R7.2): single-change cases rendered beside a
//! reference instrument and beside Reagent, then curves and levels fitted
//! from the pairs.
//!
//! A [`Case`] is one note played with some parameters changed. A
//! [`Reference`] renders it: [`Standin`] is Reagent itself, which is how CI
//! tests the harness, and [`External`] runs a command a maintainer supplies
//! that hosts the real reference headlessly, so nothing here names it or
//! needs it. [`sweep`] renders one parameter across values on both sides and
//! gives the [`Pairs`] that [`fit`] reads.
//!
//! Whatever a reference renders is written only to a scratch folder outside
//! any repository and deleted as soon as it's been measured; a report holds
//! numbers alone.

pub mod fit;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use lspk_params::{ParamId, ParamSpace};
use lspk_test::measure;
use lspk_test::render::{Instance, Output, Script, Settings};
use reagent_core::Reagent;
use reagent_core::patches::Patch;
use reagent_engine::Registry;
use serde::{Deserialize, Serialize};

/// One parameter set to one value, in the parameter's own unit: percent for
/// a level, hertz for a frequency, and a plain gain for a gain parameter such
/// as Main level.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Change {
    /// The parameter's name as a host shows it, such as `Osc A Level`.
    pub param: String,
    /// Its value.
    pub value: f64,
}

/// A note played on the reference's initial patch with `changes` made to it.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Case {
    /// What differs from the initial patch.
    pub changes: Vec<Change>,
    /// The MIDI key held.
    pub key: u8,
    /// The velocity, 0 to 1.
    pub velocity: f32,
    /// How long the render lasts, in seconds; the note holds throughout.
    pub seconds: f64,
    /// The sample rate, in hertz.
    pub rate: f64,
}

/// Something that renders a [`Case`].
pub trait Reference {
    /// The audio of `case`.
    ///
    /// # Errors
    ///
    /// The case names a parameter the reference doesn't have, or the
    /// render fails.
    fn render(&mut self, case: &Case) -> Result<Output, String>;
}

/// Reagent as a reference: its initial patch, with `base` applied under
/// every case so a test can give the stand-in a known difference.
pub struct Standin {
    space: ParamSpace,
    base: Vec<Change>,
}

impl Standin {
    /// A stand-in whose cases start from the initial patch with `base`.
    #[must_use]
    pub fn new(base: Vec<Change>) -> Self {
        Self {
            space: reagent_core::space(),
            base,
        }
    }
}

impl Reference for Standin {
    fn render(&mut self, case: &Case) -> Result<Output, String> {
        let mut patch = Patch::default();
        for change in self.base.iter().chain(&case.changes) {
            let id: ParamId = self
                .space
                .find(&change.param, &|_| None)
                .ok_or_else(|| format!("Reagent has no parameter named {}", change.param))?;
            patch.params.retain(|(set, _)| *set != id);
            patch.params.push((id, change.value));
        }
        let mut instance = Instance::<Reagent>::new()?;
        instance.load_state(&patch.state(&Registry::new()))?;
        instance.activate(Settings::new(case.rate, 128))?;
        let frames = (case.seconds * case.rate) as usize;
        Ok(instance
            .render(&Script::new(frames).note_on(0, case.key, case.velocity))
            .output)
    }
}

/// A reference hosted by a command: it's run as
/// `program args… <case.json> <out.wav>`, and must write the case's audio to
/// the WAV file and exit with status 0.
pub struct External {
    command: Vec<String>,
    scratch: PathBuf,
    count: usize,
}

impl External {
    /// A reference run by `command` (program first), writing under `scratch`.
    ///
    /// # Errors
    ///
    /// `command` is empty, or `scratch` is inside a git repository, where a
    /// render might be committed.
    pub fn new(command: Vec<String>, scratch: &Path) -> Result<Self, String> {
        if command.is_empty() {
            return Err("the reference command is empty".to_owned());
        }
        let full =
            std::path::absolute(scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
        if full.ancestors().any(|dir| dir.join(".git").exists()) {
            return Err(format!(
                "{} is inside a repository; renders belong outside one",
                full.display()
            ));
        }
        fs::create_dir_all(&full).map_err(|e| format!("{}: {e}", full.display()))?;
        Ok(Self {
            command,
            scratch: full,
            count: 0,
        })
    }
}

impl Reference for External {
    fn render(&mut self, case: &Case) -> Result<Output, String> {
        self.count += 1;
        let json = self.scratch.join(format!("case-{}.json", self.count));
        let wav = self.scratch.join(format!("out-{}.wav", self.count));
        let body = serde_json::to_vec(case).map_err(|e| e.to_string())?;
        fs::write(&json, body).map_err(|e| format!("{}: {e}", json.display()))?;
        let status = Command::new(&self.command[0])
            .args(&self.command[1..])
            .arg(&json)
            .arg(&wav)
            .status()
            .map_err(|e| format!("{}: {e}", self.command[0]))?;
        let audio = if status.success() {
            fs::read(&wav).map_err(|e| format!("{}: {e}", wav.display()))
        } else {
            Err(format!("{} failed: {status}", self.command[0]))
        }
        .and_then(|bytes| lspk_test::agent::decode_wav(&bytes));
        // The render is gone as soon as it's read, whatever came of it.
        let _ = fs::remove_file(&json);
        let _ = fs::remove_file(&wav);
        audio
    }
}

impl Drop for External {
    fn drop(&mut self) {
        // Only succeeds on an empty folder, so nothing else is touched.
        let _ = fs::remove_dir(&self.scratch);
    }
}

/// What is measured from a render, from 0.1 s in, past the attack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    /// The left channel's RMS, as a gain.
    Rms,
    /// Its pitch, in hertz.
    Pitch,
    /// Its spectral centroid, in hertz.
    Centroid,
}

impl Metric {
    /// This metric of `output`.
    ///
    /// # Errors
    ///
    /// The render is too short, or the metric isn't defined for it (silence
    /// has no pitch).
    pub fn of(self, output: &Output) -> Result<f64, String> {
        let from = (0.1 * output.sample_rate) as usize;
        let signal = output
            .channels
            .first()
            .and_then(|channel| channel.get(from..))
            .filter(|signal| !signal.is_empty())
            .ok_or("the render is shorter than 0.1 s")?;
        match self {
            Self::Rms => Some(measure::rms(signal)),
            Self::Pitch => measure::pitch(signal, output.sample_rate),
            Self::Centroid => measure::centroid(signal, output.sample_rate),
        }
        .ok_or_else(|| format!("{self:?} isn't defined for this render"))
    }
}

/// A sweep of one parameter.
#[derive(Clone, Debug)]
pub struct Sweep {
    /// The parameter in Reagent.
    pub param: String,
    /// The reference's name for it, when it differs.
    pub reference_param: Option<String>,
    /// The values, in the parameter's real unit.
    pub values: Vec<f64>,
    /// What to measure.
    pub metric: Metric,
    /// The key held.
    pub key: u8,
    /// Seconds per render.
    pub seconds: f64,
    /// The sample rate.
    pub rate: f64,
}

/// One value of a sweep and what each side measured at it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Pair {
    /// The parameter's value.
    pub x: f64,
    /// The reference's measurement.
    pub reference: f64,
    /// Reagent's.
    pub reagent: f64,
}

/// A sweep's pairs, in the order of its values.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Pairs(pub Vec<Pair>);

impl Pairs {
    /// The mean level the reference sits above Reagent, in dB, and the
    /// largest distance of any pair from that mean. For [`Metric::Rms`]
    /// sweeps; pairs where either side is silent are left out.
    #[must_use]
    pub fn level_offset_db(&self) -> Option<(f64, f64)> {
        let offsets: Vec<f64> = self
            .0
            .iter()
            .filter(|p| p.reference > 0.0 && p.reagent > 0.0)
            .map(|p| measure::db(p.reference / p.reagent))
            .collect();
        let mean = offsets.iter().sum::<f64>() / offsets.len().max(1) as f64;
        let spread = offsets.iter().map(|o| (o - mean).abs()).fold(0.0, f64::max);
        (!offsets.is_empty()).then_some((mean, spread))
    }
}

/// Renders `sweep` beside `reference` and `reagent`.
///
/// # Errors
///
/// Either side fails to render a value, or the metric is undefined for one.
pub fn sweep(
    sweep: &Sweep,
    reference: &mut dyn Reference,
    reagent: &mut dyn Reference,
) -> Result<Pairs, String> {
    let case = |param: &str, value: f64| Case {
        changes: vec![Change {
            param: param.to_owned(),
            value,
        }],
        key: sweep.key,
        velocity: 1.0,
        seconds: sweep.seconds,
        rate: sweep.rate,
    };
    let theirs = sweep.reference_param.as_deref().unwrap_or(&sweep.param);
    sweep
        .values
        .iter()
        .map(|&x| {
            Ok(Pair {
                x,
                reference: sweep.metric.of(&reference.render(&case(theirs, x))?)?,
                reagent: sweep.metric.of(&reagent.render(&case(&sweep.param, x))?)?,
            })
        })
        .collect::<Result<_, String>>()
        .map(Pairs)
}
