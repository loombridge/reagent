//! Reagent's preset files, driven as agents and the host drive the
//! commands: every module's parameters round-tripping through a file, a
//! file from an older schema, malformed files refused rather than crashing,
//! A/B compare, undo and redo, and the preset commands over the plug-in's
//! own agent server.
//!
//! `preset.save` and `preset.load` write and read files outside the
//! project, so they are `external`: an agent asks the person first. The
//! tests here drive them through the same command entry agents reach after
//! that answer; the Edit commands (A/B, undo, redo) are driven through the
//! real agent server too.

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;

use lspk_agent::client::{Client, Http};
use lspk_agent::discovery::{Discovery, Entry};
use lspk_params::{ParamDescriptor, ParamId, ParamKind};
use lspk_plugin::Plugin;
use lspk_test::agent::{check_tools, serving};
use lspk_test::clap::{ClapTestHost, PluginEvent};
use lspk_test::render::{Instance, Settings};
use reagent::params::{FILTER, OSC, channel, filter, global, id, osc};
use reagent::preset;
use reagent::{ID, INIT_PATCH, Reagent};
use serde_json::{Value, json};

const RATE: f64 = 48_000.0;
const BLOCK: usize = 128;

/// A file under the system's temporary folder, removed when it drops.
struct Scratch(PathBuf);

impl Scratch {
    fn file(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("reagent-preset-{}-{name}", std::process::id()));
        let _ = fs::remove_file(&path);
        Self(path)
    }

    fn path(&self) -> PathBuf {
        self.0.clone()
    }

    fn text(&self) -> String {
        fs::read_to_string(&self.0).expect("the preset file")
    }

    fn write(&self, text: &str) {
        fs::write(&self.0, text).expect("the preset file is writable");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Runs the command `id` with `args`, as an agent or the host does, and
/// checks it succeeds.
fn run(instance: &mut Instance<Reagent>, id: &str, args: Value) -> Value {
    instance
        .plugin_mut()
        .run_command(id, &args)
        .unwrap_or_else(|error| panic!("{id}: {error}"))
}

/// A value other than `descriptor`'s default, for a patch that sets
/// everything: the next choice, the opposite switch, or the far end of the
/// range from the value now.
fn changed(descriptor: &ParamDescriptor, now: f64) -> f64 {
    match &descriptor.kind {
        ParamKind::Choice { choices } => {
            let at = now.round().max(0.0) as usize;
            ((at + 1) % choices.len()) as f64
        }
        ParamKind::Toggle => {
            if now < 0.5 {
                1.0
            } else {
                0.0
            }
        }
        _ => {
            let (min, max) = (descriptor.min(), descriptor.max());
            if (now - min).abs() >= (max - now).abs() {
                min
            } else {
                max
            }
        }
    }
}

/// Sets every parameter of every module to a value other than its default,
/// one edit each.
fn set_every_parameter(instance: &Instance<Reagent>) {
    let values = instance.values();
    for (index, descriptor) in values.descriptors().iter().enumerate() {
        let value = changed(descriptor, values.value_at(index));
        instance
            .host()
            .edit(descriptor.id, value)
            .unwrap_or_else(|error| panic!("{} ({value}): {error}", descriptor.name));
    }
}

#[test]
fn every_modules_parameters_round_trip_through_a_preset_file() {
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance
        .activate(Settings::new(RATE, BLOCK))
        .expect("activation");
    set_every_parameter(&instance);
    // A known choice, so the file's symbolic names are checked against
    // something that doesn't move with the defaults.
    let last = (reagent::params::FILTER_TYPES.len() - 1) as f64;
    instance
        .host()
        .edit(id(FILTER, 0, filter::TYPE), last)
        .expect("an edit");
    // Two routes, so the preset's own state is in the file too.
    run(
        &mut instance,
        "mod.add_route",
        json!({ "source": "lfo2",
                "destination": { "param": id(FILTER, 0, filter::CUTOFF).0 },
                "amount": 0.5 }),
    );
    run(
        &mut instance,
        "mod.add_route",
        json!({ "source": "velocity",
                "destination": { "param": global::AMP.0 },
                "amount": -0.25 }),
    );
    instance.process(BLOCK, &[], &[]);

    let file = Scratch::file("every.reagent");
    let saved = run(&mut instance, preset::SAVE, json!({ "path": file.path() }));
    assert_eq!(saved["path"], file.path().to_string_lossy().as_ref());
    let text = file.text();
    assert!(text.contains("\"format\": \"reagent-preset\""), "{text}");
    assert!(text.contains("\"schema\": 1"), "{text}");
    // Choices are written by their names and switches as true or false.
    let last_type = reagent::params::FILTER_TYPES[last as usize];
    assert!(text.contains(last_type), "a choice by name: {last_type}");
    assert!(text.contains("\"value\": true") || text.contains("\"value\": false"));

    // Every value now, and the matrix, as the patch keeps them.
    let before: Vec<(ParamId, f64)> = instance
        .values()
        .descriptors()
        .iter()
        .map(|d| (d.id, instance.values().get(d.id).expect("a value")))
        .collect();
    let routes = run(&mut instance, "mod.list", json!({}));
    assert_eq!(routes.as_array().map(Vec::len), Some(2), "{routes}");

    // The Init patch, then the file: everything comes back.
    run(&mut instance, INIT_PATCH, json!(null));
    instance.process(BLOCK, &[], &[]);
    assert_eq!(
        instance
            .plugin()
            .matrix()
            .describe(&instance.plugin().registry().sources)
            .len(),
        0
    );
    run(&mut instance, preset::LOAD, json!({ "path": file.path() }));
    for (id, value) in &before {
        assert_eq!(instance.values().get(*id), Some(*value), "{id:?}");
    }
    assert_eq!(run(&mut instance, "mod.list", json!({})), routes);

    // The file is the kit's format, and holds the slots' engines' state.
    let saved: Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(
        saved["state"]["oscillators"][0]["wavetable"]["table"],
        "reagent/basic-shapes"
    );
}

#[test]
fn a_preset_from_an_older_schema_loads() {
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    // The format's draft, schema 0: `[id, value]` pairs, a choice by its
    // index and a switch as 0 or 1.
    let draft = Scratch::file("draft.reagent");
    draft.write(
        &json!({
            "format": "reagent-preset",
            "schema": 0,
            "params": [
                [id(OSC, 0, osc::OCTAVE).0, -2.0],
                [id(OSC, 0, osc::KEY_TRACK).0, 0],
                [id(FILTER, 0, filter::TYPE).0, 5],
            ],
            "state": {},
        })
        .to_string(),
    );
    run(&mut instance, preset::LOAD, json!({ "path": draft.path() }));
    assert_eq!(instance.values().get(id(OSC, 0, osc::OCTAVE)), Some(-2.0));
    assert_eq!(instance.values().get(id(OSC, 0, osc::KEY_TRACK)), Some(0.0));
    // SVF LP 12 is the choice at index 5.
    assert_eq!(
        instance.values().get(id(FILTER, 0, filter::TYPE)),
        Some(5.0)
    );
}

#[test]
fn every_oscillator_slots_engines_state_is_kept() {
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    // A preset with a later engine's state beside the wavetable's: this
    // build doesn't play a granular engine, and keeps its state anyway.
    let state = json!({
        "format": "reagent-preset",
        "schema": 1,
        "params": [],
        "state": { "oscillators": [
            { "wavetable": { "table": "reagent/basic-shapes" },
              "granular": { "size": 0.25, "density": 8.0 } },
            { "wavetable": { "table": "reagent/basic-shapes" } },
            {},
        ]},
    });
    let file = Scratch::file("engines.reagent");
    file.write(&state.to_string());
    run(&mut instance, preset::LOAD, json!({ "path": file.path() }));

    let saved = Scratch::file("engines-saved.reagent");
    run(&mut instance, preset::SAVE, json!({ "path": saved.path() }));
    let written: Value = serde_json::from_str(&saved.text()).expect("JSON");
    assert_eq!(
        written["state"]["oscillators"][0]["granular"],
        json!({ "size": 0.25, "density": 8.0 })
    );
    assert_eq!(
        written["state"]["oscillators"][0]["wavetable"]["table"],
        "reagent/basic-shapes"
    );
    // The slot that named nothing plays the default table.
    assert_eq!(
        written["state"]["oscillators"][2]["wavetable"]["table"],
        "reagent/basic-shapes"
    );
}

#[test]
fn malformed_presets_are_refused_not_crashed_on() {
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    let wavetable = json!({ "wavetable": { "table": "reagent/basic-shapes" } });
    let refused: Vec<(&str, Value)> = vec![
        ("not JSON", json!("this isn't a preset")),
        (
            "another format",
            json!({ "format": "other", "schema": 1, "params": [] }),
        ),
        (
            "a later schema",
            json!({ "format": "reagent-preset", "schema": 2, "params": [] }),
        ),
        (
            "an unknown parameter",
            json!({ "format": "reagent-preset", "schema": 1,
                    "params": [{ "id": 999_999_999u32, "value": 1.0 }] }),
        ),
        (
            "a choice by number",
            json!({ "format": "reagent-preset", "schema": 1,
                    "params": [{ "id": id(OSC, 0, osc::DETUNE_LAW).0, "value": 1.0 }] }),
        ),
        (
            "an unknown choice",
            json!({ "format": "reagent-preset", "schema": 1,
                    "params": [{ "id": id(OSC, 0, osc::DETUNE_LAW).0, "value": "Sideways" }] }),
        ),
        (
            "a later state layout",
            json!({ "format": "reagent-preset", "schema": 1, "params": [],
                    "state": { "matrix": { "version": 9 } } }),
        ),
        (
            "a route with no source",
            json!({ "format": "reagent-preset", "schema": 1, "params": [],
                    "state": { "matrix": { "version": 1, "slots": [
                        { "slot": 0, "source": "nowhere", "source_curve": { "bend": 0.5 },
                          "amount": 0.5, "polarity": "unipolar", "destination": null,
                          "aux": null, "aux_invert": false,
                          "aux_curve": { "bend": 0.5 }, "output": 1.0, "bypass": false }
                    ] } } }),
        ),
        (
            "two oscillator slots",
            json!({ "format": "reagent-preset", "schema": 1, "params": [],
                    "state": { "oscillators": [wavetable.clone(), wavetable.clone()] } }),
        ),
        (
            "a wavetable state that isn't one",
            json!({ "format": "reagent-preset", "schema": 1, "params": [],
                    "state": { "oscillators": [
                        { "wavetable": 3 }, wavetable.clone(), wavetable.clone() ] } }),
        ),
        (
            "a table this build doesn't have",
            json!({ "format": "reagent-preset", "schema": 1, "params": [],
                    "state": { "oscillators": [
                        { "wavetable": { "table": "somewhere/else" } },
                        wavetable.clone(), wavetable.clone() ] } }),
        ),
    ];
    for (what, value) in refused {
        let file = Scratch::file("broken.reagent");
        file.write(&value.to_string());
        let result = instance
            .plugin_mut()
            .run_command(preset::LOAD, &json!({ "path": file.path() }));
        assert!(result.is_err(), "{what} loaded: {value}");
    }
    // A file that isn't there, and one too large to be a preset.
    let missing = Scratch::file("missing.reagent");
    assert!(
        instance
            .plugin_mut()
            .run_command(preset::LOAD, &json!({ "path": missing.path() }))
            .is_err()
    );
    let huge = Scratch::file("huge.reagent");
    huge.write(&format!(
        "{{\"format\": \"reagent-preset\", \"schema\": 1, \"params\": [], \"state\": \"{}\"}}",
        "x".repeat(lspk_presets::MAX_BYTES)
    ));
    assert!(
        instance
            .plugin_mut()
            .run_command(preset::LOAD, &json!({ "path": huge.path() }))
            .is_err()
    );
}

#[test]
fn compare_swaps_two_patches_and_undo_takes_them_back() {
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    let level = id(OSC, 0, channel::LEVEL);
    instance.host().edit(level, 10.0).expect("an edit");
    assert_eq!(
        run(&mut instance, preset::STORE_A, json!(null))["side"],
        "a"
    );
    instance.host().edit(level, 90.0).expect("an edit");
    assert_eq!(
        run(&mut instance, preset::STORE_B, json!(null))["side"],
        "b"
    );
    // The patch now is the B side's, so compare brings the A side back.
    let first = run(&mut instance, preset::COMPARE, json!(null));
    assert_eq!(first["showing"], "a");
    assert_eq!(first["applied"], true);
    assert_eq!(instance.values().get(level), Some(10.0));
    // And again: the A side is stored as it is, and the B side comes back.
    let second = run(&mut instance, preset::COMPARE, json!(null));
    assert_eq!(second["showing"], "b");
    assert_eq!(second["applied"], true);
    assert_eq!(instance.values().get(level), Some(90.0));
    // Undo and redo cross the same two patches.
    assert_eq!(
        run(&mut instance, preset::UNDO, json!(null))["restored"],
        true
    );
    assert_eq!(instance.values().get(level), Some(10.0));
    assert_eq!(
        run(&mut instance, preset::REDO, json!(null))["restored"],
        true
    );
    assert_eq!(instance.values().get(level), Some(90.0));
    // With nothing kept on the other side, compare changes nothing.
    let mut empty = Instance::<Reagent>::new().expect("an instance");
    let result = run(&mut empty, preset::COMPARE, json!(null));
    assert_eq!(result["showing"], "a");
    assert_eq!(result["applied"], false);
    assert_eq!(
        run(&mut empty, preset::UNDO, json!(null))["restored"],
        false
    );
    assert_eq!(
        run(&mut empty, preset::REDO, json!(null))["restored"],
        false
    );
}

/// A tool's result, which must succeed.
fn call(client: &mut Client<Http>, tool: &str, args: Value) -> Value {
    client
        .call(tool, args)
        .expect("a reply")
        .ok()
        .unwrap_or_else(|e| panic!("{tool} failed: {e}"))
}

#[test]
fn an_agent_drives_compare_and_undo_through_its_own_server() {
    let mut host = ClapTestHost::from_entry(&reagent::clap_entry, ID).expect("the plug-in");
    host.activate(RATE, BLOCK as u32).expect("activation");
    // Off until the person switches it on, as loading a project saved that
    // way does here.
    let state = host.save_state().expect("a state");
    let on = lspk_plugin_agent::with_server(&state, true).expect("a kit state");
    host.load_state(&on).expect("the state loads");
    let entry: Entry = Discovery::user()
        .expect("a per-user folder")
        .entries()
        .expect("the discovery file")
        .into_iter()
        .find(|e| e.pid == std::process::id() && e.id == ID)
        .expect("an entry");

    let addr = SocketAddr::from(([127, 0, 0, 1], entry.port));
    let token = entry.token.clone();
    serving(&mut host, move || {
        let mut client = Client::new(Http::new(addr, token));
        client
            .initialize("reagent-preset-test")
            .expect("initialised");
        check_tools(&client.tools().expect("tools")).expect("well-described tools");
        // Every preset command is the agent's, with its permission class:
        // the file commands reach outside the project, the rest edit it.
        let listed = call(&mut client, "list_commands", json!({ "area": "preset" }));
        let commands: Vec<(String, String)> = listed["commands"]
            .as_array()
            .expect("commands")
            .iter()
            .map(|c| {
                (
                    c["id"].as_str().unwrap_or_default().to_owned(),
                    c["permission"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        for (id, permission) in [
            ("preset.save", "external"),
            ("preset.load", "external"),
            ("preset.store_a", "edit"),
            ("preset.store_b", "edit"),
            ("preset.compare", "edit"),
            ("preset.undo", "edit"),
            ("preset.redo", "edit"),
        ] {
            assert!(
                commands.contains(&(id.to_owned(), permission.to_owned())),
                "{id} ({permission}): {commands:?}"
            );
        }
        // A/B driven as an agent drives it: keep the patch as A, change the
        // level, keep it as B, and compare the two.
        let level = |client: &mut Client<Http>| {
            let got = call(
                client,
                "run",
                json!({ "commands": [
                    { "command": "params.get",
                      "args": { "params": ["Osc A Level"] } } ] }),
            );
            got["results"][0]["result"]["params"][0]["value"]
                .as_f64()
                .expect("a value")
        };
        call(
            &mut client,
            "run",
            json!({ "commands": [
                { "command": "params.set", "args": { "values": [
                    { "param": "Osc A Level", "value": 10.0 } ] } },
                { "command": "preset.store_a" }
            ] }),
        );
        call(
            &mut client,
            "run",
            json!({ "commands": [
                { "command": "params.set", "args": { "values": [
                    { "param": "Osc A Level", "value": 90.0 } ] } },
                { "command": "preset.store_b" }
            ] }),
        );
        assert_eq!(level(&mut client), 90.0);
        let compared = call(
            &mut client,
            "run",
            json!({ "commands": [ { "command": "preset.compare" } ] }),
        );
        assert_eq!(compared["results"][0]["result"]["showing"], "a");
        assert_eq!(compared["results"][0]["result"]["applied"], true);
        assert_eq!(level(&mut client), 10.0);
        let undone = call(
            &mut client,
            "run",
            json!({ "commands": [ { "command": "preset.undo" } ] }),
        );
        assert_eq!(undone["results"][0]["result"]["restored"], true);
        assert_eq!(level(&mut client), 90.0);
    });
    // The host saw the parameter changes as a person's.
    let block = host.process(BLOCK, None, &[]);
    let level = id(OSC, 0, channel::LEVEL);
    assert!(
        block
            .events
            .iter()
            .any(|e| matches!(e, PluginEvent::GestureBegin { id, .. } if *id == level.0)),
        "a gesture on the level: {:?}",
        block.events
    );
    assert!(host.take_requests().dirty);
}
