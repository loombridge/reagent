//! Reagent's default wavetable, made by formula: four frames, a saw, a
//! square, a triangle and a sine, so a new patch plays a saw and Frame
//! sweeps through the classic shapes. Factory tables arrive with
//! `reagent-content` (R6).

use lspk_wavetable::{Cache, FRAME_SIZE, SharedTable, Wavetable};

/// The default table's name in the process-wide cache.
pub const DEFAULT_TABLE: &str = "reagent/basic-shapes";

/// How many frames the default table has.
pub const DEFAULT_FRAMES: usize = 4;

/// The default table's frames: saw, square, triangle, sine.
///
/// # Panics
///
/// Never: four frames is a valid table.
#[must_use]
pub fn basic_shapes() -> Wavetable {
    let size = FRAME_SIZE as f32;
    Wavetable::from_fn(DEFAULT_FRAMES, |frame, n| {
        let x = n as f32 / size;
        match frame {
            // Falling from +1 to −1 across the cycle.
            0 => 1.0 - 2.0 * x,
            1 => {
                if x < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            2 => 1.0 - 4.0 * (x - 0.5).abs(),
            _ => (std::f32::consts::TAU * x).sin(),
        }
    })
    .expect("four frames is a valid table")
}

/// The default table, built once per process and shared by every instance.
/// Builds its mipmaps the first time, so call it off the audio thread.
///
/// # Panics
///
/// Never: the table is made by formula.
#[must_use]
pub fn default_table() -> SharedTable {
    Cache::global()
        .load(DEFAULT_TABLE, || Ok(basic_shapes()))
        .expect("the default table is made by formula")
}
