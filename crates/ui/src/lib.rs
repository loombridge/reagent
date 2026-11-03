//! Reagent's interface, built from the plug-in kit's widgets.
//!
//! This is its look: the tokens every page draws with ([`look`]: three
//! candidate visual identities, each dark and light, and the theme files a
//! person adds), its icons ([`icons`]), and how its window opens
//! ([`appearance`]: size presets and the remembered default size).

pub mod appearance;
pub mod icons;
pub mod look;

pub use appearance::{Appearance, SIZE_PRESETS, open_options};
pub use look::Look;
