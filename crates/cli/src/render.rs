//! `reagent-cli render`: a preset and a phrase, or a Standard MIDI File,
//! played through Reagent's engine and written to a WAV, headless and faster
//! than real time.
//!
//! The render runs the same engine code a host's audio thread runs, one
//! block after another on this thread, so nothing in it depends on timing:
//! the same preset and phrase render to the same samples every time and at
//! any block size. Every seed the engine uses is fixed, so its random phase
//! and noise repeat too.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use lspk_plugin::{Event, Note, Plugin};
use lspk_test::Script;
use lspk_test::render::{Instance, Output, Settings};
use reagent::Reagent;
use reagent::preset;

use crate::Outcome;

/// The largest block a render accepts, in frames.
const MAX_BLOCK: usize = 8192;

/// The highest sample rate a render accepts, in hertz: past it a render's
/// buffer could run to gigabytes.
const MAX_RATE: f64 = 768_000.0;

/// How to render.
#[derive(Debug, Parser)]
pub struct RenderArgs {
    /// The preset file to play: a Reagent preset, as `preset.save` writes it.
    #[arg(long)]
    pub preset: PathBuf,
    /// Where to write the WAV file.
    #[arg(long)]
    pub out: PathBuf,
    /// The phrase to play, when no MIDI file is given.
    #[arg(long, value_enum, default_value_t = Phrase::Chord)]
    pub phrase: Phrase,
    /// A Standard MIDI File to play instead of the phrase.
    #[arg(long)]
    pub midi: Option<PathBuf>,
    /// The sample rate, in hertz.
    #[arg(long, default_value_t = 48_000.0)]
    pub rate: f64,
    /// Frames per block; the output is the same at any block size.
    #[arg(long, default_value_t = 512)]
    pub block: usize,
}

/// A phrase built in code, for a render with no MIDI file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Phrase {
    /// A held chord that releases, then a high note.
    Chord,
    /// A short ascending run of notes.
    Run,
}

impl Phrase {
    /// Its script at `rate`.
    fn script(self, rate: f64) -> Script {
        let frames = |seconds: f64| (seconds * rate).round() as u32 as usize;
        match self {
            Self::Chord => Script::new(frames(2.0))
                .note(0, frames(1.5) as u32, 36, 1.0)
                .note(0, frames(1.5) as u32, 43, 0.9)
                .note(0, frames(1.5) as u32, 48, 0.8)
                .note(frames(1.6) as u32, frames(0.3) as u32, 60, 0.9),
            Self::Run => (0..8u32).fold(Script::new(frames(3.0)), |script, i| {
                script.note(
                    frames(f64::from(i) * 0.25) as u32,
                    frames(0.5) as u32,
                    48 + (i * 3) as u8,
                    0.8,
                )
            }),
        }
    }
}

/// Renders `args` and returns the audio, writing nothing.
///
/// # Errors
///
/// The preset file can't be read or isn't one this build reads, the MIDI
/// file isn't one, or the block size or sample rate isn't one a render takes.
pub fn render(args: &RenderArgs) -> Result<Output, String> {
    if !(1..=MAX_BLOCK).contains(&args.block) {
        return Err(format!(
            "a block holds 1 to {MAX_BLOCK} frames, not {}",
            args.block
        ));
    }
    if !(1.0..=MAX_RATE).contains(&args.rate) {
        return Err(format!(
            "a sample rate is 1 to {MAX_RATE} Hz, not {}",
            args.rate
        ));
    }
    let script = match &args.midi {
        Some(path) => script_from_midi(path, args.rate)?,
        None => args.phrase.script(args.rate),
    };
    let mut instance = Instance::<Reagent>::new()?;
    let path = args.preset.to_string_lossy();
    instance
        .plugin_mut()
        .run_command(preset::LOAD, &serde_json::json!({ "path": path }))
        .map_err(|error| format!("{}: {}", args.preset.display(), error.message()))?;
    instance.activate(Settings::new(args.rate, args.block))?;
    let output = instance.render(&script).output;
    if !output.is_finite() {
        return Err("the render holds a sample that isn't finite".to_owned());
    }
    Ok(output)
}

/// `output` written to `path` as a 32-bit float WAV file.
///
/// # Errors
///
/// The file can't be written, or the audio isn't a WAV file's shape.
pub fn write(output: &Output, path: &Path) -> Result<(), String> {
    let wav = lspk_test::agent::encode_wav(output)?;
    fs::write(path, wav).map_err(|error| format!("{}: {error}", path.display()))
}

/// `render`, then `write`, then a line saying what was written.
pub fn run(args: &RenderArgs) -> Outcome {
    let output = render(args)?;
    write(&output, &args.out)?;
    let peak = peak_dbfs(&output);
    println!(
        "wrote {}: {} frames, {} channels at {} Hz, {peak:.1} dBFS peak",
        args.out.display(),
        output.frames(),
        output.channels.len(),
        output.sample_rate,
    );
    Ok(ExitCode::SUCCESS)
}

/// The largest absolute sample, in dBFS (−∞ for silence).
fn peak_dbfs(output: &Output) -> f64 {
    let peak = output
        .channels
        .iter()
        .flatten()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    lspk_test::measure::db(f64::from(peak))
}

/// `path` read as a Standard MIDI File, as a script at `rate`.
///
/// Every note plays on the first MIDI channel; a note-on with velocity 0 is
/// a note-off, as the file format says. The render lasts a second past the
/// last event, so every note's release is heard.
fn script_from_midi(path: &Path, rate: f64) -> Result<Script, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let smf = midly::Smf::parse(&bytes)
        .map_err(|error| format!("{}: not a MIDI file: {error}", path.display()))?;
    let seconds_at = time_base(&smf);
    let mut events: Vec<Event> = Vec::new();
    for track in &smf.tracks {
        let mut ticks = 0u64;
        for event in track {
            ticks += u64::from(event.delta.as_int());
            let at = (seconds_at(ticks) * rate).round();
            if !at.is_finite() || at < 0.0 || at > f64::from(u32::MAX) {
                return Err(format!("{}: an event lands past a render", path.display()));
            }
            let midly::TrackEventKind::Midi { message, .. } = event.kind else {
                continue;
            };
            let (key, velocity) = match message {
                // A note-on with velocity 0 is a note-off, as the format says.
                midly::MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                    (key, f32::from(vel.as_int()) / 127.0)
                }
                midly::MidiMessage::NoteOn { key, .. }
                | midly::MidiMessage::NoteOff { key, .. } => (key, 0.0),
                _ => continue,
            };
            let note = Note::key(0, key.as_int());
            let time = at as u32;
            events.push(if velocity > 0.0 {
                Event::NoteOn {
                    time,
                    note,
                    velocity,
                }
            } else {
                Event::NoteOff {
                    time,
                    note,
                    velocity: 0.0,
                }
            });
        }
    }
    let last = events.iter().map(|event| event.time()).max().unwrap_or(0);
    let mut script = Script::new(last as usize + rate as usize);
    for event in events {
        script = script.event(event);
    }
    Ok(script)
}

/// A function from a file's ticks to seconds, from its timing and its tempo
/// changes.
fn time_base(smf: &midly::Smf<'_>) -> Box<dyn Fn(u64) -> f64> {
    match smf.header.timing {
        // SMPTE timing: a tick is a fixed fraction of a second.
        midly::Timing::Timecode(fps, subframes) => {
            let per_tick = 1.0 / (f64::from(fps.as_f32()) * f64::from(subframes));
            Box::new(move |ticks| ticks as f64 * per_tick)
        }
        // Metrical timing: a tick is a fraction of a beat, which a tempo
        // change sets in microseconds per quarter note. A later tempo at the
        // same tick wins, as the file's order says.
        midly::Timing::Metrical(per_quarter) => {
            let mut tempos: BTreeMap<u64, u32> = BTreeMap::new();
            tempos.insert(0, 500_000);
            for track in &smf.tracks {
                let mut ticks = 0u64;
                for event in track {
                    ticks += u64::from(event.delta.as_int());
                    if let midly::TrackEventKind::Meta(midly::MetaMessage::Tempo(micros)) =
                        event.kind
                    {
                        tempos.insert(ticks, micros.as_int());
                    }
                }
            }
            let per_quarter = f64::from(per_quarter.as_int());
            let seconds_per_tick = |micros: u32| f64::from(micros) / 1e6 / per_quarter;
            // Each entry: the tick, the seconds at it, and the seconds a tick
            // takes from there.
            let mut marks: Vec<(u64, f64, f64)> = Vec::new();
            let mut seconds = 0.0;
            let mut previous: Option<(u64, u32)> = None;
            for (ticks, micros) in tempos {
                if let Some((at, was)) = previous {
                    seconds += (ticks - at) as f64 * seconds_per_tick(was);
                }
                marks.push((ticks, seconds, seconds_per_tick(micros)));
                previous = Some((ticks, micros));
            }
            Box::new(move |ticks| {
                let at = marks.partition_point(|(tick, ..)| *tick <= ticks).max(1) - 1;
                let (tick, seconds, per_tick) = marks[at];
                seconds + (ticks - tick) as f64 * per_tick
            })
        }
    }
}
