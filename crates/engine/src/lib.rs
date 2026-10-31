//! Reagent's sound engine: its voice architecture, assembled from the
//! plug-in kit's crates.
//!
//! | Module | What it holds |
//! |---|---|
//! | [`params`] | Every parameter of every module, with IDs that never change and real names |
//! | [`routing`] | Where each channel, filter and bus goes, and the rule that keeps routes from looping |
//! | [`modulation`] | Every modulation source and destination, and the matrix as a patch saves it |
//! | [`patch`] | The patch compiler: the modules in use as a flat program, and its mailbox |
//! | [`stepping`] | Preset stepping by a controller or a note |
//! | [`table`] | The default wavetable |
//! | [`tuning`] | Tuning files and MTS-ESP: how far each key sounds from equal temperament |
//! | [`engine`] | The engine on the audio thread |
//! | [`describe`] | A patch in prose, and the guide for agents |
//!
//! The signal path, per voice: oscillators A, B and C (wavetable, from
//! `lspk-wavetable`), the sub and the noise (the kit's sub and noise
//! oscillators, R1.8: six fixed shapes, and a sample player over Reagent's
//! own colour blocks), each with its level, pan and output (the filters,
//! split by Filter balance; Main; Dry out; or None) and two bus sends; two
//! filters (`lspk-dsp`) each routed to the other filter, Main, Dry out or
//! None, with sends; Env 1 and Amp where the signal leaves the voice. Then,
//! for the whole instrument, the two buses into Main, Dry out or each
//! other, and Main and Dry out to the output. Four envelopes, ten LFOs and
//! a 64-slot modulation matrix (`lspk-mod`) move it; voicing and glide come
//! from `lspk-voice`; the voice path runs at 1, 2 or 4 times the host's
//! rate (Quality).
//!
//! Rules cited as "engine-core Rn" are the engine specification's.

pub mod describe;
pub mod engine;
pub mod modulation;
pub mod params;
pub mod patch;
pub mod routing;
pub mod stepping;
pub mod table;
pub mod tuning;

pub use engine::Engine;
pub use modulation::Registry;
