//! The matching harness, with Reagent standing in for the reference: a
//! known curve is fitted back, a known level difference is measured, and a
//! reference that's a command is run the way the real one is.

use std::path::PathBuf;

use lspk_params::db_to_gain;
use reagent_match::fit::{Law, fit};
use reagent_match::{Change, External, Metric, Pairs, Standin, Sweep, sweep};

/// A sweep of `Osc A Level` in percent: Level's gain is its square (spec:
/// engine-core R7), so the RMS follows a power law of exponent 2.
fn level_sweep() -> Sweep {
    Sweep {
        param: "Osc A Level".to_owned(),
        reference_param: None,
        values: vec![20.0, 35.0, 50.0, 65.0, 80.0, 100.0],
        metric: Metric::Rms,
        key: 57,
        seconds: 0.4,
        rate: 48_000.0,
    }
}

fn curve(pairs: &Pairs, pick: fn(&reagent_match::Pair) -> f64) -> reagent_match::fit::Fit {
    let points: Vec<_> = pairs.0.iter().map(|p| (p.x, pick(p))).collect();
    fit(&points).expect("a fit")
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("reagent-match-{name}-{}", std::process::id()))
}

#[test]
fn it_fits_a_known_curve_and_level_from_a_standin_run_as_a_command() {
    // The reference is Reagent again, run as an external command, with Main
    // level raised 6 dB (to −3 dB, from its −9 dB default). Gain parameters
    // take their value as a gain.
    let command = vec![
        env!("CARGO_BIN_EXE_reagent-match").to_owned(),
        "standin".to_owned(),
        "--set".to_owned(),
        format!("Main level={}", db_to_gain(-3.0)),
    ];
    let pairs = sweep(
        &level_sweep(),
        &mut External::new(command, &scratch("fit")).expect("a reference"),
        &mut Standin::new(Vec::new()),
    )
    .expect("a sweep");

    for fitted in [curve(&pairs, |p| p.reference), curve(&pairs, |p| p.reagent)] {
        let Law::Power { exponent } = fitted.law else {
            panic!("{:?}, not a power law", fitted.law)
        };
        assert!((exponent - 2.0).abs() < 0.02, "exponent {exponent}");
    }
    let (mean, spread) = pairs.level_offset_db().expect("a level");
    assert!((mean - 6.0).abs() < 0.05, "{mean} dB, not 6 dB");
    assert!(spread < 0.05, "spread {spread} dB");
}

#[test]
fn a_standin_with_a_base_change_differs_by_that_change() {
    let base = vec![Change {
        param: "Main level".to_owned(),
        value: db_to_gain(-15.0),
    }];
    let pairs = sweep(
        &level_sweep(),
        &mut Standin::new(base),
        &mut Standin::new(Vec::new()),
    )
    .expect("a sweep");
    let (mean, _) = pairs.level_offset_db().expect("a level");
    assert!((mean + 6.0).abs() < 0.05, "{mean} dB, not −6 dB");
}

#[test]
fn a_reference_that_fails_is_an_error_and_leaves_nothing_behind() {
    let dir = scratch("fails");
    let command = vec![
        env!("CARGO_BIN_EXE_reagent-match").to_owned(),
        "standin".to_owned(),
        "--set".to_owned(),
        "No such=1".to_owned(),
    ];
    let mut reference = External::new(command, &dir).expect("a reference");
    let error = sweep(
        &level_sweep(),
        &mut reference,
        &mut Standin::new(Vec::new()),
    )
    .expect_err("a failing reference");
    assert!(error.contains("failed"), "{error}");
    assert_eq!(
        std::fs::read_dir(&dir).expect("the scratch folder").count(),
        0,
        "no render or case file is left"
    );
}

#[test]
fn renders_are_refused_inside_a_repository() {
    let inside = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scratch");
    let error = External::new(vec!["true".to_owned()], &inside)
        .err()
        .expect("a refusal");
    assert!(error.contains("inside a repository"), "{error}");
    assert!(!inside.exists());
}

#[test]
fn fit_tells_the_laws_apart() {
    let law = |f: fn(f64) -> f64| {
        let points: Vec<_> = (1..=8).map(|i| (f64::from(i), f(f64::from(i)))).collect();
        fit(&points).expect("a fit").law
    };
    assert_eq!(law(|x| 3.0 + 2.0 * x), Law::Linear);
    assert_eq!(law(|x| 1.0 + 4.0 * x.ln()), Law::Log);
    assert_eq!(law(|x| 0.5 * (0.7 * x).exp()), Law::Exp);
    let Law::Power { exponent } = law(|x| 0.1 * x.powf(1.5)) else {
        panic!("not a power law")
    };
    assert!((exponent - 1.5).abs() < 0.02);
    assert!(
        fit(&[(1.0, 1.0), (2.0, 2.0)]).is_none(),
        "two points fit nothing"
    );
}
