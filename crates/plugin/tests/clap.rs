//! Reagent as a host loads it: from its built `.clap` file, and from its
//! entry point in this binary under the allocation trap.

use lspk_test::clap::{Block, ClapTestHost, HostEvent, PluginEvent, ProcessStatus};
use reagent::params::{FILTER, OSC, channel, filter, global, id};
use reagent::{ID, NAME};

lspk_rt::install_trap!();

const RATE: f64 = 48_000.0;
const FRAMES: usize = 512;

/// The plug-in from the `.clap` file Cargo built, active.
fn built() -> ClapTestHost {
    let clap = lspk_test::clap::bundle_built_library("reagent", NAME, ID);
    let mut host = ClapTestHost::load(&clap, ID).expect("the built plug-in");
    host.activate(RATE, FRAMES as u32).expect("activation");
    host
}

fn note_on(time: u32, key: u16) -> HostEvent {
    HostEvent::NoteOn {
        time,
        key,
        channel: 0,
        note_id: None,
        velocity: 1.0,
    }
}

fn note_off(time: u32, key: u16) -> HostEvent {
    HostEvent::NoteOff {
        time,
        key: Some(key),
        channel: Some(0),
        note_id: None,
    }
}

/// `blocks` blocks, with `events` in the first; every block's left channel
/// joined, and the last block.
fn render(host: &mut ClapTestHost, events: &[HostEvent], blocks: usize) -> (Vec<f32>, Block) {
    let mut left = Vec::new();
    let mut block = host.process(FRAMES, None, events);
    left.extend_from_slice(&block.outputs[0]);
    for _ in 1..blocks {
        block = host.process(FRAMES, None, &[]);
        left.extend_from_slice(&block.outputs[0]);
    }
    (left, block)
}

#[test]
fn the_built_plugin_shows_every_parameter_and_plays_each_key() {
    let mut host = built();
    assert_eq!(host.audio_ports(true), Vec::<u32>::new());
    assert_eq!(host.audio_ports(false), [2]);
    assert_eq!(host.note_ports(true), 1);
    let params = host.params();
    assert_eq!(params.len(), reagent::params().len());
    assert!(params.iter().all(|p| p.automatable));
    let names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
    for name in [
        "Macro 1",
        "Main level",
        "Osc A Level",
        "Osc C Sweep",
        "Osc Sub Pan",
        "Osc Noise Output",
        "Filter 2 Cutoff",
        "Bus 1 Output",
        "Env 1 Attack",
        "Env 4 End level",
        "LFO 10 Rate",
    ] {
        assert!(names.contains(&name), "{name}");
    }
    let main = params
        .iter()
        .find(|p| p.name == "Main level")
        .expect("Main level");
    assert_eq!(host.text(main.id, main.default).as_deref(), Some("-9.0 dB"));

    for key in [45u16, 69, 81] {
        let (left, block) = render(&mut host, &[note_on(0, key)], 48);
        let measured = lspk_test::measure::pitch(&left[4_800..], RATE).expect("a pitch");
        let expected = 440.0 * 2f64.powf((f64::from(key) - 69.0) / 12.0);
        assert!(
            (measured - expected).abs() < expected * 1e-4,
            "key {key}: {measured} Hz, not {expected} Hz"
        );
        assert_eq!(block.status, ProcessStatus::Continue);
        let (_, block) = render(&mut host, &[note_off(0, key)], 20);
        assert_eq!(block.status, ProcessStatus::Sleep);
    }
}

#[test]
fn a_note_ends_after_its_release_and_the_host_hears_it() {
    let mut host = built();
    let (left, _) = render(&mut host, &[note_on(100, 69)], 2);
    assert!(
        left[..=100].iter().all(|s| *s == 0.0),
        "silent until the note's frame"
    );
    assert!(left[200..].iter().any(|s| *s != 0.0));

    let (_, block) = render(&mut host, &[note_off(10, 69)], 1);
    assert_eq!(block.status, ProcessStatus::Continue, "still releasing");
    // Env 1's 15 ms release ends within a few blocks.
    let mut ended = Vec::new();
    let mut last = block;
    for _ in 0..10 {
        last = host.process(FRAMES, None, &[]);
        ended.extend(last.events.iter().copied());
    }
    assert!(
        matches!(ended[..], [PluginEvent::NoteEnd { key: Some(69), .. }]),
        "{ended:?}"
    );
    assert_eq!(last.status, ProcessStatus::Sleep);
    assert!(last.outputs.iter().flatten().all(|s| *s == 0.0));
}

// Spec: engine-core R117, R122 (eight voices; the oldest is stolen).
#[test]
fn a_ninth_note_takes_the_oldest_voice() {
    let mut host = built();
    let notes: Vec<HostEvent> = (0..=8u16).map(|i| note_on(u32::from(i), 60 + i)).collect();
    let block = host.process(FRAMES, None, &notes);
    assert_eq!(
        block.events,
        [PluginEvent::NoteEnd {
            time: 8,
            key: Some(60)
        }]
    );
}

#[test]
fn with_no_notes_it_sleeps_in_silence() {
    let mut host = built();
    let (left, block) = render(&mut host, &[], 1);
    assert_eq!(block.status, ProcessStatus::Sleep);
    assert!(left.iter().all(|s| *s == 0.0));
}

#[test]
fn main_level_scales_it_down_to_silence() {
    let mut host = built();
    let (loud, _) = render(&mut host, &[note_on(0, 69)], 8);
    let peak = |s: &[f32]| s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak(&loud[3 * FRAMES..]) > 0.05);
    let silent = HostEvent::Param {
        time: 0,
        id: global::MAIN_LEVEL.0,
        value: 0.0,
    };
    let (quiet, _) = render(&mut host, &[silent], 4);
    assert!(quiet[3 * FRAMES..].iter().all(|s| *s == 0.0));
}

#[test]
fn processing_stays_off_the_heap() {
    assert!(lspk_rt::trap::is_installed());
    // From the entry point in this binary, so the trap sees the plug-in's
    // allocations as well as the host's.
    let mut host = ClapTestHost::from_entry(&reagent::clap_entry, ID).expect("the plug-in");
    host.activate(RATE, FRAMES as u32).expect("activation");
    let param = |time: u32, id: lspk_params::ParamId, value: f64| HostEvent::Param {
        time,
        id: id.0,
        value,
    };
    for i in 0..64u16 {
        let key = 36 + i % 36;
        let events = [
            note_on(u32::from(i), key),
            param(50, id(OSC, 1, channel::ON), f64::from(i % 2)),
            param(60, id(OSC, 2, channel::OUTPUT), f64::from(i % 4)),
            param(70, id(FILTER, 0, filter::ON), f64::from((i / 2) % 2)),
            param(
                80,
                id(FILTER, 0, filter::CUTOFF),
                200.0 + 50.0 * f64::from(i),
            ),
            param(90, id(FILTER, 1, filter::OUTPUT), f64::from(i % 4)),
            param(95, id(FILTER, 0, filter::OUTPUT), f64::from((i + 1) % 4)),
            param(100, global::QUALITY, f64::from(i % 3)),
            param(110, global::VOICES, f64::from(1 + i % 32)),
            note_off(400, 36 + (i + 12) % 36),
            HostEvent::NoteChoke {
                time: 450,
                key: Some(key.saturating_sub(5)),
            },
        ];
        host.process(FRAMES, None, &events);
    }
}
