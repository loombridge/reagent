//! `reagent-cli train-match` and `compare-match`: the model that proposes
//! where `match_sound` starts (R9.4), trained on datasets `reagent-cli
//! dataset` made, and measured on a dataset it never saw.
//!
//! A dataset record is read back as a patch (the parameters and routes it
//! lists), kept if the matching space can make it, and rendered again for the
//! perception features the model reads; the dataset's own features are the
//! simple ones, and its audio is optional, so nothing else is needed from the
//! folder. The note is the dataset's, which `dataset.json` records.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use clap::Parser;
use lspk_mod::Polarity;
use lspk_params::ParamId;
use lspk_test::features;
use reagent_core::learnt::{self, Model};
use reagent_core::matching::{self, Args, Space, Start};
use reagent_engine::Registry;
use serde_json::{Value, json};

use crate::Outcome;
use crate::dataset::{HOLD_SECONDS, KEY, RATE, RENDER_SECONDS};

/// How to train the model.
#[derive(Debug, Parser)]
pub struct TrainArgs {
    /// A dataset folder to train on; give it again for more.
    #[arg(long, required = true)]
    pub dataset: Vec<PathBuf>,
    /// Where to write the model, as `crates/core/src/match_model.bin` is.
    #[arg(long)]
    pub out: PathBuf,
    /// How many hidden units the network has.
    #[arg(long, default_value_t = 64)]
    pub hidden: usize,
    /// How many passes over the patches.
    #[arg(long, default_value_t = 80)]
    pub epochs: usize,
    /// The seed the network's weights and the order of the patches come from.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
    /// How many patches render at once; default one per core.
    #[arg(long)]
    pub threads: Option<usize>,
}

/// How to measure the model.
#[derive(Debug, Parser)]
pub struct CompareArgs {
    /// A dataset folder the model wasn't trained on.
    #[arg(long)]
    pub dataset: PathBuf,
    /// How many of its patches to match.
    #[arg(long, default_value_t = 20)]
    pub cases: usize,
    /// How many candidates each search renders.
    #[arg(long, default_value_t = 150)]
    pub evaluations: usize,
    /// How many patches render at once; default one per core.
    #[arg(long)]
    pub threads: Option<usize>,
}

fn threads(wanted: Option<usize>) -> usize {
    wanted
        .or_else(|| std::thread::available_parallelism().ok().map(Into::into))
        .unwrap_or(1)
        .max(1)
}

/// The points of the matching space that a dataset's patches are, in index
/// order; the patches it can't make (the sub, the noise) are left out.
fn points(dataset: &Path, space: &Space) -> Result<Vec<Vec<f64>>, String> {
    let read = |name: &str| {
        let path = dataset.join(name);
        fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
    };
    let manifest: Value = serde_json::from_str(&read("dataset.json")?)
        .map_err(|e| format!("{}: dataset.json: {e}", dataset.display()))?;
    if manifest["format"] != 1 || manifest["note"]["key"] != KEY {
        return Err(format!(
            "{}: not a dataset of format 1 playing key {KEY}",
            dataset.display()
        ));
    }
    let mut points = Vec::new();
    for line in read("patches.jsonl")?.lines() {
        let record: Value = serde_json::from_str(line)
            .map_err(|e| format!("{}: patches.jsonl: {e}", dataset.display()))?;
        let bad = || format!("{}: a patch record is malformed", dataset.display());
        let params: Vec<(ParamId, f64)> = record["params"]
            .as_array()
            .ok_or_else(bad)?
            .iter()
            .map(|p| Some((ParamId(p[0].as_u64()? as u32), p[1].as_f64()?)))
            .collect::<Option<_>>()
            .ok_or_else(bad)?;
        let routes: Vec<(&str, ParamId, f32, Polarity)> = record["routes"]
            .as_array()
            .ok_or_else(bad)?
            .iter()
            .map(|r| {
                Some((
                    r["source"].as_str()?,
                    ParamId(r["destination"].as_u64()? as u32),
                    r["amount"].as_f64()? as f32,
                    match r["polarity"].as_str()? {
                        "unipolar" => Polarity::Unipolar,
                        "bipolar" => Polarity::Bipolar,
                        _ => return None,
                    },
                ))
            })
            .collect::<Option<_>>()
            .ok_or_else(bad)?;
        points.extend(space.encode(&params, &routes));
    }
    Ok(points)
}

/// What `f` makes of each item, on `threads` threads, in order.
fn map<T: Sync, U: Send>(items: &[T], threads: usize, f: impl Fn(&T) -> U + Sync) -> Vec<U> {
    let chunk = items.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let workers: Vec<_> = items
            .chunks(chunk)
            .map(|items| scope.spawn(|| items.iter().map(&f).collect::<Vec<_>>()))
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("a render thread"))
            .collect()
    })
}

/// The dataset's note, played by the patch at `point`.
fn render(
    space: &Space,
    registry: &Registry,
    point: &[f64],
) -> Result<lspk_test::render::Output, String> {
    matching::render(
        &space.decode(point),
        registry,
        RATE,
        KEY,
        HOLD_SECONDS,
        RENDER_SECONDS,
    )
}

/// Trains the model and writes it.
pub fn train(args: &TrainArgs) -> Outcome {
    let space = Space::new();
    let registry = Registry::new();
    let mut points = Vec::new();
    for dataset in &args.dataset {
        points.extend(self::points(dataset, &space)?);
    }
    let samples: Vec<_> = map(&points, threads(args.threads), |point| {
        let output = render(&space, &registry, point).ok()?;
        let features = features::extract(&output).filter(|_| output.is_finite())?;
        Some((learnt::input(&features), point.clone()))
    })
    .into_iter()
    .flatten()
    .collect();
    if samples.is_empty() {
        return Err("no patch of the datasets can be trained on".to_owned());
    }
    let (model, loss) = Model::train(&samples, args.hidden, args.epochs, args.seed);
    fs::write(&args.out, model.to_bytes()).map_err(|e| format!("{}: {e}", args.out.display()))?;
    println!(
        "trained on {} patches of {}: mean squared error {loss:.4}; wrote {}",
        samples.len(),
        points.len(),
        args.out.display()
    );
    Ok(ExitCode::SUCCESS)
}

/// The evaluations after which a search's best distance was at or under
/// `target`, or `None` if it never was.
fn reached(history: &[(usize, f64)], target: f64) -> Option<usize> {
    history
        .iter()
        .find(|(_, best)| *best <= target)
        .map(|(done, _)| *done)
}

/// Matches each held-out patch from random points and from the model's
/// patch, with the same budget and seed, and reports how the two compare:
/// the final distances, and how many evaluations each search took to reach
/// the distance the random search ended at.
///
/// # Errors
///
/// The dataset can't be read, or a patch can't be rendered.
pub fn comparison(args: &CompareArgs) -> Result<Value, String> {
    let space = Space::new();
    let registry = Registry::new();
    let points = points(&args.dataset, &space)?;
    let points = &points[..args.cases.min(points.len())];
    let threads = threads(args.threads);
    let mut cases = Vec::new();
    for (n, point) in points.iter().enumerate() {
        let reference = render(&space, &registry, point)?;
        let run = |start| {
            let mut history = Vec::new();
            let found = matching::search(
                &reference,
                &Args {
                    evaluations: Some(args.evaluations),
                    seed: n as u64,
                    key: Some(KEY),
                    hold: Some(HOLD_SECONDS),
                    threads: Some(threads),
                    start,
                },
                &mut |p| history.push((p.evaluations, p.best_distance.unwrap_or(f64::INFINITY))),
                &AtomicBool::new(false),
            )?;
            Ok::<_, String>((found, history))
        };
        let (random, random_history) = run(Start::Random)?;
        let (learnt, learnt_history) = run(Start::Learnt)?;
        cases.push(json!({
            "case": n,
            "baseline": random.baseline,
            "random": random.distance,
            "learnt": learnt.distance,
            "random_reached_at": reached(&random_history, random.distance),
            "learnt_reached_random_at": reached(&learnt_history, random.distance),
        }));
    }
    let mean = |key: &str| {
        cases
            .iter()
            .map(|c| c[key].as_f64().unwrap_or(0.0))
            .sum::<f64>()
            / cases.len().max(1) as f64
    };
    let count = |f: fn(&Value) -> bool| cases.iter().filter(|c| f(c)).count();
    Ok(json!({
            "evaluations": args.evaluations,
            "mean_baseline": mean("baseline"),
            "mean_random": mean("random"),
            "mean_learnt": mean("learnt"),
            "closer": count(|c| c["learnt"].as_f64() < c["random"].as_f64()),
            "faster": count(|c| {
                let at = |key: &str| c[key].as_u64().unwrap_or(u64::MAX);
                at("learnt_reached_random_at") < at("random_reached_at")
            }),
            "of": cases.len(),
            "cases": cases,
    }))
}

/// Prints [`comparison`].
pub fn compare(args: &CompareArgs) -> Outcome {
    println!("{:#}", comparison(args)?);
    Ok(ExitCode::SUCCESS)
}
