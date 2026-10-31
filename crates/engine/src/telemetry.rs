//! What the engine shows the interface: the live values the audio thread
//! publishes through the kit's telemetry channel while an editor is open
//! (R2.3).
//!
//! A [`Frame`] carries each modulated destination's value per voice (for
//! the modulation rings), the first oscillator's phase per voice, each LFO's
//! phase and each envelope's stage per voice, the output's peak levels, and
//! a scope and a spectrum of the output. The engine writes only while an
//! editor watches, so a closed window costs one atomic load per block, and
//! it keeps sleeping when silent.

use lspk_dsp::Analyser;
pub use lspk_mod::telemetry::{Frame, Layout, Publisher, Watcher};

use crate::engine::MAX_VOICES;
use crate::modulation::Registry;
use crate::params::{ENVELOPES, LFOS};

/// The output channels metered, left and right.
pub const METERED: usize = 2;
/// The samples the spectrum is taken from, and so its resolution.
const ANALYSIS: usize = 1024;
/// The samples in the scope: the last 512 the host hears, 11 ms at 48 kHz.
pub const SCOPE: usize = 512;

/// The layout of Reagent's frames.
#[must_use]
pub fn layout(registry: &Registry) -> Layout {
    Layout {
        destinations: registry.destinations.len(),
        voices: MAX_VOICES,
        lfos: LFOS,
        envelopes: ENVELOPES,
        channels: METERED,
        scope: SCOPE,
        bins: ANALYSIS / 2 + 1,
    }
}

/// The audio thread's end of the channel and what it needs to fill a frame.
pub(crate) struct State {
    pub(crate) publisher: Publisher<Frame>,
    pub(crate) analyser: Analyser,
    /// The block's channels mixed to one, for the analyser.
    pub(crate) mono: Vec<f32>,
    /// Each channel's peak since the last publication.
    pub(crate) peaks: [f32; METERED],
}

impl State {
    /// Allocates; make it off the audio thread.
    pub(crate) fn new(publisher: Publisher<Frame>, max_frames: usize) -> Self {
        Self {
            publisher,
            analyser: Analyser::new(ANALYSIS),
            mono: vec![0.0; max_frames],
            peaks: [0.0; METERED],
        }
    }
}
