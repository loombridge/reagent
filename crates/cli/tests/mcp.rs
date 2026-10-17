//! `reagent-cli mcp` driven by the kit's scripted MCP client over its real
//! standard input and output, headless: discovery with schemas, a batch as
//! one step and a dry run that changes nothing, a query by path, a preview
//! that returns audio and an analysis, a checkpoint that restores the
//! patch, and the permission rules (destructive and external commands are
//! refused, with the reason, unless `--autonomous`).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use lspk_test::agent::{Client, Stdio, audio, check_tools};
use reagent::params::{FILTER, filter, id};
use serde_json::{Value, json};

const CUTOFF: &str = r#"filter "1" / param "Cutoff""#;

/// The cutoff's parameter ID, which a modulation route names.
fn cutoff_id() -> u32 {
    id(FILTER, 0, filter::CUTOFF).0
}

/// A client of a new `reagent-cli mcp`, initialised.
fn start(autonomous: bool) -> Client<Stdio> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_reagent-cli"));
    command.arg("mcp");
    if autonomous {
        command.arg("--autonomous");
    }
    let mut client = Client::new(Stdio::spawn(&mut command).expect("reagent-cli mcp starts"));
    client.initialize("reagent-cli-test").expect("initialised");
    client
}

/// A tool's result, which must succeed.
fn call(client: &mut Client<Stdio>, tool: &str, args: Value) -> Value {
    client
        .call(tool, args)
        .expect("a reply")
        .ok()
        .unwrap_or_else(|e| panic!("{tool} failed: {e}"))
}

/// A tool's refusal or failure, which must be one.
fn fails(client: &mut Client<Stdio>, tool: &str, args: Value) -> String {
    client
        .call(tool, args)
        .expect("a reply")
        .ok()
        .expect_err("the call fails")
}

/// The cutoff's value now, in hertz.
fn cutoff(client: &mut Client<Stdio>) -> f64 {
    let state = call(client, "query", json!({ "params": [CUTOFF] }));
    state["params"][0]["value"].as_f64().expect("a value")
}

/// The commands of `step` run as one batch.
fn run(client: &mut Client<Stdio>, commands: Value, dry_run: bool) -> Value {
    call(
        client,
        "run",
        json!({ "commands": commands, "dry_run": dry_run }),
    )
}

fn set_cutoff(text: &str) -> Value {
    json!({ "command": "params.set",
        "args": { "values": [ { "param": CUTOFF, "text": text } ] } })
}

#[test]
fn an_agent_lists_describes_and_reads_the_guides_by_id() {
    let mut client = start(false);
    let tools = client.tools().expect("tools");
    check_tools(&tools).expect("well-described tools");
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert_eq!(
        names,
        [
            "list_commands",
            "describe_command",
            "run",
            "query",
            "checkpoint",
            "preview"
        ]
    );
    let guide = client
        .read_resource("lspk://guide")
        .expect("the kit's guide");
    assert!(guide.contains("Reagent"), "{guide}");
    let own = client
        .read_resource("lspk://plugin-guide")
        .expect("Reagent's guide");
    assert!(own.contains("Filter balance"), "{own}");

    let areas = call(&mut client, "list_commands", json!({}));
    assert!(areas["areas"]["preset"].as_u64() > Some(0), "{areas}");
    assert!(areas["areas"].get("agent").is_none(), "{areas}");
    let found = call(
        &mut client,
        "list_commands",
        json!({ "search": "load preset" }),
    );
    let load = found["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|c| c["id"] == "preset.load")
        .expect("preset.load");
    assert_eq!(load["permission"], "external");
    assert_eq!(load["access"], "refused to agents");
    let described = call(
        &mut client,
        "describe_command",
        json!({ "command": "params.set" }),
    );
    assert!(described["args"].is_object() && described["result"].is_object());
}

#[test]
fn a_batch_is_one_step_a_dry_run_changes_nothing_and_a_path_reads_the_value() {
    let mut client = start(false);
    let before = cutoff(&mut client);

    let dry = run(&mut client, json!([set_cutoff("400 Hz")]), true);
    assert_eq!(dry["dry_run"], true);
    assert_eq!(cutoff(&mut client), before);

    // A batch where the second command fails changes nothing.
    let error = fails(
        &mut client,
        "run",
        json!({ "commands": [ set_cutoff("400 Hz"), { "command": "nothing.here" } ] }),
    );
    assert!(error.contains("Nothing changed"), "{error}");
    assert_eq!(cutoff(&mut client), before);

    // A batch that works lands together: the cutoff and a modulation route.
    let done = run(
        &mut client,
        json!([
            { "command": "params.set", "args": { "values": [
                { "param": "Filter 1 On", "value": 1.0 } ] } },
            set_cutoff("400 Hz"),
            { "command": "mod.add_route", "args": {
                "source": "env2", "destination": { "param": cutoff_id() }, "amount": 0.4 } },
        ]),
        false,
    );
    assert_eq!(done["results"].as_array().map(Vec::len), Some(3));
    assert!((cutoff(&mut client) - 400.0).abs() < 0.5);
    let state = call(&mut client, "query", json!({}));
    let summary = state["summary"].as_str().expect("a summary");
    assert!(
        summary.contains("Filter 1 is a Ladder LP 12 at 400 Hz"),
        "{summary}"
    );
    assert!(
        summary.contains("Envelope 2 to Filter 1 Cutoff by +40 %"),
        "{summary}"
    );
}

#[test]
fn a_preview_returns_audio_and_an_analysis() {
    let mut client = start(false);
    let note = client
        .call(
            "preview",
            json!({ "notes": [ { "key": 69, "length": 0.5, "velocity": 1.0 } ] }),
        )
        .expect("a reply");
    let analysis = note.clone().ok().expect("a preview");
    assert_eq!(analysis["pitch"]["note"], "A4", "{analysis}");
    assert!(analysis["stereo"]["width"].is_number(), "{analysis}");
    assert!(
        analysis["spectrum"]["centroid_over_time_hz"].is_array(),
        "{analysis}"
    );
    let sound = audio(&note).expect("audio");
    assert!(sound.frames() > 0 && !sound.is_silent());
}

#[test]
fn a_checkpoint_puts_the_patch_back() {
    let mut client = start(false);
    let before = cutoff(&mut client);
    call(
        &mut client,
        "checkpoint",
        json!({ "action": "save", "name": "start" }),
    );
    run(
        &mut client,
        json!([
            set_cutoff("400 Hz"),
            { "command": "mod.add_route", "args": {
                "source": "env2", "destination": { "param": cutoff_id() }, "amount": 0.4 } },
        ]),
        false,
    );
    assert!((cutoff(&mut client) - 400.0).abs() < 0.5);
    let restored = call(
        &mut client,
        "checkpoint",
        json!({ "action": "restore", "name": "start" }),
    );
    assert!(restored["params_changed"].as_u64() > Some(0), "{restored}");
    assert_eq!(cutoff(&mut client), before);
    let routes = run(&mut client, json!([{ "command": "mod.list" }]), false);
    assert_eq!(routes["results"][0]["result"], json!([]));
}

/// A file in this process's temporary folder.
fn temporary(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("reagent-cli-mcp-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("a temporary folder");
    dir.join(name)
}

#[test]
fn files_are_refused_without_autonomous_and_written_with_it() {
    let path = temporary("agent.json");
    let _ = fs::remove_file(&path);
    let save = json!({ "commands": [ { "command": "preset.save",
        "args": { "path": path.to_string_lossy() } } ] });

    let mut client = start(false);
    let message = fails(&mut client, "run", save.clone());
    assert!(message.contains("`preset.save` is external"), "{message}");
    assert!(
        message.contains("`reagent-cli mcp` has no person to ask"),
        "{message}"
    );
    assert!(message.contains("--autonomous"), "{message}");
    assert!(!path.exists());
    // The refusal is for the whole batch, not only the file command.
    let batch = json!({ "commands": [ set_cutoff("400 Hz"), save["commands"][0].clone() ] });
    fails(&mut client, "run", batch);
    assert!(cutoff(&mut client) > 400.5);

    let mut client = start(true);
    run(&mut client, json!([set_cutoff("400 Hz")]), false);
    call(&mut client, "run", save);
    assert!(path.exists());
    // Another instance loads the file, and the preset history undoes it.
    let mut other = start(true);
    call(
        &mut other,
        "run",
        json!({ "commands": [ { "command": "preset.load",
            "args": { "path": path.to_string_lossy() } } ] }),
    );
    assert!((cutoff(&mut other) - 400.0).abs() < 0.5);
    let undone = run(&mut other, json!([{ "command": "preset.undo" }]), false);
    assert_eq!(undone["results"][0]["result"]["restored"], true);
    assert!(cutoff(&mut other) > 400.5);
}

#[test]
fn the_server_ends_with_its_input() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_reagent-cli"));
    command.arg("mcp");
    let stdio = Stdio::spawn(&mut command).expect("starts");
    assert!(stdio.finish().expect("exits").success());
}
