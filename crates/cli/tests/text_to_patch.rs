//! `reagent-cli text-to-patch`: the loop moves the control a quality word
//! names until its reading fits, a description with no instrument is
//! refused, the evaluation set runs and passes, and the recorded results
//! are what a fresh run gives.

use std::fs;
use std::path::PathBuf;

use reagent_cli::text_to_patch::{TextToPatchArgs, run};
use serde_json::Value;

lspk_rt::install_trap!();

fn out(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("reagent-t2p-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("a temporary folder");
    dir.join(name)
}

fn results(description: Option<&str>, name: &str) -> Value {
    let path = out(name);
    run(&TextToPatchArgs {
        description: description.map(str::to_owned),
        out: Some(path.clone()),
    })
    .expect("the loop runs");
    serde_json::from_str(&fs::read_to_string(path).expect("results")).expect("JSON")
}

#[test]
fn the_loop_closes_the_filter_until_a_lead_is_dark() {
    let result = results(Some("A dark lead."), "dark.json");
    let step = &result["steps"][0];
    assert_eq!(step["word"], "dark");
    assert!(
        step["renders"].as_u64().unwrap() > 1,
        "the recipe's own cutoff isn't dark"
    );
    assert_eq!(step["reached"], true);
    assert!(result["readings"]["brightness_oct"].as_f64().unwrap() <= 1.2);
}

#[test]
fn a_description_with_no_instrument_is_refused() {
    let error = run(&TextToPatchArgs {
        description: Some("A dark sound.".to_owned()),
        out: None,
    })
    .unwrap_err();
    assert!(error.contains("instrument"), "{error}");
}

#[test]
fn the_evaluation_set_passes_and_matches_the_recorded_results() {
    let fresh = results(None, "eval.json");
    assert_eq!(fresh["passed"], fresh["of"], "a case fails its judge");
    let recorded: Value = serde_json::from_str(
        &fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../evidence/text-to-patch/results.json"
        ))
        .expect("the recorded results"),
    )
    .expect("JSON");
    let verdicts = |v: &Value| -> Vec<(String, bool)> {
        v["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["description"].as_str().unwrap().to_owned(),
                    r["passed"] == true,
                )
            })
            .collect()
    };
    assert_eq!(
        verdicts(&fresh),
        verdicts(&recorded),
        "remake evidence/text-to-patch/results.json"
    );
}

#[test]
fn the_guide_has_a_recipe_for_each_instrument_word() {
    let guide = include_str!("../../engine/guide.md");
    for word in ["Bass", "Lead", "Pad", "Pluck"] {
        assert!(guide.contains(&format!("- **{word}:**")), "{word}");
    }
}
