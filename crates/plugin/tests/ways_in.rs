//! Each way in to Reagent for an agent, exercised by a scripted client and
//! headless: its own agent server in every format it ships in (CLAP, VST3
//! and Audio Unit), the agent-commands extension a kit-aware host serves
//! through its own agent API, and what a sandboxed host that refuses the
//! server's socket or its discovery file shows the person. `reagent-cli
//! mcp` has its own tests in `reagent-cli`.
//!
//! Each test needs a process of its own, as cargo-nextest gives it: the
//! discovery file and the environment are the process's.

use std::net::SocketAddr;
use std::time::Duration;

use lspk_agent::client::{Client, Http};
use lspk_agent::discovery::{Discovery, Entry};
use lspk_test::agent::check_tools;
use lspk_test::clap::{ClapTestHost, PluginEvent};
use reagent::params::{OSC, channel, id};
use reagent::{ID, NAME};
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

/// This process's entry in the discovery file.
fn own_entry() -> Entry {
    Discovery::user()
        .expect("a per-user folder")
        .entries()
        .expect("the discovery file")
        .into_iter()
        .find(|e| e.pid == std::process::id() && e.id == ID)
        .expect("an entry")
}

/// Runs `calls` on a thread of its own while `idle` gives the plug-in its
/// main thread, as the host's event loop does when the server wakes it.
fn serving<T: Send + 'static>(
    mut idle: impl FnMut(),
    calls: impl FnOnce() -> T + Send + 'static,
) -> T {
    let calls = std::thread::spawn(calls);
    while !calls.is_finished() {
        idle();
    }
    calls
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// What an agent that finds Reagent in the discovery file does first:
/// connects with the entry's token, reads both guides, lists the commands
/// and the patch, and sets the first oscillator's level as one step.
fn meet(entry: &Entry) -> Client<Http> {
    let addr = SocketAddr::from(([127, 0, 0, 1], entry.port));
    let mut client = Client::new(Http::new(addr, entry.token.clone()));
    client.initialize("reagent-test").expect("initialised");
    check_tools(&client.tools().expect("tools")).expect("well-described tools");
    let guide = client
        .read_resource("lspk://plugin-guide")
        .expect("Reagent's guide");
    assert!(guide.contains("Filter balance"), "{guide}");
    assert!(guide.contains("Working with the tools"), "{guide}");
    let found = call(&mut client, "list_commands", json!({ "area": "mod" }));
    assert!(found["commands"].to_string().contains("mod.add_route"));
    let state = call(&mut client, "query", json!({}));
    assert_eq!(state["plugin"]["kind"], "instrument");
    let summary = state["summary"].as_str().expect("a summary");
    assert!(summary.contains("oscillator A at 75 % level"), "{summary}");
    let set = call(
        &mut client,
        "run",
        json!({ "commands": [ { "command": "params.set", "args": { "values": [
            { "param": "Osc A Level", "text": "50 %" } ] } } ] }),
    );
    assert_eq!(set["results"][0]["result"]["params"][0]["text"], "50 %");
    client
}

#[test]
fn the_clap_export_serves_an_agent_found_in_the_discovery_file() {
    let mut host = ClapTestHost::from_entry(&reagent::clap_entry, ID).expect("the plug-in");
    host.set_track_name("Lead");
    host.activate(RATE, FRAMES as u32).expect("activation");
    let state = host.save_state().expect("a state");
    let on = lspk_plugin_agent::with_server(&state, true).expect("a kit state");
    host.load_state(&on).expect("the state loads");

    let entry = own_entry();
    assert_eq!(entry.name, NAME);
    assert_eq!(entry.track.as_deref(), Some("Lead"));
    assert_eq!(entry.url, format!("http://127.0.0.1:{}/mcp", entry.port));
    let (_, ()) = lspk_test::agent::serving(&mut host, move || (meet(&entry), ()));
    // The host saw the change as a person's gesture.
    let block = host.process(FRAMES, None, &[]);
    let level = id(OSC, 0, channel::LEVEL).0;
    assert!(
        block
            .events
            .iter()
            .any(|e| matches!(e, PluginEvent::GestureBegin { id, .. } if *id == level)),
        "{:?}",
        block.events
    );
    assert!(host.take_requests().dirty);
}

#[test]
fn the_vst3_export_serves_an_agent_found_in_the_discovery_file() {
    #[cfg(windows)]
    // SAFETY: the crate's own module entry, as a host calls it.
    unsafe {
        reagent::InitDll();
    }
    // SAFETY: the crate's own VST3 factory, which lives as long as the
    // process.
    let mut host =
        unsafe { lspk_test::vst3::Vst3TestHost::from_factory(reagent::GetPluginFactory()) }
            .expect("an instance");
    host.activate(RATE, FRAMES as u32).expect("activation");
    let state = host.save_state().expect("a state");
    let on = lspk_plugin_agent::with_server(&state, true).expect("a kit state");
    host.load_state(&on).expect("the state loads");

    let entry = own_entry();
    let level = id(OSC, 0, channel::LEVEL).0;
    // The wrapper calls the plug-in's main thread from the host's event
    // loop, so the loop runs while the agent works.
    let _client = serving(|| host.idle(Duration::from_millis(5)), move || meet(&entry));
    host.idle(Duration::from_millis(100));
    let events = host.take_handler_events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, lspk_test::vst3::HandlerEvent::Begin(p) if *p == level)),
        "{events:?}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_audio_unit_export_keeps_the_agent_server_off_until_it_is_switched_on() {
    let mut host =
        lspk_test::auv2::AuTestHost::new(reagent::GetPluginFactoryAUV2, ["aumu", "Rtst", "Test"])
            .expect("an instance");
    host.initialize(FRAMES as u32).expect("initialisation");
    host.midi(0x90, 69, 127);
    assert!(host.render(FRAMES as u32).is_ok());
    let entries = Discovery::user()
        .expect("a per-user folder")
        .entries()
        .unwrap_or_default();
    assert!(
        entries.iter().all(|e| e.pid != std::process::id()),
        "{entries:?}"
    );
}

#[test]
fn a_kit_aware_host_lists_and_runs_the_commands_through_the_extension() {
    let mut host = ClapTestHost::from_entry(&reagent::clap_entry, ID).expect("the plug-in");
    host.activate(RATE, FRAMES as u32).expect("activation");
    let commands = host.commands().expect("the extension's list");
    for command in &commands {
        assert!(
            !command.description.is_empty() && command.permission.is_some(),
            "{} is missing its description or permission class",
            command.id
        );
        assert!(
            command.args.is_some() && command.result.is_some(),
            "{} is missing a schema",
            command.id
        );
    }
    let ids: Vec<&str> = commands.iter().map(|c| &*c.id).collect();
    for wanted in ["params.get", "params.set", "mod.add_route", "preset.load"] {
        assert!(ids.contains(&wanted), "{wanted} in {ids:?}");
    }
    // The ones a host leaves out when it serves agents itself.
    let mut person: Vec<&str> = ids
        .iter()
        .copied()
        .filter(|id| lspk_plugin_agent::commands::person_only(id))
        .collect();
    person.sort_unstable();
    assert_eq!(person, ["agent.server_status", "agent.set_server"]);
    // Every command the guide tells an agent to run exists.
    for token in reagent_engine::describe::GUIDE
        .split('`')
        .skip(1)
        .step_by(2)
    {
        let command = token.contains('.')
            && token.split('.').all(|part| {
                !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase() || c == '_')
            });
        assert!(!command || ids.contains(&token), "the guide names {token}");
    }
    let load = commands.iter().find(|c| c.id == "preset.load").expect("it");
    assert_eq!(load.permission, Some(lspk_commands::Permission::External));

    // A command changes the patch and the host hears it as a person's.
    let level = id(OSC, 0, channel::LEVEL).0;
    let set = host
        .run_command(
            "params.set",
            &json!({ "values": [ { "param": "Osc A Level", "text": "50 %" } ] }),
        )
        .expect("it runs");
    assert_eq!(set["params"][0]["text"], "50 %");
    let block = host.process(FRAMES, None, &[]);
    assert!(
        block
            .events
            .iter()
            .any(|e| matches!(e, PluginEvent::GestureBegin { id, .. } if *id == level)),
        "{:?}",
        block.events
    );
    assert!(host.take_requests().dirty);

    // A failure says why, for the host to pass on.
    let unknown = host
        .run_command("nothing.here", &json!({}))
        .expect_err("an unknown command fails");
    assert!(unknown.contains("nothing.here"), "{unknown}");
    let bad = host
        .run_command(
            "params.set",
            &json!({ "values": [ { "param": "No Such Knob", "value": 1.0 } ] }),
        )
        .expect_err("an unknown parameter fails");
    assert!(bad.contains("No Such Knob"), "{bad}");
}

/// An instance with its server switched on, which has run its main thread
/// once to start it, and its `agent.server_status`.
fn switched_on() -> (ClapTestHost, Value) {
    let mut host = ClapTestHost::from_entry(&reagent::clap_entry, ID).expect("the plug-in");
    host.activate(RATE, FRAMES as u32).expect("activation");
    let state = host.save_state().expect("a state");
    let on = lspk_plugin_agent::with_server(&state, true).expect("a kit state");
    host.load_state(&on).expect("the state loads");
    host.on_main_thread();
    let status = host
        .run_command("agent.server_status", &json!({}))
        .expect("the status");
    (host, status)
}

/// The instance still plays and takes an agent's commands.
fn still_works(host: &mut ClapTestHost) {
    host.run_command(
        "params.set",
        &json!({ "values": [ { "param": "Osc A Level", "text": "50 %" } ] }),
    )
    .expect("a command runs");
    let block = host.process(
        FRAMES,
        None,
        &[lspk_test::clap::HostEvent::NoteOn {
            time: 0,
            key: 69,
            channel: 0,
            note_id: None,
            velocity: 1.0,
        }],
    );
    assert!(block.outputs[0].iter().any(|s| s.abs() > 0.001));
}

#[test]
fn a_host_that_refuses_the_discovery_file_leaves_the_server_running_and_says_so() {
    // A file where the per-user folder would be made.
    let blocker = std::env::temp_dir().join(format!("reagent-ways-in-{}", std::process::id()));
    std::fs::write(&blocker, b"not a folder").expect("a file");
    let variable = if cfg!(windows) {
        "APPDATA"
    } else if cfg!(target_os = "macos") {
        "HOME"
    } else {
        "XDG_RUNTIME_DIR"
    };
    // SAFETY: nextest runs each test in a process of its own, and nothing
    // else has started a thread that reads the environment yet.
    unsafe { std::env::set_var(variable, &blocker) };

    let (mut host, status) = switched_on();
    assert_eq!(status["on"], true);
    assert_eq!(status["running"], true, "{status}");
    let error = status["error"].as_str().expect("an error");
    assert!(
        error.contains("couldn't write the discovery file"),
        "{error}"
    );
    assert!(status.get("discovery").is_none(), "{status}");
    // Without the file the person still has the URL and token to give an
    // agent by hand, and they work.
    let entry = Entry::new(
        ID,
        NAME,
        status["url"].as_str().expect("a url"),
        status["url"]
            .as_str()
            .and_then(|u| u.rsplit(':').next())
            .and_then(|p| p.trim_end_matches("/mcp").parse().ok())
            .expect("a port"),
        status["token"].as_str().expect("a token"),
    );
    lspk_test::agent::serving(&mut host, move || (meet(&entry), ()));
    still_works(&mut host);
}

/// Refuses every socket the process binds from here on, as an App Sandbox
/// without the network-server entitlement does.
#[cfg(target_os = "macos")]
fn refuse_listening_sockets() {
    use std::ffi::{CStr, c_char, c_int};

    // SAFETY: Apple's `sandbox_init`, called with a NUL-terminated profile
    // and a place for its error text.
    unsafe extern "C" {
        fn sandbox_init(profile: *const c_char, flags: u64, error: *mut *mut c_char) -> c_int;
    }
    let profile = c"(version 1) (allow default) (deny network-bind)";
    let mut error = std::ptr::null_mut();
    // SAFETY: as above; the profile outlives the call.
    let result = unsafe { sandbox_init(profile.as_ptr(), 0, &mut error) };
    assert_eq!(result, 0, "{:?}", {
        // SAFETY: on failure `error` points to a C string.
        (!error.is_null()).then(|| unsafe { CStr::from_ptr(error) })
    });
}

#[cfg(target_os = "macos")]
#[test]
fn a_host_that_refuses_the_socket_leaves_the_plugin_working_and_says_so() {
    refuse_listening_sockets();
    let (mut host, status) = switched_on();
    assert_eq!(status["on"], true);
    assert_eq!(status["running"], false, "{status}");
    let error = status["error"].as_str().expect("an error");
    assert!(error.contains("the agent server couldn't start"), "{error}");
    assert!(status.get("url").is_none() && status.get("token").is_none());
    assert!(
        Discovery::user()
            .and_then(|d| d.entries().ok())
            .unwrap_or_default()
            .iter()
            .all(|e| e.pid != std::process::id())
    );
    // The host's own agent, or the person, still reaches it.
    still_works(&mut host);
}
