//! Reagent's editor runs commands: undo, redo and A/B compare are the
//! `preset.*` commands, run on the main thread through Reagent's own
//! registry as an agent's are, and a parameter edit from the editor is the
//! kit's `params.set`.

use std::sync::Arc;

use lspk_params::ParamId;
use lspk_plugin::editor::{EditorChanges, EditorHost};
use lspk_plugin::host::{Edit, HostBackend};
use lspk_plugin::{EditorContext, EditorSize, Host, ParamValues, Plugin, Registry};
use lspk_plugin_agent::WithAgent;
use reagent::params::{FILTER, filter, id};
use reagent::{Reagent, editor_actions, preset};
use serde_json::{Value, json};

struct NoHost;

impl EditorHost for NoHost {
    fn request_callback(&self) {}
    fn request_resize(&self, _size: EditorSize) -> bool {
        true
    }
}

impl HostBackend for NoHost {
    fn request_flush(&self) {}
    fn mark_dirty(&self) {}
    fn request_restart(&self) {}
    fn request_process(&self) {}
    fn request_callback(&self) {}
    fn values_changed(&self) {}
}

/// Reagent as the CLAP export runs it, with an editor's link to it, and
/// the main thread's callback made by hand.
struct Rig {
    cx: EditorContext,
    changes: EditorChanges,
    plugin: WithAgent<Reagent>,
    host: Host,
    /// The edits the host link has queued, taken off.
    edits: Box<dyn FnMut() -> Option<Edit>>,
    heard: Vec<Edit>,
}

impl Rig {
    fn new() -> Self {
        let host = Host::new(Box::new(NoHost));
        let plugin = WithAgent::<Reagent>::new(host.clone());
        let values = Arc::new(ParamValues::for_plugin(&plugin).expect("Reagent's parameters"));
        let mut queue = host.connect(values.clone());
        let (cx, changes) = EditorContext::new(values, Box::new(NoHost));
        Self {
            cx,
            changes,
            plugin,
            host,
            edits: Box::new(move || queue.pop().ok()),
            heard: Vec::new(),
        }
    }

    /// The host calls back: the main thread runs what the editor asked.
    fn callback(&mut self) {
        self.changes.apply(&self.host, &mut self.plugin);
        assert_eq!(self.cx.take_command_error(), None);
        self.heard.extend(std::iter::from_fn(&mut self.edits));
    }

    fn run(&mut self, command: &str, label: &str) {
        self.cx.run_command(command, json!({}), label);
        self.callback();
    }

    /// A parameter's real value now.
    fn value(&self, param: ParamId) -> f64 {
        let values = self.cx.values();
        let view = values.view(param).expect("a parameter");
        view.real(values.get(param).expect("a parameter"))
    }
}

#[test]
fn the_editors_buttons_are_registered_preset_commands() {
    let rig = Rig::new();
    let mut registry = Registry::new();
    rig.plugin
        .register_commands(&mut registry)
        .expect("the commands register");
    let actions = editor_actions();
    let commands: Vec<&str> = actions.iter().map(|a| a.command.as_str()).collect();
    assert_eq!(
        commands,
        [
            preset::UNDO,
            preset::REDO,
            preset::STORE_A,
            preset::STORE_B,
            preset::COMPARE
        ]
    );
    for action in &actions {
        let spec = registry.get(&action.command).expect("a registered command");
        assert_eq!(action.label, spec.label);
    }
}

#[test]
fn undo_redo_and_compare_run_from_the_editor() {
    let mut rig = Rig::new();
    let cutoff = id(FILTER, 0, filter::CUTOFF);

    // A parameter edit is a command, and the host hears one gesture.
    rig.cx.edit(cutoff, 200.0).expect("a parameter");
    assert_ne!(rig.value(cutoff), 200.0, "asked, not yet done");
    rig.callback();
    assert!((rig.value(cutoff) - 200.0).abs() < 1.0);
    assert!(matches!(rig.heard.first(), Some(Edit::Begin(p)) if *p == cutoff));
    assert!(matches!(rig.heard.last(), Some(Edit::End(p)) if *p == cutoff));

    // A at 200 Hz, B at 4 kHz, then compare swings between them.
    rig.run(preset::STORE_A, "Store A");
    rig.cx.edit(cutoff, 4000.0).expect("a parameter");
    rig.callback();
    rig.run(preset::STORE_B, "Store B");
    rig.run(preset::COMPARE, "Switch A and B");
    assert!((rig.value(cutoff) - 200.0).abs() < 1.0, "A is showing");
    rig.run(preset::COMPARE, "Switch A and B");
    assert!((rig.value(cutoff) - 4000.0).abs() < 1.0, "B is showing");

    // Undo takes the last switch back, redo makes it again, and each says
    // which change it was: the label the editor ran it under.
    rig.run(preset::UNDO, "Undo");
    assert!((rig.value(cutoff) - 200.0).abs() < 1.0);
    rig.run(preset::REDO, "Redo");
    assert!((rig.value(cutoff) - 4000.0).abs() < 1.0);
    let undone: Value = rig
        .plugin
        .run_command(preset::UNDO, &json!({}))
        .expect("undo runs");
    assert_eq!(
        undone,
        json!({ "restored": true, "label": "Switch A and B" })
    );
}
