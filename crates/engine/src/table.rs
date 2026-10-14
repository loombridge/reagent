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

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape checks allow `f32` rounding against an `f64` ideal.
    const SHAPE_TOLERANCE: f32 = 1e-4;

    /// The saw's last sample stops one of 2048 steps short of −1, so the
    /// peak check is looser than the shape check.
    const PEAK_TOLERANCE: f32 = 1e-3;

    /// The ideal sample of the frame's classic waveform at `n`.
    fn ideal(frame: usize, n: usize) -> f32 {
        let x = n as f64 / FRAME_SIZE as f64;
        let value = match frame {
            0 => 1.0 - 2.0 * x,
            1 => {
                if x < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            2 => 1.0 - 4.0 * (x - 0.5).abs(),
            _ => (std::f64::consts::TAU * x).sin(),
        };
        value as f32
    }

    #[test]
    fn the_default_table_has_four_frames() {
        assert_eq!(DEFAULT_FRAMES, 4);
        assert_eq!(basic_shapes().frame_count(), DEFAULT_FRAMES);
    }

    #[test]
    fn every_frame_peaks_at_one() {
        for (frame, samples) in basic_shapes().frames().iter().enumerate() {
            let max = samples.iter().copied().fold(f32::MIN, f32::max);
            let min = samples.iter().copied().fold(f32::MAX, f32::min);
            assert!(
                (max - 1.0).abs() <= PEAK_TOLERANCE,
                "frame {frame} reaches {max}, not +1"
            );
            assert!(
                (min + 1.0).abs() <= PEAK_TOLERANCE,
                "frame {frame} reaches {min}, not -1"
            );
        }
    }

    #[test]
    fn the_frames_are_a_saw_a_square_a_triangle_and_a_sine() {
        for (frame, samples) in basic_shapes().frames().iter().enumerate() {
            for (n, &sample) in samples.iter().enumerate() {
                let ideal = ideal(frame, n);
                assert!(
                    (sample - ideal).abs() <= SHAPE_TOLERANCE,
                    "frame {frame}, sample {n}: {sample} is not {ideal}"
                );
            }
        }
    }
}
