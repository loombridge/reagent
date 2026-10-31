//! Preset stepping by a controller or a note: two assigned numbers step the
//! preset forward and back.
//!
//! The audio thread watches the events it is given. In [`Mode::Controller`] a
//! controller message at or above 64 steps once, and the controller must
//! fall below 64 before it can step again, as a button's press and release
//! do. In [`Mode::Note`] a note-on of an assigned key steps. The messages
//! and notes that step are taken out of the event stream: they don't sound
//! or modulate. Steps wait in [`Stepping`] for the main thread to take.

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use lspk_plugin::Event;

/// What assigns the two steps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Nothing steps presets.
    #[default]
    Off,
    /// Two controller numbers.
    Controller,
    /// Two keys.
    Note,
}

/// The assignment and the steps waiting, shared between the audio thread and
/// the main thread.
pub struct Stepping {
    /// Mode, forward and back packed: the mode in bits 16 and up, forward in
    /// bits 8 to 15 and back in bits 0 to 7, 255 for none.
    config: AtomicU32,
    pending: AtomicI32,
}

const NONE: u32 = 255;

impl Stepping {
    /// An assignment of nothing.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            config: AtomicU32::new(NONE << 8 | NONE),
            pending: AtomicI32::new(0),
        })
    }

    /// Assigns `forward` and `back` (controller numbers or keys, 0 to 127;
    /// `None` leaves a step unassigned) under `mode`.
    pub fn set(&self, mode: Mode, forward: Option<u8>, back: Option<u8>) {
        let pack = |n: Option<u8>| n.map_or(NONE, |n| u32::from(n & 127));
        let mode = mode as u32;
        self.config.store(
            mode << 16 | pack(forward) << 8 | pack(back),
            Ordering::Relaxed,
        );
    }

    /// The steps since the last call, forward counted up and back down.
    pub fn take(&self) -> i32 {
        self.pending.swap(0, Ordering::AcqRel)
    }
}

/// The audio thread's side.
pub struct Stepper {
    shared: Arc<Stepping>,
    /// Whether each controller has fallen below 64 since it last stepped.
    armed: [bool; 2],
}

impl Stepper {
    /// A stepper over `shared`.
    #[must_use]
    pub fn new(shared: Arc<Stepping>) -> Self {
        Self {
            shared,
            armed: [true; 2],
        }
    }

    /// Whether `event` steps a preset, and so goes no further. Takes no lock
    /// and allocates nothing.
    pub fn takes(&mut self, event: &Event) -> bool {
        let config = self.shared.config.load(Ordering::Relaxed);
        let mode = match config >> 16 {
            1 => Mode::Controller,
            2 => Mode::Note,
            _ => return false,
        };
        let assigned = [(config >> 8) & 255, config & 255];
        let step = |which: usize| {
            let by = if which == 0 { 1 } else { -1 };
            self.shared.pending.fetch_add(by, Ordering::AcqRel);
        };
        match (*event, mode) {
            (
                Event::Midi {
                    data: [status, number, value],
                    ..
                },
                Mode::Controller,
            ) if status & 0xF0 == 0xB0 => {
                let Some(which) = assigned.iter().position(|&a| a == u32::from(number)) else {
                    return false;
                };
                if value >= 64 {
                    if std::mem::take(&mut self.armed[which]) {
                        step(which);
                    }
                } else {
                    self.armed[which] = true;
                }
                true
            }
            (Event::NoteOn { note, .. }, Mode::Note) => {
                let which = note
                    .key
                    .and_then(|k| assigned.iter().position(|&a| a == u32::from(k)));
                which.inspect(|&w| step(w)).is_some()
            }
            (Event::NoteOff { note, .. } | Event::NoteChoke { note, .. }, Mode::Note) => {
                note.key.is_some_and(|k| assigned.contains(&u32::from(k)))
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lspk_plugin::Note;

    fn cc(number: u8, value: u8) -> Event {
        Event::Midi {
            time: 0,
            port: 0,
            data: [0xB0, number, value],
        }
    }

    fn note_on(key: u8) -> Event {
        Event::NoteOn {
            time: 0,
            note: Note {
                key: Some(key),
                ..Note::default()
            },
            velocity: 1.0,
        }
    }

    // A controller of 64 or more steps; it must fall
    // below 64 before it can step again.
    #[test]
    fn a_controller_steps_once_a_press_and_not_again_until_released() {
        let shared = Stepping::new();
        shared.set(Mode::Controller, Some(21), Some(22));
        let mut stepper = Stepper::new(shared.clone());
        for e in [cc(21, 127), cc(21, 127), cc(21, 90), cc(21, 0), cc(21, 64)] {
            assert!(stepper.takes(&e));
        }
        assert_eq!(shared.take(), 2);
        assert!(stepper.takes(&cc(22, 100)));
        assert_eq!(shared.take(), -1);
        assert_eq!(shared.take(), 0);
        assert!(!stepper.takes(&cc(23, 127)));
    }

    #[test]
    fn a_note_steps_and_does_not_sound() {
        let shared = Stepping::new();
        shared.set(Mode::Note, Some(0), Some(1));
        let mut stepper = Stepper::new(shared.clone());
        assert!(stepper.takes(&note_on(0)));
        assert!(stepper.takes(&note_on(0)));
        assert!(stepper.takes(&note_on(1)));
        assert!(!stepper.takes(&note_on(60)));
        assert_eq!(shared.take(), 1);
        // A controller does nothing in Note mode.
        assert!(!stepper.takes(&cc(0, 127)));
    }

    #[test]
    fn nothing_steps_when_unassigned_or_off() {
        let shared = Stepping::new();
        let mut stepper = Stepper::new(shared.clone());
        assert!(!stepper.takes(&cc(21, 127)));
        shared.set(Mode::Controller, None, None);
        assert!(!stepper.takes(&cc(0, 127)));
        assert_eq!(shared.take(), 0);
    }
}
