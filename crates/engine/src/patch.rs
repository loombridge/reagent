//! The patch compiler: a patch compiled into a flat program of only the
//! modules in use, and the mailbox that hands one to the audio thread.
//!
//! The program says what runs: an oscillator that is off isn't in it, a
//! filter no source is routed to isn't in it (engine-core R59), the filters
//! keep the order they process in (R65), and a voice waits for all four
//! envelopes only when something asks it to (R10). [`Patch::compile`] makes
//! one from the channels' and filters' states, and allocates nothing.
//!
//! The plug-in's main thread compiles a patch when the patch changes — at
//! activation and when a command changes it — and sends it over a
//! [`mailbox`]; the engine takes it at a block's start and hands the program
//! it replaces back to the sender, which drops it there. A host that
//! automates a switch or a routing control changes what is in use on the
//! audio thread itself, so the engine refreshes the running program in place
//! from the states it has just derived, through the same [`Patch::compile`].
//!
//! # State across a swap
//!
//! The program holds what runs, not what the modules are: the voices, the
//! oscillators' phases, the filters' tails, the envelopes and the LFOs are
//! the engine's and never move with a program. Swapping one is inaudible — a
//! render with a recompile forced between two blocks is identical, bit for
//! bit, to one without.

use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, Ordering};

use lspk_dsp::filter::{VoiceParams, pitch_of};
use lspk_params::ParamId;

use crate::params::{self, FILTER, FILTERS, NOISE, OSC, OSCILLATORS, SUB, channel, filter};
use crate::routing::{ChannelOutput, FilterOutput, balance_shares, filter_order};

/// The mixer channels that run: oscillators A to C, the sub and the noise.
pub const CHANNELS: usize = OSCILLATORS + 2;

/// A channel's parameters as last turned into what the mixer needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelState {
    pub(crate) on: bool,
    pub(crate) level: f32,
    pub(crate) pan: f32,
    pub(crate) output: ChannelOutput,
    pub(crate) balance: f32,
    pub(crate) send: [f32; 2],
    pub(crate) amp_env: bool,
}

impl ChannelState {
    /// Channel `s`'s state as `get` reports its parameters: an oscillator
    /// (0 to 2), the sub (3) or the noise (4).
    #[must_use]
    pub fn of(s: usize, get: &impl Fn(ParamId) -> f64) -> Self {
        let (module, instance) = match s {
            0..=2 => (OSC, s as u8),
            3 => (SUB, 0),
            _ => (NOISE, 0),
        };
        let p = |param: u16| params::id(module, instance, param);
        Self {
            on: get(p(channel::ON)) >= 0.5,
            level: get(p(channel::LEVEL)) as f32,
            pan: get(p(channel::PAN)) as f32,
            output: ChannelOutput::from_index(get(p(channel::OUTPUT))),
            balance: get(p(channel::BALANCE)) as f32,
            send: [
                get(p(channel::SEND_1)) as f32,
                get(p(channel::SEND_2)) as f32,
            ],
            amp_env: get(p(channel::AMP_ENV)) >= 0.5,
        }
    }
}

/// A filter's parameters as last turned into what the voice path needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterState {
    pub(crate) on: bool,
    /// Cutoff as a pitch, A4 = 69.
    pub(crate) cutoff: f32,
    pub(crate) voice: VoiceParams,
    pub(crate) clean: bool,
    pub(crate) key_track: bool,
    pub(crate) output: FilterOutput,
    pub(crate) send: [f32; 2],
    pub(crate) amp_env: bool,
}

impl FilterState {
    /// Filter `f`'s state as `get` reports its parameters.
    #[must_use]
    pub fn of(f: usize, get: &impl Fn(ParamId) -> f64) -> Self {
        let p = |param: u16| params::id(FILTER, f as u8, param);
        Self {
            on: get(p(filter::ON)) >= 0.5,
            cutoff: pitch_of(get(p(filter::CUTOFF)) as f32, 440.0),
            voice: VoiceParams {
                resonance: get(p(filter::RESONANCE)) as f32 / 100.0,
                drive: get(p(filter::DRIVE)) as f32 / 100.0,
                saturation: get(p(filter::CHARACTER)) as f32 / 100.0,
                character: get(p(filter::CHARACTER)) as f32 / 100.0,
                stereo_offset: (get(p(filter::STEREO)) as f32 - 50.0) / 50.0 * 12.0,
                mix: get(p(filter::MIX)) as f32 / 100.0,
                level: get(p(filter::LEVEL)) as f32,
                morph_x: get(p(filter::MORPH_X)) as f32,
                morph_y: get(p(filter::MORPH_Y)) as f32,
            },
            clean: get(p(filter::CLEAN_DRIVE)) >= 0.5,
            key_track: get(p(filter::KEY_TRACK)) >= 0.5,
            output: FilterOutput::from_index(get(p(filter::OUTPUT))),
            send: [get(p(filter::SEND_1)) as f32, get(p(filter::SEND_2)) as f32],
            amp_env: get(p(filter::AMP_ENV)) >= 0.5,
        }
    }
}

/// A patch compiled into a flat program: the modules that run, in the order
/// they run, and nothing else.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Patch {
    channels: [u8; CHANNELS],
    n_channels: u8,
    filters: [u8; FILTERS],
    n_filters: u8,
    all_envs: bool,
}

impl Patch {
    /// Compiles the channels and filters as last derived: the channels that
    /// are on, the filters at least one source reaches (engine-core R59), the
    /// filters in the order they process (R65), and whether a voice lasts
    /// until every envelope ends (R10). Allocates nothing, so the audio
    /// thread can refresh a program with it too.
    #[must_use]
    pub fn compile(channels: &[ChannelState; CHANNELS], filters: &[FilterState; FILTERS]) -> Self {
        let mut patch = Self::default();
        for (s, state) in channels.iter().enumerate() {
            if state.on {
                patch.channels[usize::from(patch.n_channels)] = s as u8;
                patch.n_channels += 1;
            }
        }
        for f in filter_order([filters[0].output, filters[1].output]) {
            if fed(channels, filters, f) {
                patch.filters[usize::from(patch.n_filters)] = f as u8;
                patch.n_filters += 1;
            }
        }
        patch.all_envs = channels.iter().any(|c| {
            c.on && !c.amp_env && matches!(c.output, ChannelOutput::Main | ChannelOutput::Dry)
        }) || filters.iter().any(|f| f.on && !f.amp_env);
        patch
    }

    /// The channels that run, in order.
    #[must_use]
    pub fn channels(&self) -> &[u8] {
        &self.channels[..usize::from(self.n_channels)]
    }

    /// The filters that run, in the order they process.
    #[must_use]
    pub fn filters(&self) -> &[u8] {
        &self.filters[..usize::from(self.n_filters)]
    }

    /// Whether a voice lasts until every envelope ends (engine-core R10).
    #[must_use]
    pub fn all_envs(&self) -> bool {
        self.all_envs
    }

    /// How many modules it runs: the channels and the filters.
    #[must_use]
    pub fn modules(&self) -> usize {
        self.channels().len() + self.filters().len()
    }
}

/// Whether filter `f` takes any signal: a channel routed to the filters with
/// a share for it, or, in series, the other filter feeding it (engine-core
/// R59, R65).
fn fed(channels: &[ChannelState; CHANNELS], filters: &[FilterState; FILTERS], f: usize) -> bool {
    let other = 1 - f;
    fed_direct(channels, f)
        || (filters[other].output == FilterOutput::Other && fed_direct(channels, other))
}

/// Whether a channel routes into filter `f`, with a share for it.
fn fed_direct(channels: &[ChannelState; CHANNELS], f: usize) -> bool {
    channels.iter().any(|c| {
        c.on && c.output == ChannelOutput::Filter && {
            let (s1, s2) = balance_shares(c.balance);
            if f == 0 { s1 > 0.0 } else { s2 > 0.0 }
        }
    })
}

/// The programs waiting for the engine, and the ones it has retired.
struct Shared {
    incoming: AtomicPtr<Patch>,
    retired: AtomicPtr<Patch>,
}

impl Shared {
    /// Takes the box in `slot`, if any.
    fn take(slot: &AtomicPtr<Patch>) -> Option<Box<Patch>> {
        let p = slot.swap(ptr::null_mut(), Ordering::AcqRel);
        // SAFETY: a non-null pointer in a slot came from `Box::into_raw` and
        // only `take` or `take_replaced` clears it, so this is the box's only
        // owner.
        (!p.is_null()).then(|| unsafe { Box::from_raw(p) })
    }

    /// Puts `p` in `slot` and takes what was there.
    fn take_replaced(slot: &AtomicPtr<Patch>, p: *mut Patch) -> Option<Box<Patch>> {
        let old = slot.swap(p, Ordering::AcqRel);
        // SAFETY: as in `take`.
        (!old.is_null()).then(|| unsafe { Box::from_raw(old) })
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        drop(Self::take(&self.incoming));
        drop(Self::take(&self.retired));
    }
}

/// The sending end: compiled patches go in, retired ones are dropped here.
pub struct PatchSender {
    shared: Arc<Shared>,
}

/// The receiving end, which the engine holds.
pub struct PatchReceiver {
    shared: Arc<Shared>,
}

/// A mailbox for compiled patches: a sender for the thread that compiles,
/// and a receiver for the engine. Allocates; make it off the audio thread.
#[must_use]
pub fn mailbox() -> (PatchSender, PatchReceiver) {
    let shared = Arc::new(Shared {
        incoming: AtomicPtr::new(ptr::null_mut()),
        retired: AtomicPtr::new(ptr::null_mut()),
    });
    (
        PatchSender {
            shared: Arc::clone(&shared),
        },
        PatchReceiver { shared },
    )
}

impl PatchSender {
    /// Sends `patch`; the engine takes it at its next block. A patch sent
    /// earlier and not yet taken is dropped here.
    pub fn send(&mut self, patch: Patch) {
        self.collect();
        let p = Box::into_raw(Box::new(patch));
        drop(Shared::take_replaced(&self.shared.incoming, p));
    }

    /// Drops the program the engine last retired, if any. Returns whether
    /// there was one.
    pub fn collect(&mut self) -> bool {
        Shared::take(&self.shared.retired).is_some()
    }

    /// Whether a sent patch is still waiting for the engine.
    #[must_use]
    pub fn pending(&self) -> bool {
        !self.shared.incoming.load(Ordering::Acquire).is_null()
    }
}

impl PatchReceiver {
    /// The waiting patch, copied out, if there is one and the retired slot
    /// is free for the box it arrived in — which goes to the sender to free.
    /// Wait-free, and frees nothing.
    pub(crate) fn take(&mut self) -> Option<Patch> {
        if !self.shared.retired.load(Ordering::Acquire).is_null() {
            return None;
        }
        let ptr = self.shared.incoming.swap(ptr::null_mut(), Ordering::AcqRel);
        if ptr.is_null() {
            return None;
        }
        // SAFETY: the swap took the only pointer to the box `send` made.
        // `Patch` is `Copy` and holds no heap of its own, so copying it out
        // leaves the box only to be freed by the sender, never here.
        let patch = unsafe { *ptr };
        let previous = self.shared.retired.swap(ptr, Ordering::AcqRel);
        // The sender keeps the retired slot empty before it sends, so nothing
        // is replaced here. Were something, it is leaked rather than freed:
        // the audio thread never frees.
        debug_assert!(previous.is_null(), "the retired slot was empty");
        let _ = previous;
        Some(patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(on: bool, output: ChannelOutput) -> ChannelState {
        ChannelState {
            on,
            level: 75.0,
            pan: 0.0,
            output,
            balance: -100.0,
            send: [0.0; 2],
            amp_env: true,
        }
    }

    fn filter(on: bool, output: FilterOutput) -> FilterState {
        FilterState {
            on,
            cutoff: 60.0,
            voice: VoiceParams::default(),
            clean: false,
            key_track: false,
            output,
            send: [0.0; 2],
            amp_env: true,
        }
    }

    // Spec: engine-core R8 (a source routed to None keeps running as a warp
    // or modulation source, but the program still holds its channel; what a
    // channel routes to is the mixer's business).
    #[test]
    fn the_program_holds_the_channels_that_are_on() {
        let channels = [
            channel(true, ChannelOutput::Filter),
            channel(false, ChannelOutput::Main),
            channel(false, ChannelOutput::Main),
            channel(false, ChannelOutput::Main),
            channel(true, ChannelOutput::None),
        ];
        let filters = [filter(false, FilterOutput::Main); FILTERS];
        let patch = Patch::compile(&channels, &filters);
        assert_eq!(patch.channels(), [0, 4]);
        assert_eq!(
            patch.filters(),
            [0],
            "A routes to the filters at balance −100"
        );
        assert_eq!(patch.modules(), 3);
        assert!(!patch.all_envs());
    }

    // Spec: engine-core R9 (a filter that is off passes its input on), R59
    // (a filter that is on processes only when a source reaches it).
    #[test]
    fn the_program_holds_the_filters_a_source_reaches() {
        let mut channels = [channel(false, ChannelOutput::Main); CHANNELS];
        channels[0] = channel(true, ChannelOutput::Filter);
        let filters = [
            filter(true, FilterOutput::Main),
            filter(true, FilterOutput::Main),
        ];
        let patch = Patch::compile(&channels, &filters);
        assert_eq!(
            patch.filters(),
            [0],
            "balance −100 sends everything to Filter 1"
        );

        // Balance +100 sends it to Filter 2 alone.
        channels[0].balance = 100.0;
        let patch = Patch::compile(&channels, &filters);
        assert_eq!(patch.filters(), [1]);

        // Routed to Main, no filter takes it.
        channels[0].output = ChannelOutput::Main;
        let patch = Patch::compile(&channels, &filters);
        assert!(patch.filters().is_empty());

        // An oscillator that is off routes nothing wherever it points.
        channels[0] = channel(false, ChannelOutput::Filter);
        let patch = Patch::compile(&channels, &filters);
        assert!(patch.filters().is_empty());
    }

    // Spec: engine-core R59, R65 (a filter is in use when a source reaches
    // it, and in series the feeding filter runs first).
    #[test]
    fn the_filters_a_source_reaches_keep_their_order() {
        let mut channels = [channel(false, ChannelOutput::Main); CHANNELS];
        channels[0] = channel(true, ChannelOutput::Filter);
        let mut filters = [
            filter(true, FilterOutput::Main),
            filter(true, FilterOutput::Main),
        ];
        // Balance −100 sends the channel to Filter 1 alone, +100 to Filter 2
        // alone, and the middle to both.
        assert_eq!(Patch::compile(&channels, &filters).filters(), [0]);
        channels[0].balance = 100.0;
        assert_eq!(Patch::compile(&channels, &filters).filters(), [1]);
        channels[0].balance = 0.0;
        assert_eq!(Patch::compile(&channels, &filters).filters(), [0, 1]);

        // In series the feeding filter runs first, and the other is in use
        // even when the channel reaches only the feeding one.
        channels[0].balance = -100.0;
        filters[0].output = FilterOutput::Other;
        assert_eq!(Patch::compile(&channels, &filters).filters(), [0, 1]);

        // With Filter 2 feeding Filter 1 and the channel reaching only
        // Filter 2, Filter 2 runs first and Filter 1 follows it.
        channels[0].balance = 100.0;
        filters[0].output = FilterOutput::Main;
        filters[1].output = FilterOutput::Other;
        assert_eq!(Patch::compile(&channels, &filters).filters(), [1, 0]);
    }

    // Spec: engine-core R10 (with a used channel's amp envelope off, the voice
    // lasts until the longest release of any envelope has finished).
    #[test]
    fn all_envs_follows_the_used_amp_switches() {
        let mut channels = [channel(false, ChannelOutput::Main); CHANNELS];
        let filters = [filter(false, FilterOutput::Main); FILTERS];
        channels[0] = channel(true, ChannelOutput::Main);
        assert!(!Patch::compile(&channels, &filters).all_envs());
        channels[0].amp_env = false;
        assert!(Patch::compile(&channels, &filters).all_envs());
        // A channel that isn't used, or that isn't heard, doesn't ask for it.
        channels[0] = channel(true, ChannelOutput::None);
        assert!(!Patch::compile(&channels, &filters).all_envs());
        channels[0] = channel(false, ChannelOutput::Main);
        assert!(!Patch::compile(&channels, &filters).all_envs());
        let mut filters = [filter(true, FilterOutput::Main); FILTERS];
        filters[0].amp_env = false;
        assert!(Patch::compile(&channels, &filters).all_envs());
    }

    #[test]
    fn the_mailbox_hands_a_patch_over_and_retires_its_box() {
        let (mut sender, mut receiver) = mailbox();
        assert!(!sender.pending());
        assert_eq!(receiver.take(), None);

        let patch = Patch {
            channels: [2, 0, 0, 0, 0],
            n_channels: 1,
            ..Patch::default()
        };
        sender.send(patch);
        assert!(sender.pending());
        assert_eq!(receiver.take(), Some(patch));
        assert!(!sender.pending());

        // While the retired box waits for the sender, the next take is
        // refused: the audio thread never frees.
        assert!(receiver.take().is_none(), "the retired slot is occupied");
        assert!(sender.collect(), "the sender frees the retired box");
        assert!(!sender.collect());
        assert_eq!(receiver.take(), None);

        // A patch sent again replaces one not yet taken, dropped here.
        sender.send(patch);
        sender.send(patch);
        assert_eq!(receiver.take(), Some(patch));
    }
}
