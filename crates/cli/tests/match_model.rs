//! The learnt starting point (R9.4): a model trains from a dataset's records
//! and writes a file the plug-in reads back, and on patches the model never
//! trained on, matching from its patch gets nearer than matching from random
//! points.

use std::path::PathBuf;

use reagent_cli::dataset::{self, DatasetArgs};
use reagent_cli::match_model::{CompareArgs, TrainArgs, comparison, train};
use reagent_core::learnt::Model;

lspk_rt::install_trap!();

fn dataset_in(name: &str, seed: u64, count: usize) -> PathBuf {
    let out = std::env::temp_dir().join(format!("reagent-r94-{}-{name}", std::process::id()));
    dataset::run(&DatasetArgs {
        out: out.clone(),
        count,
        seed,
        audio: false,
        threads: None,
    })
    .expect("a dataset");
    out
}

#[test]
fn training_writes_a_model_the_plug_in_reads_back() {
    let data = dataset_in("train", 5, 24);
    let out = data.join("model.bin");
    train(&TrainArgs {
        dataset: vec![data],
        out: out.clone(),
        hidden: 8,
        epochs: 2,
        seed: 0,
        threads: None,
    })
    .expect("training");
    let bytes = std::fs::read(out).expect("the model file");
    let model = Model::from_bytes(&bytes).expect("a model the plug-in reads");
    assert_eq!(model.to_bytes(), bytes);
}

#[test]
fn a_dataset_that_is_not_one_is_refused_with_its_name() {
    let error = comparison(&CompareArgs {
        dataset: std::env::temp_dir().join("reagent-r94-no-such-dataset"),
        cases: 1,
        evaluations: 2,
        threads: None,
    })
    .expect_err("no dataset");
    assert!(error.contains("reagent-r94-no-such-dataset"), "{error}");
}

/// The measurement `evidence/match-model/results.json` records: patches of a
/// dataset the model never saw, each matched from random points and from the
/// model's patch with 150 candidates.
#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "renders thousands of notes: run it with `cargo nextest run --release`"
)]
fn matching_from_the_models_patch_gets_nearer_than_from_random_points() {
    // The seed is not one the model trained on.
    let held_out = dataset_in("held-out", 1, 200);
    let result = comparison(&CompareArgs {
        dataset: held_out,
        cases: 40,
        evaluations: 150,
        threads: None,
    })
    .expect("a comparison");
    let (random, learnt) = (
        result["mean_random"].as_f64().unwrap(),
        result["mean_learnt"].as_f64().unwrap(),
    );
    assert!(learnt < random, "learnt {learnt}, random {random}");
    let (closer, of) = (
        result["closer"].as_u64().unwrap(),
        result["of"].as_u64().unwrap(),
    );
    assert!(closer * 2 > of, "closer in {closer} of {of}");
}
