//! `reagent-cli match-benchmark` (R9.7): how near sound matching gets to a
//! fixed set of references, by metrics anyone can recompute, with the tools
//! it is compared against.
//!
//! **The references** are made by Loombridge and rendered here, so nothing
//! third-party is committed: [`DRAWN`] patches the dataset sampler draws from
//! [`REFERENCE_SEED`] (some of them outside what `match_sound` can set: the
//! sub, the noise, an LFO on a frame) and the hand-built patches of
//! `reagent_core::patches`. Each plays one note, as the dataset does; the
//! search isn't told its key or how long it is held.
//!
//! **The metrics** are the kit's (R9.2): the distance between the
//! perception features of the reference and of what a tool made, and the
//! error in each reading a listener can name (brightness in octaves, pitch in
//! cents, level in dB, attack and duration in seconds, stereo width). The
//! distance is what `match_sound` minimises, so it favours `match_sound`
//! against a tool that doesn't optimise it; the readings don't.
//!
//! **The tools** are `match_sound` at each budget, the same search from
//! random points only (what the learnt starting point adds), the Init patch
//! (the floor), and any command given after `--`, run as
//! `program args… <reference.wav> <out.wav>`: it hears the reference and
//! writes the sound it made to match it. A command that drives some other
//! instrument headlessly is the user's to write; none ships here.
//! `--write-references` writes the references as WAV files, for matching them
//! by hand.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::atomic::AtomicBool;

use clap::Parser;
use lspk_mod::Target;
use lspk_test::features::{self, Features};
use lspk_test::render::Output;
use reagent_core::matching::{self, Args, Space, Start};
use reagent_core::patches;
use reagent_engine::Registry;
use serde_json::{Value, json};

use crate::Outcome;
use crate::dataset::{self, HOLD_SECONDS, KEY, RATE, RENDER_SECONDS};
use crate::text_to_patch::Readings;

/// The seed the drawn references come from.
pub const REFERENCE_SEED: u64 = 2026;
/// How many references are drawn from it.
pub const DRAWN: usize = 16;
/// What a sound that renders silence counts as: further than any distance
/// the kit's ranks.
const SILENT: f64 = 100.0;

/// How to run the benchmark.
#[derive(Debug, Parser)]
pub struct BenchmarkArgs {
    /// Write the results as JSON to this file instead of standard output.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// The candidate budgets `match_sound` is run at, separated by commas.
    #[arg(long, value_delimiter = ',', default_values_t = [150, 600])]
    pub budgets: Vec<usize>,
    /// Only the first this many references, of the 20; default all.
    #[arg(long, default_value_t = DRAWN + 4)]
    pub references: usize,
    /// How many seeds each search is run with; the scores are their means.
    #[arg(long, default_value_t = 3)]
    pub seeds: u64,
    /// How many patches render at once; default one per core.
    #[arg(long)]
    pub threads: Option<usize>,
    /// Write the references as WAV files to this folder and stop.
    #[arg(long)]
    pub write_references: Option<PathBuf>,
    /// What to call the command after `--` in the results.
    #[arg(long, default_value = "external")]
    pub tool_name: String,
    /// Where the command's renders are written, outside any repository.
    #[arg(long)]
    pub scratch: Option<PathBuf>,
    /// A tool to compare against: the command that matches a reference.
    #[arg(last = true)]
    pub tool: Vec<String>,
}

/// One of the sounds to match.
pub struct Reference {
    pub name: String,
    /// `drawn` by the sampler, or `designed` by hand.
    pub kind: &'static str,
    /// Whether `match_sound` could set a patch that makes it exactly.
    pub in_space: bool,
    pub output: Output,
}

/// The first `limit` references of the set, in a fixed order.
///
/// # Errors
///
/// A patch can't be rendered.
pub fn references(registry: &Registry, limit: usize) -> Result<Vec<Reference>, String> {
    let space = Space::new();
    let reference = |name: String, kind, patch: &patches::Patch, output| {
        let routes: Option<Vec<_>> = patch
            .routes
            .iter()
            .map(|(source, target, amount, polarity)| match target {
                Target::Param(destination) => Some((*source, *destination, *amount, *polarity)),
                _ => None,
            })
            .collect();
        Reference {
            name,
            kind,
            in_space: routes.is_some_and(|routes| space.encode(&patch.params, &routes).is_some()),
            output,
        }
    };
    let mut set = Vec::new();
    for index in 0..DRAWN.min(limit) {
        let (patch, _, output) = dataset::playable(REFERENCE_SEED, index, registry)?;
        set.push(reference(
            format!("drawn-{index:02}"),
            "drawn",
            &patch,
            output,
        ));
    }
    for patch in [
        patches::supersaw(),
        patches::unison16(),
        patches::bass(),
        patches::pad(),
    ] {
        if set.len() >= limit {
            break;
        }
        let output = matching::render(&patch, registry, RATE, KEY, HOLD_SECONDS, RENDER_SECONDS)?;
        if !output.is_finite() || features::extract(&output).is_none() {
            return Err(format!(
                "{} renders silence or a non-finite sample",
                patch.name
            ));
        }
        set.push(reference(patch.name.to_owned(), "designed", &patch, output));
    }
    Ok(set)
}

/// How near one sound is to a reference.
struct Score {
    distance: f64,
    /// Each reading's absolute error, where both sounds have it.
    errors: Vec<(&'static str, f64)>,
    silent: bool,
}

fn score(reference: &Features, made: &Output) -> Score {
    let Some(made) = made.is_finite().then(|| features::extract(made)).flatten() else {
        return Score {
            distance: SILENT,
            errors: Vec::new(),
            silent: true,
        };
    };
    let (a, b) = (Readings::of(reference), Readings::of(&made));
    let mut errors = vec![
        ("level_db", (a.level_db - b.level_db).abs()),
        ("attack_s", (a.attack_s - b.attack_s).abs()),
        ("duration_s", (a.duration_s - b.duration_s).abs()),
        ("width", (a.width - b.width).abs()),
    ];
    if let (Some(a), Some(b)) = (a.brightness_oct, b.brightness_oct) {
        errors.push(("brightness_oct", (a - b).abs()));
    }
    if let (Some(a), Some(b)) = (a.pitch_hz, b.pitch_hz) {
        errors.push(("pitch_cents", 1200.0 * (a / b).log2().abs()));
    }
    Score {
        distance: features::distance(reference, &made),
        errors,
        silent: false,
    }
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[middle]
    } else {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    }
}

/// Four decimals: the results are for reading, and a last digit that differs
/// between platforms isn't a result.
fn round(x: f64) -> f64 {
    (x * 1e4).round() / 1e4
}

/// A tool that matches a reference: what it hears and what it makes.
pub type External<'a> = (
    &'a str,
    &'a mut dyn FnMut(&Output) -> Result<Output, String>,
);

/// Runs the benchmark: every tool on every reference.
///
/// # Errors
///
/// A search or the external tool fails, or an argument is out of range.
pub fn benchmark(
    args: &BenchmarkArgs,
    mut external: Option<External<'_>>,
) -> Result<Value, String> {
    if args.budgets.is_empty()
        || args.budgets.contains(&0)
        || args.seeds == 0
        || args.references == 0
    {
        return Err("give at least one budget, seed and reference, none of them zero".to_owned());
    }
    let registry = Registry::new();
    let set = references(&registry, args.references)?;
    // tool -> reference -> one score per seed
    let mut scores: BTreeMap<String, Vec<Vec<Score>>> = BTreeMap::new();
    let mut seconds: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut init_distance = Vec::new();
    for (i, reference) in set.iter().enumerate() {
        let features = features::extract(&reference.output)
            .ok_or_else(|| format!("{} is silent", reference.name))?;
        let length =
            (reference.output.frames() as f64 / reference.output.sample_rate).clamp(0.25, 4.0);
        for &budget in &args.budgets {
            for (tool, start) in [
                ("match_sound", Start::Learnt),
                ("random_search", Start::Random),
            ] {
                let name = format!("{tool}-{budget}");
                for seed in 0..args.seeds {
                    let found = matching::search(
                        &reference.output,
                        &Args {
                            evaluations: Some(budget),
                            seed,
                            key: None,
                            hold: None,
                            threads: args.threads,
                            start,
                        },
                        &mut |_| {},
                        &AtomicBool::new(false),
                    )?;
                    let made = matching::render(
                        &found.patch,
                        &registry,
                        reference.output.sample_rate,
                        found.key,
                        found.hold,
                        length,
                    )?;
                    if init_distance.len() == i {
                        // The same key and hold for every search of this reference.
                        let init = matching::render(
                            &patches::init(),
                            &registry,
                            reference.output.sample_rate,
                            found.key,
                            found.hold,
                            length,
                        )?;
                        let init = score(&features, &init);
                        init_distance.push(init.distance);
                        scores
                            .entry("init".to_owned())
                            .or_insert_with(|| fresh(set.len()))[i]
                            .push(init);
                    }
                    seconds.entry(name.clone()).or_default().push(found.seconds);
                    scores
                        .entry(name.clone())
                        .or_insert_with(|| fresh(set.len()))[i]
                        .push(score(&features, &made));
                }
            }
        }
        if let Some((name, run)) = external.as_mut() {
            let made =
                run(&reference.output).map_err(|e| format!("{name} on {}: {e}", reference.name))?;
            scores
                .entry((*name).to_owned())
                .or_insert_with(|| fresh(set.len()))[i]
                .push(score(&features, &made));
        }
    }

    // Each tool's distance on each reference, the mean over its seeds.
    let distances: BTreeMap<&str, Vec<f64>> = scores
        .iter()
        .map(|(name, tool)| {
            let means = tool
                .iter()
                .map(|seeds| mean(&seeds.iter().map(|s| s.distance).collect::<Vec<_>>()))
                .collect();
            (name.as_str(), means)
        })
        .collect();
    // The tool nearest each reference, the Init patch not counting.
    let mut nearest = BTreeMap::<&str, usize>::new();
    for i in 0..set.len() {
        let best = distances
            .iter()
            .filter(|(name, _)| **name != "init")
            .min_by(|a, b| a.1[i].total_cmp(&b.1[i]));
        if let Some((name, _)) = best {
            *nearest.entry(name).or_default() += 1;
        }
    }
    let tools: BTreeMap<&str, Value> = scores
        .iter()
        .map(|(name, tool)| {
            let distances = &distances[name.as_str()];
            let closeness: Vec<f64> = distances
                .iter()
                .zip(&init_distance)
                .map(|(d, init)| 1.0 - d / init)
                .collect();
            let mut errors = BTreeMap::<&str, Vec<f64>>::new();
            for score in tool.iter().flatten() {
                for (reading, error) in &score.errors {
                    errors.entry(reading).or_default().push(*error);
                }
            }
            let summary = json!({
                "mean_distance": round(mean(distances)),
                "median_distance": round(median(distances)),
                "mean_closeness": round(mean(&closeness)),
                "nearest_on": nearest.get(name.as_str()).copied().unwrap_or(0),
                "silent": tool.iter().flatten().filter(|s| s.silent).count(),
                "mean_abs_error": errors.iter().map(|(reading, e)| (*reading, round(mean(e)))).collect::<BTreeMap<_, _>>(),
            });
            let rows: Vec<Value> = set
                .iter()
                .zip(distances.iter().zip(&closeness))
                .map(|(reference, (distance, closeness))| {
                    json!({
                        "reference": reference.name,
                        "distance": round(*distance),
                        "closeness": round(*closeness),
                    })
                })
                .collect();
            (name.as_str(), json!({ "summary": summary, "per_reference": rows }))
        })
        .collect();
    Ok(json!({
        "format": 1,
        "setup": {
            "sample_rate": RATE,
            "reference_seed": REFERENCE_SEED,
            "note": { "key": KEY, "hold_s": HOLD_SECONDS, "render_s": RENDER_SECONDS },
            "budgets": args.budgets,
            "seeds": args.seeds,
            "distance": "the plug-in kit's perception distance: 0 for the same features, about 1 for a difference a listener calls large, no upper bound; a sound that renders silence counts 100",
            "closeness": "1 minus the distance over the Init patch's: 1 for a match, 0 for no nearer than Init",
            "mean_abs_error": "brightness_oct in octaves, pitch_cents in cents, level_db in dB, attack_s and duration_s in seconds, width from 0 to 1",
        },
        "references": set.iter().zip(&init_distance).map(|(r, init)| json!({
            "name": r.name, "kind": r.kind, "in_space": r.in_space, "init_distance": round(*init),
        })).collect::<Vec<_>>(),
        "tools": tools,
        "mean_seconds": seconds.iter().map(|(name, s)| (name, round(mean(s)))).collect::<BTreeMap<_, _>>(),
    }))
}

fn fresh(references: usize) -> Vec<Vec<Score>> {
    (0..references).map(|_| Vec::new()).collect()
}

/// A command that matches a reference: run as `program args… <reference.wav>
/// <out.wav>` in `scratch`, which must be outside any git repository, where
/// a render might be committed. Its files are deleted once read.
fn command(
    command: &[String],
    scratch: &Path,
) -> Result<impl FnMut(&Output) -> Result<Output, String>, String> {
    let scratch =
        std::path::absolute(scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    if scratch.ancestors().any(|dir| dir.join(".git").exists()) {
        return Err(format!(
            "{} is inside a repository; renders belong outside one",
            scratch.display()
        ));
    }
    fs::create_dir_all(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let command = command.to_vec();
    Ok(move |reference: &Output| {
        let (heard, made) = (scratch.join("reference.wav"), scratch.join("made.wav"));
        crate::render::write(reference, &heard)?;
        let status = Command::new(&command[0])
            .args(&command[1..])
            .arg(&heard)
            .arg(&made)
            // Its output goes to stderr, so it can't corrupt the JSON results.
            .stdout(std::io::stderr())
            .status()
            .map_err(|e| format!("{}: {e}", command[0]));
        let audio = match status {
            Ok(status) if status.success() => fs::read(&made)
                .map_err(|e| format!("{}: {e}", made.display()))
                .and_then(|bytes| lspk_test::agent::decode_wav(&bytes)),
            Ok(status) => Err(format!("{} failed: {status}", command[0])),
            Err(e) => Err(e),
        };
        let _ = fs::remove_file(&heard);
        let _ = fs::remove_file(&made);
        audio
    })
}

/// Runs the benchmark, or writes the references, and prints the results.
pub fn run(args: &BenchmarkArgs) -> Outcome {
    if let Some(dir) = &args.write_references {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let set = references(&Registry::new(), args.references)?;
        for reference in &set {
            crate::render::write(
                &reference.output,
                &dir.join(format!("{}.wav", reference.name)),
            )?;
        }
        println!("wrote {} references to {}", set.len(), dir.display());
        return Ok(ExitCode::SUCCESS);
    }
    let mut external = match (args.tool.is_empty(), &args.scratch) {
        (true, _) => None,
        (false, Some(scratch)) => Some(command(&args.tool, scratch)?),
        (false, None) => return Err("a command to compare needs --scratch".to_owned()),
    };
    let report = benchmark(
        args,
        external
            .as_mut()
            .map(|run| (args.tool_name.as_str(), run as &mut dyn FnMut(&Output) -> _)),
    )?;
    if let Some(scratch) = &args.scratch {
        // Only succeeds on an empty folder, so nothing else is touched.
        let _ = fs::remove_dir(scratch);
    }
    let text = format!("{report:#}\n");
    match &args.out {
        Some(path) => fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?,
        None => print!("{text}"),
    }
    Ok(ExitCode::SUCCESS)
}
