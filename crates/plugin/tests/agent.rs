//! Reagent driven by an agent through its own agent server, over the real
//! HTTP transport with the kit's scripted MCP client, while the CLAP test
//! host plays it: the guides, a patch change (Filter 1 on, a lower cutoff,
//! and an envelope routed to it in the modulation matrix) that the host
//! sees as a person's gestures, a note previewed and analysed, and the Init
//! patch restored.
//!
//! One test, so this process's discovery entries are all its own.

use std::net::SocketAddr;

use lspk_agent::client::{Client, Http};
use lspk_agent::discovery::{Discovery, Entry};
use lspk_test::agent::{audio, check_tools, serving};
use lspk_test::clap::{ClapTestHost, PluginEvent};
use reagent::params::{FILTER, filter, id};
use reagent::{ID, INIT_PATCH};
use serde_json::{Value, json};

const RATE: f64 = 48_000.0;
const FRAMES: usize = 256;

/// A tool's result, which must succeed.
fn call(client: &mut Client<Http>, tool: &str, args: Value) -> Value {
    client
        .call(tool, args)
        .expect("a reply")
        .ok()
        .unwrap_or_else(|e| panic!("{tool} failed: {e}"))
}

#[test]
fn an_agent_designs_a_patch_through_its_own_server() {
    let mut host = ClapTestHost::from_entry(&reagent::clap_entry, ID).expect("the plug-in");
    host.activate(RATE, FRAMES as u32).expect("activation");
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

    let cutoff = id(FILTER, 0, filter::CUTOFF);
    let filter_on = id(FILTER, 0, filter::ON);
    let addr = SocketAddr::from(([127, 0, 0, 1], entry.port));
    let token = entry.token.clone();
    let (mut client, ()) = serving(&mut host, move || {
        let mut client = Client::new(Http::new(addr, token));
        client.initialize("reagent-test").expect("initialised");
        check_tools(&client.tools().expect("tools")).expect("well-described tools");
        let guide = client
            .read_resource("lspk://plugin-guide")
            .expect("Reagent's guide");
        assert!(guide.contains("Filter balance"), "{guide}");
        let found = call(&mut client, "list_commands", json!({ "area": "mod" }));
        let ids: Vec<&str> = found["commands"]
            .as_array()
            .expect("commands")
            .iter()
            .filter_map(|c| c["id"].as_str())
            .collect();
        assert!(ids.contains(&"mod.add_route"), "{ids:?}");
        let state = call(&mut client, "query", json!({}));
        assert_eq!(state["plugin"]["kind"], "instrument");
        let summary = state["summary"].as_str().expect("a summary");
        assert!(summary.contains("oscillator A at 75 % level"), "{summary}");

        // The patch change: Filter 1 on at 400 Hz, Env 2 sweeping it.
        let set = call(
            &mut client,
            "run",
            json!({ "commands": [
                { "command": "params.set", "args": { "values": [
                    { "param": "Filter 1 On", "value": 1.0 },
                    { "param": "filter \"1\" / param \"Cutoff\"", "text": "400 Hz" },
                    { "param": "Env 2 Decay", "text": "300 ms" },
                    { "param": "Env 2 Sustain", "value": 0.0 } ] } },
                { "command": "mod.add_route", "args": {
                    "source": "env2", "destination": { "param": cutoff.0 }, "amount": 0.4 } }
            ] }),
        );
        assert_eq!(set["results"][0]["result"]["params"][1]["text"], "400 Hz");
        assert_eq!(set["results"][1]["result"]["slot"], 0);
        let after = call(&mut client, "query", json!({}));
        let summary = after["summary"].as_str().expect("a summary");
        assert!(
            summary.contains("Filter 1 is a Ladder LP 12 at 400 Hz"),
            "{summary}"
        );
        assert!(
            summary.contains("Envelope 2 to Filter 1 Cutoff by +40 %"),
            "{summary}"
        );
        (client, ())
    });

    // The host saw the parameter changes as a person's.
    let block = host.process(FRAMES, None, &[]);
    for param in [filter_on, cutoff] {
        let begun = block
            .events
            .iter()
            .any(|e| matches!(e, PluginEvent::GestureBegin { id, .. } if *id == param.0));
        assert!(begun, "a gesture on {param:?}: {:?}", block.events);
    }
    assert!(host.take_requests().dirty);

    // A note: A4, heard and analysed through a separate copy, matrix
    // included.
    let (back, note) = serving(&mut host, move || {
        let preview = client
            .call(
                "preview",
                json!({ "notes": [ { "key": 69, "length": 0.5, "velocity": 1.0 } ] }),
            )
            .expect("a reply");
        (client, preview)
    });
    client = back;
    let analysis = note.clone().ok().expect("a preview");
    assert_eq!(analysis["pitch"]["note"], "A4", "{analysis}");
    assert!(audio(&note).expect("audio").frames() > 0);

    // The Init patch again: the matrix empties and Filter 1 is off.
    let (_, after) = serving(&mut host, move || {
        call(
            &mut client,
            "run",
            json!({ "commands": [ { "command": INIT_PATCH } ] }),
        );
        let routes = call(
            &mut client,
            "run",
            json!({ "commands": [ { "command": "mod.list" } ] }),
        );
        let state = call(&mut client, "query", json!({}));
        (client, (routes, state))
    });
    let (routes, state) = after;
    assert_eq!(routes["results"][0]["result"], json!([]));
    let summary = state["summary"].as_str().expect("a summary");
    assert!(!summary.contains("Filter 1 is"), "{summary}");
}
