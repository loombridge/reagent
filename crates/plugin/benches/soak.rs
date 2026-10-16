//! The plan's worst-case render, measured in release: ten minutes at
//! 32-frame blocks with notes, modulation and preset changes, through the
//! reference patches, under the allocation trap, then the idle cost once
//! every tail has ended.
//!
//! Timing varies with the machine, so this prints numbers rather than
//! asserting them; the trap is what a failure is. Run it with
//! `cargo bench -p reagent --bench soak`. It is not one of the `perf`
//! benchmarks the pull-request instruction gate counts: ten minutes of
//! audio under Callgrind would not finish, so this is the timing harness
//! for the reference machines the plan's nightly run uses.
//!
//! Spec: engine-core R1.17 (the reference patches meet their budgets).

use std::process::ExitCode;
use std::time::{Duration, Instant};

use lspk_plugin::{Event, Note, ProcessStatus};
use lspk_test::cpu::Timings;
use lspk_test::render::{Instance, Settings};
use reagent::Reagent;
use reagent::params::{OSC, channel, id};
use reagent::patches;
use reagent_engine::Registry;

lspk_rt::install_trap!();

const RATE: f64 = 48_000.0;
/// The plan's worst-case render lasts ten minutes.
const MINUTES: f64 = 10.0;
/// The plan's smallest block: 32 frames.
const BLOCK: usize = 32;
/// The plan's per-voice budgets run from 8 voices up; the render runs the
/// low end, so every block carries the full load.
const VOICES: usize = 8;
/// The frames between chord changes and between recompiles.
const CHORD: usize = 24_000;
const SWITCH: usize = 48_000;

/// Runs the render and returns its exit status.
fn run() -> ExitCode {
    let registry = Registry::new();
    let patches = patches::all();
    let states: Vec<Vec<u8>> = patches
        .iter()
        .map(|patch| patch.clone().with_voices(VOICES).state(&registry))
        .collect();
    let mut instance = Instance::<Reagent>::new().expect("an instance");
    instance.load_state(&states[0]).expect("the patch loads");
    instance
        .activate(Settings::new(RATE, BLOCK))
        .expect("activation");

    let blocks = (MINUTES * 60.0 * RATE / BLOCK as f64) as usize;
    let per_patch = blocks / states.len();
    let started = Instant::now();
    let mut timings = Timings::new(RATE);
    let mut events: Vec<Event> = Vec::new();
    let mut held: Vec<u8> = Vec::new();
    let mut preset_changes = 0usize;
    let mut recompiles = 0usize;
    let mut worst = (Duration::ZERO, 0usize);
    for index in 0..blocks {
        let frame = index * BLOCK;
        // A different reference patch every tenth of the render: a preset
        // change on the main thread, taken up by the audio thread.
        if index.is_multiple_of(per_patch) {
            let next = (index / per_patch) % states.len();
            instance.load_state(&states[next]).expect("the patch loads");
            preset_changes += 1;
        }
        events.clear();
        // A new chord every half second, the old keys released first, and a
        // switch that decides what is in use once a second, so the program
        // is recompiled in place on the audio thread.
        if frame.is_multiple_of(CHORD) {
            for key in held.drain(..) {
                events.push(Event::NoteOff {
                    time: 0,
                    note: Note::key(0, key),
                    velocity: 0.0,
                });
            }
            let step = (frame / CHORD) as u8;
            for i in 0..VOICES as u8 {
                let key = 36 + step % 24 + i * 3;
                held.push(key);
                events.push(Event::NoteOn {
                    time: 0,
                    note: Note::key(0, key),
                    velocity: 0.8,
                });
            }
        }
        if frame.is_multiple_of(SWITCH) {
            let on = if (frame / SWITCH).is_multiple_of(2) {
                1.0
            } else {
                0.0
            };
            events.push(Event::Param {
                time: 0,
                id: id(OSC, 2, channel::ON),
                value: on,
            });
            recompiles += 1;
        }
        let block = instance.process(BLOCK, &[], &events);
        if block.duration > worst.0 {
            worst = (block.duration, index);
        }
        timings.push(BLOCK, block.duration);
    }
    let played = started.elapsed();

    // Release the last chord and let every tail end, then measure the idle
    // cost: whether the plug-in sleeps, and what a block costs if it does
    // not.
    events.clear();
    for key in held.drain(..) {
        events.push(Event::NoteOff {
            time: 0,
            note: Note::key(0, key),
            velocity: 0.0,
        });
    }
    instance.process(BLOCK, &[], &events);
    let mut sleeps = false;
    for _ in 0..((4.0 * RATE) as usize / BLOCK) {
        if instance.process(BLOCK, &[], &[]).status == ProcessStatus::Sleep {
            sleeps = true;
            break;
        }
    }
    let mut idle = Timings::new(RATE);
    if sleeps {
        // A second of blocks a host would not even call, since the plug-in
        // has told it it is asleep.
        for _ in 0..((RATE) as usize / BLOCK) {
            idle.push(BLOCK, instance.process(BLOCK, &[], &[]).duration);
        }
    }

    let (worst_duration, worst_index) = worst;
    let worst_frame = worst_index * BLOCK;
    println!(
        "reference patches, {MINUTES} min at {BLOCK} frames, {VOICES} voices, 48 kHz\n\
         \x20 process: worst block {:.1} % of its deadline, median {:.1} %, p99 {:.1} %, \
         {:.3} % of a core\n\
         \x20 idle:    {} (mean {:.0} ns a block)\n\
         \x20 {} preset changes and {recompiles} recompiles, {blocks} blocks in {:.1} s\n\
         \x20 slowest block at frame {worst_frame} ({:?}), {}",
        timings.worst_deadline_fraction() * 100.0,
        timings.median().as_secs_f64() * RATE / BLOCK as f64 * 100.0,
        timings.percentile(0.99).as_secs_f64() * RATE / BLOCK as f64 * 100.0,
        timings.load() * 100.0,
        if sleeps {
            "asleep: costs a host nothing"
        } else {
            "never sleeps"
        },
        idle.mean().as_secs_f64() * 1e9,
        preset_changes,
        played.as_secs_f64(),
        worst_duration,
        {
            let mut what = Vec::new();
            if worst_index.is_multiple_of(per_patch) {
                what.push("preset change");
            }
            if worst_frame.is_multiple_of(SWITCH) {
                what.push("recompile");
            }
            if worst_frame.is_multiple_of(CHORD) {
                what.push("chord change");
            }
            if what.is_empty() {
                "neither".to_owned()
            } else {
                what.join(" + ")
            }
        },
    );
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    run()
}
