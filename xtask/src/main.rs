//! Repository tasks for Reagent. Run `cargo xtask help` for the list.

mod private_terms;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::Value;

const USAGE: &str = "\
Usage: cargo xtask <task>

Tasks:
  check          Run every check: format, clippy, tests (cargo-nextest, then the
                 doctests), layers, deny, the deny self-test, and private terms
  layers         Check the crate layering rules
  deny-selftest  Prove that deny.toml still rejects an incompatible licence
  private-terms  Fail if a tracked file contains a word listed in .git/info/private-terms
                 (maintainers' checkouts only; skipped without it)
  help           Show this message
";

/// The cargo-nextest release the check is written for, and CI installs. Keep
/// in step with `.config/nextest.toml` and the CI workflow.
const NEXTEST_VERSION: &str = "0.9.146";

/// This crate's package name.
const XTASK: &str = "xtask";

/// The UI crates: Reagent's interface and the kit's widget set it's built
/// from.
const UI: &[&str] = &["reagent-ui", "lspk-ui"];

/// Crates that never depend on a UI crate or reach a UI toolkit, so they
/// build and test headless: the engine, the factory content and the
/// importers. xtask is here too, so every task builds without a windowing
/// system's headers.
const HEADLESS: &[&str] = &["reagent-engine", "reagent-content", "reagent-import", XTASK];

/// Windowing, GPU and UI-toolkit crates that headless crates must not reach.
const UI_TOOLKITS: &[&str] = &[
    "baseview", "eframe", "egui", "iced", "slint", "vizia", "wgpu", "winit",
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = workspace_root().and_then(|root| run(&root, &args));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let task = args.first().map(String::as_str);
    if let Some(extra) = args.get(1) {
        return Err(format!("unexpected `{extra}`\n\n{USAGE}"));
    }
    match task {
        Some("check") => check(root),
        Some("layers") => layers(root),
        Some("deny-selftest") => deny_selftest(root),
        Some("private-terms") => private_terms::check(root),
        None | Some("help" | "-h" | "--help") => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown task `{other}`\n\n{USAGE}")),
    }
}

/// Runs every check, reports each result, and fails if any failed.
fn check(root: &Path) -> Result<(), String> {
    require_cargo_deny()?;
    require_cargo_nextest()?;
    type Step<'a> = (&'a str, Box<dyn Fn() -> Result<(), String> + 'a>);
    let steps: Vec<Step<'_>> = vec![
        (
            "format",
            Box::new(|| cargo(root, &["fmt", "--all", "--check"])),
        ),
        (
            "clippy",
            Box::new(|| {
                cargo(
                    root,
                    &[
                        "clippy",
                        "--workspace",
                        "--all-targets",
                        "--all-features",
                        "--locked",
                        "--",
                        "-D",
                        "warnings",
                    ],
                )
            }),
        ),
        ("tests", Box::new(|| run_tests(root))),
        ("layers", Box::new(|| layers(root))),
        (
            "licences, advisories, bans, sources",
            Box::new(|| cargo(root, &["deny", "--locked", "check"])),
        ),
        ("deny self-test", Box::new(|| deny_selftest(root))),
        ("private terms", Box::new(|| private_terms::check(root))),
    ];

    let mut failed = Vec::new();
    for (name, step) in &steps {
        eprintln!("\n==> {name}");
        if let Err(message) = step() {
            eprintln!("{message}");
            failed.push(*name);
        }
    }

    eprintln!("\n==> summary");
    for (name, _) in &steps {
        let status = if failed.contains(name) {
            "FAILED"
        } else {
            "ok"
        };
        eprintln!("{status:>6}  {name}");
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("{} of {} checks failed", failed.len(), steps.len()))
    }
}

/// Runs the tests with cargo-nextest, each in a process of its own and as
/// many at once as there are cores, then the doctests, which nextest doesn't
/// run, with `cargo test --doc`. Both run, so one run reports every failure.
fn run_tests(root: &Path) -> Result<(), String> {
    let tests = cargo(
        root,
        &[
            "nextest",
            "run",
            "--workspace",
            "--all-features",
            "--locked",
        ],
    );
    let docs = cargo(
        root,
        &["test", "--doc", "--workspace", "--all-features", "--locked"],
    );
    tests.and(docs)
}

/// Enforces Reagent's layering:
///
/// 1. The headless crates never depend on `reagent-ui` or `lspk-ui`.
/// 2. The headless crates never reach a windowing, GPU or UI-toolkit crate.
///
/// The rules follow normal and build dependencies, transitively. Dev
/// dependencies don't ship, so they're not checked.
fn layers(root: &Path) -> Result<(), String> {
    let graph = DependencyGraph::from_metadata(&cargo_metadata(root)?)?;
    let violations = layer_violations(&graph);
    if violations.is_empty() {
        eprintln!("{} crates follow the layering rules", graph.members.len());
        Ok(())
    } else {
        Err(format!(
            "layering rules broken:\n  {}",
            violations.join("\n  ")
        ))
    }
}

/// Every way the graph breaks the layering rules, one line each.
fn layer_violations(graph: &DependencyGraph) -> Vec<String> {
    let mut violations = Vec::new();
    let names: BTreeSet<&str> = graph.members.iter().map(|m| graph.name(m)).collect();
    for headless in HEADLESS {
        if !names.contains(headless) {
            violations.push(format!(
                "{headless} is listed as headless but isn't in the workspace; update HEADLESS"
            ));
        }
    }
    for member in &graph.members {
        let name = graph.name(member);
        if !HEADLESS.contains(&name) {
            continue;
        }
        if let Some(path) = graph.path_to(member, |dep| UI.contains(&dep)) {
            violations.push(format!("{name} depends on a UI crate: {path}"));
        }
        if let Some(path) = graph.path_to(member, |dep| UI_TOOLKITS.contains(&dep)) {
            violations.push(format!("{name} reaches a UI crate: {path}"));
        }
    }
    violations
}

/// Runs cargo-deny against a fixture that depends on a GPL-2.0-only crate,
/// and fails unless deny.toml rejects it.
fn deny_selftest(root: &Path) -> Result<(), String> {
    require_cargo_deny()?;
    let manifest = root.join("xtask/fixtures/licence-reject/Cargo.toml");
    let output = Command::new(cargo_bin())
        .arg("deny")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--config")
        .arg(root.join("deny.toml"))
        .args(["check", "licenses"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("couldn't run cargo deny: {e}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        return Err(
            "deny.toml accepted a GPL-2.0-only dependency; the licence policy is broken".into(),
        );
    }
    if !stderr.contains("GPL-2.0-only") {
        return Err(format!(
            "cargo deny failed on the fixture, but not because of its GPL-2.0-only crate:\n{stderr}"
        ));
    }
    eprintln!("deny.toml rejects a GPL-2.0-only dependency, as intended");
    Ok(())
}

/// The resolved dependency graph from `cargo metadata`, by package ID.
struct DependencyGraph {
    members: Vec<String>,
    names: BTreeMap<String, String>,
    edges: BTreeMap<String, Vec<String>>,
}

impl DependencyGraph {
    fn from_metadata(metadata: &Value) -> Result<Self, String> {
        let malformed = |what: &str| format!("unexpected `cargo metadata` output: no {what}");

        let names = metadata["packages"]
            .as_array()
            .ok_or_else(|| malformed("packages"))?
            .iter()
            .filter_map(|p| Some((p["id"].as_str()?.to_owned(), p["name"].as_str()?.to_owned())))
            .collect();

        let members = metadata["workspace_members"]
            .as_array()
            .ok_or_else(|| malformed("workspace_members"))?
            .iter()
            .filter_map(|id| id.as_str().map(str::to_owned))
            .collect();

        let mut edges = BTreeMap::new();
        for node in metadata["resolve"]["nodes"]
            .as_array()
            .ok_or_else(|| malformed("resolve.nodes"))?
        {
            let Some(id) = node["id"].as_str() else {
                continue;
            };
            let deps = node["deps"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|dep| ships(dep))
                .filter_map(|dep| dep["pkg"].as_str().map(str::to_owned))
                .collect();
            edges.insert(id.to_owned(), deps);
        }

        Ok(Self {
            members,
            names,
            edges,
        })
    }

    fn name<'a>(&'a self, id: &'a str) -> &'a str {
        self.names.get(id).map_or(id, String::as_str)
    }

    /// The shortest dependency path from `start` to a package whose name
    /// matches, written as `a -> b -> c`, or `None` if there isn't one.
    fn path_to(&self, start: &str, matches: impl Fn(&str) -> bool) -> Option<String> {
        let mut previous: BTreeMap<&str, &str> = BTreeMap::new();
        let mut seen = BTreeSet::from([start]);
        let mut queue = VecDeque::from([start]);

        while let Some(id) = queue.pop_front() {
            for dep in self.edges.get(id).into_iter().flatten() {
                let dep = dep.as_str();
                if !seen.insert(dep) {
                    continue;
                }
                previous.insert(dep, id);
                if matches(self.name(dep)) {
                    let mut path = vec![self.name(dep)];
                    let mut at = dep;
                    while let Some(&before) = previous.get(at) {
                        path.push(self.name(before));
                        at = before;
                    }
                    path.reverse();
                    return Some(path.join(" -> "));
                }
                queue.push_back(dep);
            }
        }
        None
    }
}

/// Whether a dependency edge from `cargo metadata` ships in a build, that is,
/// whether it's a normal or build dependency rather than only a dev one.
fn ships(dep: &Value) -> bool {
    dep["dep_kinds"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|kind| kind["kind"].is_null() || kind["kind"] == "build")
}

fn cargo_metadata(root: &Path) -> Result<Value, String> {
    let output = Command::new(cargo_bin())
        .args([
            "metadata",
            "--format-version",
            "1",
            "--all-features",
            "--locked",
        ])
        .current_dir(root)
        .output()
        .map_err(|e| format!("couldn't run cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("couldn't parse cargo metadata: {e}"))
}

/// Runs a cargo command in the workspace root, streaming its output.
fn cargo(root: &Path, args: &[&str]) -> Result<(), String> {
    let status = Command::new(cargo_bin())
        .args(args)
        .current_dir(root)
        .status()
        .map_err(|e| format!("couldn't run cargo {}: {e}", args.join(" ")))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo {} failed", args.join(" ")))
    }
}

fn require_cargo_deny() -> Result<(), String> {
    let installed = Command::new(cargo_bin())
        .args(["deny", "--version"])
        .output()
        .is_ok_and(|output| output.status.success());
    if installed {
        Ok(())
    } else {
        Err(
            "cargo-deny isn't installed. Install it with `cargo install --locked cargo-deny`."
                .into(),
        )
    }
}

fn require_cargo_nextest() -> Result<(), String> {
    let installed = Command::new(cargo_bin())
        .args(["nextest", "--version"])
        .output()
        .is_ok_and(|output| output.status.success());
    if installed {
        Ok(())
    } else {
        Err(format!(
            "cargo-nextest isn't installed, and the check runs the tests with it. Install it \
             with `cargo install --locked cargo-nextest@{NEXTEST_VERSION}`."
        ))
    }
}

/// The cargo that invoked this task, so the pinned toolchain is used.
fn cargo_bin() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

/// The root of the workspace that `cargo xtask` was run in, asked of cargo
/// at run time rather than fixed when xtask was compiled, because a target
/// directory shared between checkouts can reuse a binary built in another.
fn workspace_root() -> Result<PathBuf, String> {
    let output = Command::new(cargo_bin())
        .args(["locate-project", "--workspace", "--message-format", "plain"])
        .output()
        .map_err(|e| format!("couldn't run cargo locate-project: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    let manifest = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    manifest
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("{} has no parent directory", manifest.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A graph of the headless crates plus `extra` members, with `edges`
    /// between packages named, not by ID.
    fn graph(extra: &[&str], edges: &[(&str, &str)]) -> DependencyGraph {
        let members: Vec<String> = HEADLESS
            .iter()
            .chain(extra)
            .map(|&name| name.to_owned())
            .collect();
        let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for &(from, to) in edges {
            map.entry(from.to_owned()).or_default().push(to.to_owned());
        }
        DependencyGraph {
            members,
            names: BTreeMap::new(),
            edges: map,
        }
    }

    #[test]
    fn reagent_as_planned_follows_the_rules() {
        let graph = graph(
            &[
                "reagent-ui",
                "reagent",
                "reagent-cli",
                "lspk-ui",
                "lspk-dsp",
            ],
            &[
                ("reagent", "reagent-ui"),
                ("reagent", "reagent-engine"),
                ("reagent-ui", "lspk-ui"),
                ("lspk-ui", "egui"),
                ("reagent-engine", "lspk-dsp"),
                ("reagent-import", "reagent-engine"),
                ("reagent-cli", "reagent-engine"),
            ],
        );
        assert_eq!(layer_violations(&graph), Vec::<String>::new());
    }

    #[test]
    fn a_headless_crate_reaching_a_ui_crate_is_caught() {
        let graph = graph(
            &["reagent-ui", "lspk-ui"],
            &[
                ("reagent-import", "reagent-engine"),
                ("reagent-engine", "reagent-ui"),
                ("reagent-content", "lspk-ui"),
            ],
        );
        let violations = layer_violations(&graph);
        for expected in [
            "reagent-engine depends on a UI crate: reagent-engine -> reagent-ui",
            "reagent-import depends on a UI crate: reagent-import -> reagent-engine -> reagent-ui",
            "reagent-content depends on a UI crate: reagent-content -> lspk-ui",
        ] {
            assert!(violations.contains(&expected.to_owned()), "{violations:?}");
        }
    }

    #[test]
    fn a_headless_crate_reaching_a_toolkit_is_caught() {
        let graph = graph(&[], &[("reagent-engine", "winit")]);
        assert_eq!(
            layer_violations(&graph),
            ["reagent-engine reaches a UI crate: reagent-engine -> winit"]
        );
    }

    #[test]
    fn a_missing_headless_crate_is_reported() {
        let mut graph = graph(&[], &[]);
        graph.members.retain(|name| name != "reagent-import");
        assert_eq!(layer_violations(&graph).len(), 1);
    }
}
