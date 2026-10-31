//! The learnt starting point for `match_sound` (R9.4): a small neural
//! network that reads a recording's perception features and proposes the
//! patch that made it.
//!
//! The network is trained only on Reagent's own renders, the datasets
//! `reagent-cli dataset` makes (R9.1), by `reagent-cli train-match`; it
//! runs here with no network and no other file, from the weights compiled in
//! (`match_model.bin`). It maps the features of one note to a point of the
//! matching [`Space`](crate::matching::Space): [`input`] lays the features
//! out as numbers near 0 to 1, a hidden layer of ReLU units and a linear
//! layer give each dimension of the space, and the output is clamped into
//! its range. A choice (the filter type, the unison count) comes out as the
//! share of its range the network expects, which decodes to the choice in
//! that share: a start, not an answer. The search refines it.
//!
//! shortcut: the dataset plays one key (48), so the network reads pitch as
//! an absolute and is best on references near that key, and the space is
//! the one R9.3 has; retrain it (`reagent-cli train-match`) when either
//! changes. The test `the_shipped_model_fits_the_space` fails until it is.

use std::sync::OnceLock;

use lspk_test::features::{BANDS, FRAMES, Features, Modulation};

use crate::matching::Rng;

/// The numbers [`input`] makes.
pub const INPUTS: usize = BANDS * FRAMES + FRAMES + 12;

/// The file's first four bytes.
const MAGIC: &[u8; 4] = b"RMM1";
/// The shipped weights.
static SHIPPED: &[u8] = include_bytes!("match_model.bin");
/// The lowest pitch the network reads: the dataset's key, in hertz.
const PITCH_REF_HZ: f64 = 130.81;

/// The features of one note, as the network reads them: the spectrum and
/// the envelope scaled to 0 to 1 (loudest 1), then duration, level, pitch,
/// the two modulations (present, rate, depth) and the stereo image, each
/// scaled to about 0 to 1.
#[must_use]
pub fn input(features: &Features) -> Vec<f32> {
    let modulation = |m: Option<Modulation>, full_depth: f64| {
        let (rate, depth) = m.map_or((0.0, 0.0), |m| {
            (
                (m.rate_hz / 2.0).log2() / 3.4,
                (m.depth / full_depth).min(1.0),
            )
        });
        [f64::from(m.is_some()), rate, depth]
    };
    let (pitch, has_pitch) = features.pitch_hz.map_or((0.0, 0.0), |hz| {
        ((hz / PITCH_REF_HZ).log2().clamp(-2.0, 4.0) / 4.0, 1.0)
    });
    let mut numbers: Vec<f64> = features
        .spectrum
        .iter()
        .map(|db| db / 80.0 + 1.0)
        .chain(features.envelope.iter().map(|db| db / 60.0 + 1.0))
        .collect();
    numbers.extend([
        features.duration.max(0.01).log2() / 3.0,
        (features.level_db + 30.0) / 30.0,
        has_pitch,
        pitch,
    ]);
    numbers.extend(modulation(features.amplitude_modulation, 6.0));
    numbers.extend(modulation(features.brightness_modulation, 1.0));
    numbers.extend([features.stereo.width, features.stereo.correlation]);
    numbers.into_iter().map(|n| n as f32).collect()
}

/// A network from the features to a point of the matching space.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    inputs: usize,
    hidden: usize,
    outputs: usize,
    /// The hidden layer's weights (a row of `inputs` for each unit) and
    /// biases, then the output layer's weights (a row of `hidden` for each
    /// output) and biases.
    weights: Vec<f32>,
}

impl Model {
    /// Where each block starts in `weights`: hidden weights, hidden biases,
    /// output weights, output biases; and the total.
    fn offsets(inputs: usize, hidden: usize, outputs: usize) -> [usize; 5] {
        let w1 = 0;
        let b1 = w1 + hidden * inputs;
        let w2 = b1 + hidden;
        let b2 = w2 + outputs * hidden;
        [w1, b1, w2, b2, b2 + outputs]
    }

    /// An untrained network, its weights drawn from `rng` (He's scaling).
    fn new(inputs: usize, hidden: usize, outputs: usize, rng: &mut Rng) -> Self {
        let [_, b1, w2, b2, total] = Self::offsets(inputs, hidden, outputs);
        let weights = (0..total)
            .map(|i| match i {
                _ if i < b1 => rng.normal() * (2.0 / inputs as f64).sqrt(),
                _ if i < w2 || i >= b2 => 0.0,
                _ => rng.normal() * (2.0 / hidden as f64).sqrt(),
            } as f32)
            .collect();
        Self {
            inputs,
            hidden,
            outputs,
            weights,
        }
    }

    /// The hidden units for `x`, and the outputs, unclamped.
    fn forward(&self, x: &[f32], hidden: &mut [f32], out: &mut [f32]) {
        let [w1, b1, w2, b2, _] = Self::offsets(self.inputs, self.hidden, self.outputs);
        let dot = |row: &[f32], x: &[f32]| row.iter().zip(x).map(|(w, x)| w * x).sum::<f32>();
        for (j, unit) in hidden.iter_mut().enumerate() {
            let row = &self.weights[w1 + j * self.inputs..][..self.inputs];
            *unit = (self.weights[b1 + j] + dot(row, x)).max(0.0);
        }
        for (k, output) in out.iter_mut().enumerate() {
            let row = &self.weights[w2 + k * self.hidden..][..self.hidden];
            *output = self.weights[b2 + k] + dot(row, hidden);
        }
    }

    /// The point of the space `input` (from [`input`]) points to, each
    /// coordinate in 0 to 1.
    ///
    /// # Panics
    ///
    /// If `input` isn't the size the network reads ([`INPUTS`], for the
    /// shipped one).
    #[must_use]
    pub fn predict(&self, input: &[f32]) -> Vec<f64> {
        assert_eq!(input.len(), self.inputs, "the size the network reads");
        let (mut hidden, mut out) = (vec![0.0; self.hidden], vec![0.0; self.outputs]);
        self.forward(input, &mut hidden, &mut out);
        out.into_iter()
            .map(|x| f64::from(x).clamp(0.0, 1.0 - f64::EPSILON))
            .collect()
    }

    /// The network compiled into this build.
    ///
    /// # Panics
    ///
    /// If `match_model.bin` isn't a network: a test keeps it one.
    #[must_use]
    pub fn shipped() -> &'static Self {
        static MODEL: OnceLock<Model> = OnceLock::new();
        MODEL.get_or_init(|| Self::from_bytes(SHIPPED).expect("the shipped network parses"))
    }

    /// The file `reagent-cli train-match` writes: `RMM1`, the input, hidden
    /// and output sizes as little-endian 32-bit numbers, then the weights as
    /// little-endian 32-bit floats.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        for size in [self.inputs, self.hidden, self.outputs] {
            bytes.extend((size as u32).to_le_bytes());
        }
        bytes.extend(self.weights.iter().flat_map(|w| w.to_le_bytes()));
        bytes
    }

    /// A network from [`Self::to_bytes`]'s file.
    ///
    /// # Errors
    ///
    /// The bytes aren't such a file.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let body = bytes
            .strip_prefix(MAGIC)
            .filter(|body| body.len() >= 12)
            .ok_or("not a match model file")?;
        let size =
            |i: usize| u32::from_le_bytes(body[i * 4..][..4].try_into().expect("4 bytes")) as usize;
        let (inputs, hidden, outputs) = (size(0), size(1), size(2));
        let total = Self::offsets(inputs, hidden, outputs)[4];
        let weights = &body[12..];
        if weights.len() != total * 4 {
            return Err("the match model file's sizes don't fit".to_owned());
        }
        Ok(Self {
            inputs,
            hidden,
            outputs,
            weights: weights
                .as_chunks::<4>()
                .0
                .iter()
                .map(|w| f32::from_le_bytes(*w))
                .collect(),
        })
    }

    /// A network of `hidden` units fitted to `samples` (an [`input`] and the
    /// point of the space it should give), by Adam over minibatches,
    /// `epochs` passes, from `seed`; and its mean squared error over the
    /// last pass. The same samples and seed give the same network.
    ///
    /// # Panics
    ///
    /// If there are no samples, or they differ in size.
    #[must_use]
    pub fn train(
        samples: &[(Vec<f32>, Vec<f64>)],
        hidden: usize,
        epochs: usize,
        seed: u64,
    ) -> (Self, f64) {
        const BATCH: usize = 64;
        const RATE: f32 = 2e-3;
        const BETAS: (f32, f32) = (0.9, 0.999);
        let (inputs, outputs) = (samples[0].0.len(), samples[0].1.len());
        let mut rng = Rng(seed);
        let mut model = Self::new(inputs, hidden, outputs, &mut rng);
        let [w1, b1, w2, b2, total] = Self::offsets(inputs, hidden, outputs);
        let (mut grad, mut m, mut v) = (
            vec![0.0f32; total],
            vec![0.0f32; total],
            vec![0.0f32; total],
        );
        let (mut h, mut out, mut dh) = (vec![0.0; hidden], vec![0.0; outputs], vec![0.0; hidden]);
        let mut order: Vec<usize> = (0..samples.len()).collect();
        let (mut step, mut loss) = (0i32, 0.0);
        for epoch in 0..epochs {
            for i in (1..order.len()).rev() {
                order.swap(i, rng.next() as usize % (i + 1));
            }
            // The rate falls to a tenth over the passes.
            let rate = RATE * 0.1f32.powf(epoch as f32 / epochs as f32);
            loss = 0.0;
            for batch in order.chunks(BATCH) {
                grad.fill(0.0);
                for &s in batch {
                    let (x, target) = (&samples[s].0, &samples[s].1);
                    model.forward(x, &mut h, &mut out);
                    dh.fill(0.0);
                    for k in 0..outputs {
                        let error = out[k] - target[k] as f32;
                        loss += f64::from(error * error);
                        grad[b2 + k] += error;
                        for j in 0..hidden {
                            grad[w2 + k * hidden + j] += error * h[j];
                            dh[j] += error * model.weights[w2 + k * hidden + j];
                        }
                    }
                    for j in (0..hidden).filter(|&j| h[j] > 0.0) {
                        grad[b1 + j] += dh[j];
                        for (g, x) in grad[w1 + j * inputs..][..inputs].iter_mut().zip(x) {
                            *g += dh[j] * x;
                        }
                    }
                }
                step += 1;
                let correction = (1.0 - BETAS.0.powi(step), 1.0 - BETAS.1.powi(step));
                for i in 0..total {
                    let g = grad[i] / batch.len() as f32;
                    m[i] = BETAS.0 * m[i] + (1.0 - BETAS.0) * g;
                    v[i] = BETAS.1 * v[i] + (1.0 - BETAS.1) * g * g;
                    model.weights[i] -=
                        rate * (m[i] / correction.0) / ((v[i] / correction.1).sqrt() + 1e-8);
                }
            }
        }
        (model, loss / (samples.len() * outputs) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matching::Space;

    #[test]
    fn the_shipped_model_fits_the_space() {
        let model = Model::shipped();
        assert_eq!(model.inputs, INPUTS);
        assert_eq!(model.outputs, Space::new().len());
    }

    #[test]
    fn a_network_learns_a_mapping_and_survives_its_file() {
        let mut rng = Rng(7);
        let samples: Vec<(Vec<f32>, Vec<f64>)> = (0..400)
            .map(|_| {
                let x: Vec<f32> = (0..3).map(|_| rng.unit() as f32).collect();
                let target = vec![f64::from(x[0]), f64::from(x[1] * x[2])];
                (x, target)
            })
            .collect();
        let (model, loss) = Model::train(&samples, 16, 200, 1);
        assert!(loss < 0.01, "loss {loss}");
        assert_eq!(Model::train(&samples, 16, 200, 1).0, model, "repeats");
        let again = Model::from_bytes(&model.to_bytes()).expect("its own file");
        assert_eq!(again, model);
        assert!(Model::from_bytes(b"RMM1").is_err());
    }

    #[test]
    fn encode_inverts_decode() {
        let space = Space::new();
        let mut rng = Rng(3);
        for _ in 0..20 {
            let point: Vec<f64> = (0..space.len()).map(|_| rng.unit()).collect();
            let patch = space.decode(&point);
            let routes: Vec<_> = patch
                .routes
                .iter()
                .map(|(source, target, amount, polarity)| {
                    let lspk_mod::Target::Param(destination) = target else {
                        unreachable!("the space routes to parameters")
                    };
                    (*source, *destination, *amount, *polarity)
                })
                .collect();
            let again = space
                .encode(&patch.params, &routes)
                .expect("a patch the space made");
            let (a, b) = (patch, space.decode(&again));
            // Equal to within what the logarithm and the power give back.
            assert_eq!(a.params.len(), b.params.len());
            for ((ida, x), (idb, y)) in a.params.iter().zip(&b.params) {
                assert_eq!(ida, idb);
                assert!((x - y).abs() <= 1e-9 * x.abs().max(1.0), "{ida:?}: {x} {y}");
            }
            assert_eq!(a.routes, b.routes);
        }
    }

    #[test]
    fn encode_refuses_what_the_space_cannot_make() {
        use reagent_engine::params::{SUB, channel, id};
        let space = Space::new();
        assert!(
            space
                .encode(&[(id(SUB, 0, channel::ON), 1.0)], &[])
                .is_none()
        );
        assert!(space.encode(&[], &[]).is_some(), "the Init patch");
    }
}
