//! The matching benchmark (R9.7): the reference set is fixed and Loombridge's
//! own, a run repeats whatever the thread count, a tool is scored like the
//! built-in ones, and the recorded results were made from this reference set.

use lspk_test::render::Output;
use reagent_cli::match_benchmark::{BenchmarkArgs, DRAWN, benchmark, references};
use reagent_engine::Registry;
use serde_json::Value;

lspk_rt::install_trap!();

/// A small run: one budget too small to find much, one seed, two references.
fn args(threads: usize) -> BenchmarkArgs {
    BenchmarkArgs {
        out: None,
        budgets: vec![4],
        references: 2,
        seeds: 1,
        threads: Some(threads),
        write_references: None,
        tool_name: "external".to_owned(),
        scratch: None,
        tool: Vec::new(),
    }
}

fn summary<'a>(report: &'a Value, tool: &str) -> &'a Value {
    &report["tools"][tool]["summary"]
}

#[test]
fn a_run_repeats_whatever_the_thread_count() {
    let one = benchmark(&args(1), None).expect("one thread");
    let three = benchmark(&args(3), None).expect("three threads");
    assert_eq!(one["references"], three["references"]);
    assert_eq!(one["tools"], three["tools"]);
    for tool in ["init", "match_sound-4", "random_search-4"] {
        assert!(one["tools"][tool].is_object(), "{tool}");
    }
}

#[test]
fn a_tool_that_returns_the_reference_scores_a_perfect_match() {
    let mut copy = |reference: &Output| Ok::<_, String>(reference.clone());
    let report = benchmark(&args(2), Some(("copy", &mut copy))).expect("a run");
    let perfect = summary(&report, "copy");
    assert_eq!(perfect["mean_distance"], 0.0);
    assert_eq!(perfect["mean_closeness"], 1.0);
    assert_eq!(perfect["mean_abs_error"]["pitch_cents"], 0.0);
    assert_eq!(perfect["nearest_on"], 2);
}

#[test]
fn a_tool_that_makes_silence_scores_the_worst_distance() {
    let mut silence = |_: &Output| Ok::<_, String>(Output::new(48_000.0, vec![vec![0.0; 4_800]]));
    let report = benchmark(&args(2), Some(("silence", &mut silence))).expect("a run");
    let worst = summary(&report, "silence");
    assert_eq!(worst["mean_distance"], 100.0);
    assert_eq!(worst["silent"], 2);
}

#[test]
fn a_failing_tool_stops_the_run_naming_the_reference() {
    let mut fails = |_: &Output| Err::<Output, _>("no sound".to_owned());
    let error = benchmark(&args(1), Some(("broken", &mut fails))).expect_err("a failure");
    assert!(
        error.contains("broken") && error.contains("drawn-00"),
        "{error}"
    );
}

#[test]
fn the_reference_set_is_fixed_and_the_recorded_results_were_made_from_it() {
    let set = references(&Registry::new(), usize::MAX).expect("references");
    let names: Vec<_> = set.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(set.len(), DRAWN + 4);
    assert_eq!(&names[..2], ["drawn-00", "drawn-01"]);
    assert_eq!(&names[DRAWN..], ["supersaw", "unison16", "bass", "pad"]);
    // Some the search can make, some it cannot.
    assert!(set.iter().any(|r| r.in_space));
    assert!(set.iter().any(|r| !r.in_space));

    let recorded: Value = serde_json::from_str(include_str!(
        "../../../evidence/match-benchmark/results.json"
    ))
    .expect("recorded results");
    let now: Vec<_> = set
        .iter()
        .map(|r| (r.name.as_str(), r.kind, r.in_space))
        .collect();
    let then: Vec<_> = recorded["references"]
        .as_array()
        .expect("references")
        .iter()
        .map(|r| {
            (
                r["name"].as_str().unwrap(),
                r["kind"].as_str().unwrap(),
                r["in_space"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(now, then);
}

#[test]
fn a_scratch_folder_inside_a_repository_is_refused() {
    let mut run = args(1);
    run.tool = vec!["true".to_owned()];
    run.scratch = Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scratch"));
    let error = reagent_cli::match_benchmark::run(&run).expect_err("refused");
    assert!(error.contains("inside a repository"), "{error}");
}

/// A command given after `--` is run on each reference and its WAV file read
/// back: `cp` returns the reference itself, which scores a perfect match.
#[cfg(unix)]
#[test]
fn a_command_is_run_on_each_reference_and_scored() {
    let dir = std::env::temp_dir().join(format!("reagent-r97-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a folder");
    let out = dir.join("results.json");
    let mut run = args(1);
    run.out = Some(out.clone());
    run.tool = vec!["cp".to_owned()];
    run.scratch = Some(dir.join("scratch"));
    reagent_cli::match_benchmark::run(&run).expect("a run");
    let report: Value = serde_json::from_slice(&std::fs::read(&out).expect("results")).unwrap();
    assert_eq!(summary(&report, "external")["mean_distance"], 0.0);
    std::fs::remove_file(out).expect("results removed");
    assert!(!dir.join("scratch").exists(), "the renders are deleted");
    std::fs::remove_dir(dir).expect("the folder is empty");
}
