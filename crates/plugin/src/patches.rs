//! Hand-built patches the tests and benchmarks play: the Init patch, a
//! detuned saw lead, a filtered bass and a pad moved by LFOs. Each is a
//! set of parameter values and modulation routes, saved as the plug-in
//! kit's state so a test or a host can load it.

use lspk_mod::{Edit, Matrix, Polarity, Target};
use lspk_params::ParamId;
use reagent_engine::Registry;
use reagent_engine::modulation;
use reagent_engine::params::{self, FILTER, LFO, OSC, channel, env, filter, global, id, lfo, osc};

/// A patch: parameter values (the rest at their defaults) and routes.
#[derive(Clone, Debug, Default)]
pub struct Patch {
    /// Its name.
    pub name: &'static str,
    /// Each parameter it sets, with its value in real units.
    pub params: Vec<(ParamId, f64)>,
    /// Its modulation routes: source key, destination, amount, polarity.
    pub routes: Vec<(&'static str, Target, f32, Polarity)>,
}

impl Patch {
    /// The same patch with the voice limit set to `voices`.
    #[must_use]
    pub fn with_voices(mut self, voices: usize) -> Self {
        self.params.retain(|(id, _)| *id != global::VOICES);
        self.params.push((global::VOICES, voices as f64));
        self
    }

    /// Its modulation matrix.
    ///
    /// # Panics
    ///
    /// If a route names a source or destination Reagent doesn't have.
    #[must_use]
    pub fn matrix(&self, registry: &Registry) -> Matrix {
        let mut matrix = Matrix::new();
        for (source, destination, amount, polarity) in &self.routes {
            matrix
                .apply(
                    &Edit::AddRoute {
                        source: (*source).to_owned(),
                        destination: *destination,
                        amount: *amount,
                        polarity: *polarity,
                    },
                    &registry.sources,
                    &registry.destinations,
                )
                .expect("a route Reagent can make");
        }
        matrix
    }

    /// The patch as the plug-in kit's state: every value it sets by ID, then
    /// Reagent's own state, the matrix.
    #[must_use]
    pub fn state(&self, registry: &Registry) -> Vec<u8> {
        let own = modulation::save(&self.matrix(registry), &registry.sources);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"LSPK");
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&(self.params.len() as u32).to_le_bytes());
        for (id, value) in &self.params {
            bytes.extend_from_slice(&id.0.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&own);
        bytes
    }
}

fn osc_param(s: u8, param: u16) -> ParamId {
    id(OSC, s, param)
}

fn filter_param(f: u8, param: u16) -> ParamId {
    id(FILTER, f, param)
}

/// The Init patch: oscillator A's saw through Filter 1, which is off.
#[must_use]
pub fn init() -> Patch {
    Patch {
        name: "init",
        ..Patch::default()
    }
}

/// An empty patch: no oscillator, the sub or the noise on. The compiler
/// leaves every channel out of the program, so a note runs the voices and
/// the envelopes and nothing else — the benchmark's floor case.
#[must_use]
pub fn empty() -> Patch {
    Patch {
        name: "empty",
        params: vec![(osc_param(0, channel::ON), 0.0)],
        routes: Vec::new(),
    }
}

/// A lead: oscillator A's saw as seven unison copies with the Dense detune
/// law, over oscillator B an octave down as three, through a bright
/// low-pass filter.
#[must_use]
pub fn supersaw() -> Patch {
    Patch {
        name: "supersaw",
        params: vec![
            (osc_param(0, osc::UNISON), 7.0),
            (osc_param(0, osc::DETUNE), 0.3),
            (osc_param(0, osc::DETUNE_LAW), 1.0),
            (osc_param(0, channel::LEVEL), 70.0),
            (osc_param(1, channel::ON), 1.0),
            (osc_param(1, channel::OUTPUT), 0.0),
            (osc_param(1, osc::OCTAVE), -1.0),
            (osc_param(1, osc::UNISON), 3.0),
            (osc_param(1, osc::DETUNE), 0.15),
            (osc_param(1, channel::LEVEL), 50.0),
            (filter_param(0, filter::ON), 1.0),
            // SVF LP 12.
            (filter_param(0, filter::TYPE), 5.0),
            (filter_param(0, filter::CUTOFF), 5000.0),
            (filter_param(0, filter::RESONANCE), 15.0),
            (params::env_id(0, env::ATTACK), 5.0),
            (params::env_id(0, env::RELEASE), 300.0),
        ],
        routes: Vec::new(),
    }
}

/// A bass: a saw and a square an octave down through a driven 24 dB
/// ladder, Env 2 sweeping the cutoff down at each note.
#[must_use]
pub fn bass() -> Patch {
    let cutoff = Target::Param(filter_param(0, filter::CUTOFF));
    Patch {
        name: "bass",
        params: vec![
            (osc_param(0, osc::OCTAVE), -1.0),
            (osc_param(1, channel::ON), 1.0),
            (osc_param(1, channel::OUTPUT), 0.0),
            (osc_param(1, osc::OCTAVE), -1.0),
            (osc_param(1, osc::FINE), 5.0),
            // The square: the second of four frames.
            (osc_param(1, osc::FRAME), 100.0 / 3.0),
            (osc_param(1, channel::LEVEL), 60.0),
            (filter_param(0, filter::ON), 1.0),
            // Ladder LP 24.
            (filter_param(0, filter::TYPE), 3.0),
            (filter_param(0, filter::CUTOFF), 180.0),
            (filter_param(0, filter::RESONANCE), 35.0),
            (filter_param(0, filter::DRIVE), 30.0),
            (params::env_id(0, env::DECAY), 600.0),
            (params::env_id(0, env::SUSTAIN), 0.5),
            (params::env_id(0, env::RELEASE), 80.0),
            (params::env_id(1, env::DECAY), 250.0),
            (params::env_id(1, env::SUSTAIN), 0.0),
        ],
        routes: vec![("env2", cutoff, 0.45, Polarity::Unipolar)],
    }
}

/// A pad: a saw and a triangle with smooth frames through a soft low-pass,
/// a slow attack and long release, LFO 1 swinging the cutoff and LFO 2 the
/// saw's frame.
#[must_use]
pub fn pad() -> Patch {
    let cutoff = Target::Param(filter_param(0, filter::CUTOFF));
    let frame = Target::Param(osc_param(0, osc::FRAME));
    Patch {
        name: "pad",
        params: vec![
            (osc_param(0, osc::SMOOTH), 1.0),
            (osc_param(0, osc::FRAME), 30.0),
            (osc_param(0, channel::LEVEL), 65.0),
            (osc_param(0, channel::PAN), -15.0),
            (osc_param(1, channel::ON), 1.0),
            (osc_param(1, channel::OUTPUT), 0.0),
            (osc_param(1, osc::FRAME), 200.0 / 3.0),
            (osc_param(1, osc::FINE), 7.0),
            (osc_param(1, channel::LEVEL), 65.0),
            (osc_param(1, channel::PAN), 15.0),
            (filter_param(0, filter::ON), 1.0),
            // SVF LP 24.
            (filter_param(0, filter::TYPE), 7.0),
            (filter_param(0, filter::CUTOFF), 1200.0),
            (filter_param(0, filter::RESONANCE), 20.0),
            (params::env_id(0, env::ATTACK), 400.0),
            (params::env_id(0, env::RELEASE), 1500.0),
            // LFO 1 at 0.6 Hz, LFO 2 at a half note.
            (id(LFO, 0, lfo::SYNC), 0.0),
            (id(LFO, 0, lfo::RATE), 0.6),
            (id(LFO, 1, lfo::DIVISION), 6.0),
        ],
        routes: vec![
            ("lfo1", cutoff, 0.2, Polarity::Bipolar),
            ("lfo2", frame, 0.3, Polarity::Bipolar),
        ],
    }
}

/// The patches the benchmarks and golden renders play.
#[must_use]
pub fn all() -> [Patch; 4] {
    [init(), supersaw(), bass(), pad()]
}
