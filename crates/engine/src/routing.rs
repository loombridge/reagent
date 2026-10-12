//! The mixer's routing: where each channel, filter and bus sends its
//! signal, and the rule that keeps routes from looping (engine-core
//! section 8).
//!
//! Observed in the reference (session 2, the Mix page): routing Bus 2 to
//! Bus 1 while Bus 1 goes to Bus 2 resets Bus 1 to Main, and the same for
//! Filter 2 to Filter 1. So the newer route wins and the earlier one goes
//! to Main ([`resolve_loop`]).

/// Where a mixer channel (an oscillator, the sub or the noise) goes
/// (engine-core R8), by the index its Output parameter stores.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ChannelOutput {
    /// Through the filters, split by Filter balance.
    Filter,
    /// To Main.
    #[default]
    Main,
    /// To the dry output, which skips every effect.
    Dry,
    /// Nowhere; it still runs as a modulation source.
    None,
}

impl ChannelOutput {
    /// The output stored as `index`.
    #[must_use]
    pub fn from_index(index: f64) -> Self {
        match index.round() as i64 {
            0 => Self::Filter,
            2 => Self::Dry,
            3 => Self::None,
            _ => Self::Main,
        }
    }
}

/// Where a filter's output goes (engine-core R65).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FilterOutput {
    /// Into the other filter: the two in series.
    Other,
    /// To Main.
    #[default]
    Main,
    /// To the dry output.
    Dry,
    /// Nowhere.
    None,
}

impl FilterOutput {
    /// The output stored as `index`.
    #[must_use]
    pub fn from_index(index: f64) -> Self {
        match index.round() as i64 {
            0 => Self::Other,
            2 => Self::Dry,
            3 => Self::None,
            _ => Self::Main,
        }
    }
}

/// Where a bus goes (engine-core R66).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BusOutput {
    /// To Main.
    #[default]
    Main,
    /// To the dry output.
    Dry,
    /// Into the other bus.
    Other,
}

impl BusOutput {
    /// The output stored as `index`.
    #[must_use]
    pub fn from_index(index: f64) -> Self {
        match index.round() as i64 {
            1 => Self::Dry,
            2 => Self::Other,
            _ => Self::Main,
        }
    }
}

/// The index each output's "the other one" choice is stored as: a filter's
/// Other filter (0) and a bus's Other bus (2).
pub const FILTER_OTHER: f64 = 0.0;
/// A bus's Other bus.
pub const BUS_OTHER: f64 = 2.0;
/// The index Main is stored as, for a filter.
pub const FILTER_MAIN: f64 = 1.0;
/// The index Main is stored as, for a bus.
pub const BUS_MAIN: f64 = 0.0;

/// A pair of routes that can point at each other: the two filters, or the
/// two buses. `points_at_other[i]` says whether member `i` routes into the
/// other member.
///
/// When member `changed` has just been routed into the other while the
/// other already routes back, the other is reset to Main: returns the
/// index of the member to reset, or `None` if there's no loop
/// (engine-core R65, R68; observed, session 2).
#[must_use]
pub fn resolve_loop(points_at_other: [bool; 2], changed: usize) -> Option<usize> {
    let other = 1 - changed.min(1);
    (points_at_other[changed.min(1)] && points_at_other[other]).then_some(other)
}

/// The order two filters process in: a filter that feeds the other goes
/// first, so series routing reads a finished signal.
#[must_use]
pub fn filter_order(outputs: [FilterOutput; 2]) -> [usize; 2] {
    if outputs[1] == FilterOutput::Other && outputs[0] != FilterOutput::Other {
        [1, 0]
    } else {
        [0, 1]
    }
}

/// The shares of a channel's signal Filter 1 and Filter 2 take at Filter
/// balance `balance` (−100 to +100): a linear crossfade, so the two always
/// sum to the whole signal. The reference's law is to be measured (R7).
#[must_use]
pub fn balance_shares(balance: f32) -> (f32, f32) {
    let b = (balance / 100.0).clamp(-1.0, 1.0);
    ((1.0 - b) * 0.5, (1.0 + b) * 0.5)
}

/// The left and right gains of pan `pan` (−50 to +50): a balance law, unity
/// in both channels at the centre and full in the near channel at either
/// end. The reference's law is to be measured.
#[must_use]
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let p = (pan / 50.0).clamp(-1.0, 1.0);
    ((1.0 - p).min(1.0), (1.0 + p).min(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Spec: engine-core R65, R68 (observed: the earlier route goes to Main).
    #[test]
    fn a_route_back_resets_the_earlier_one_to_main() {
        assert_eq!(resolve_loop([true, true], 1), Some(0));
        assert_eq!(resolve_loop([true, true], 0), Some(1));
        assert_eq!(resolve_loop([true, false], 0), None);
        assert_eq!(resolve_loop([false, true], 1), None);
        assert_eq!(resolve_loop([false, false], 0), None);
    }

    // Spec: engine-core R65 (series puts the feeding filter first).
    #[test]
    fn a_feeding_filter_processes_first() {
        use FilterOutput::*;
        assert_eq!(filter_order([Main, Main]), [0, 1]);
        assert_eq!(filter_order([Other, Main]), [0, 1]);
        assert_eq!(filter_order([Main, Other]), [1, 0]);
        assert_eq!(filter_order([Dry, Other]), [1, 0]);
    }

    // Spec: engine-core R8 (the stored indices).
    #[test]
    fn outputs_read_their_stored_indices() {
        assert_eq!(ChannelOutput::from_index(0.0), ChannelOutput::Filter);
        assert_eq!(ChannelOutput::from_index(1.0), ChannelOutput::Main);
        assert_eq!(ChannelOutput::from_index(2.0), ChannelOutput::Dry);
        assert_eq!(ChannelOutput::from_index(3.0), ChannelOutput::None);
        assert_eq!(FilterOutput::from_index(FILTER_OTHER), FilterOutput::Other);
        assert_eq!(FilterOutput::from_index(FILTER_MAIN), FilterOutput::Main);
        assert_eq!(BusOutput::from_index(BUS_OTHER), BusOutput::Other);
        assert_eq!(BusOutput::from_index(BUS_MAIN), BusOutput::Main);
        assert_eq!(BusOutput::from_index(1.0), BusOutput::Dry);
    }

    #[test]
    fn balance_and_pan_keep_the_centre_whole() {
        assert_eq!(balance_shares(-100.0), (1.0, 0.0));
        assert_eq!(balance_shares(100.0), (0.0, 1.0));
        assert_eq!(balance_shares(0.0), (0.5, 0.5));
        assert_eq!(pan_gains(0.0), (1.0, 1.0));
        assert_eq!(pan_gains(-50.0), (1.0, 0.0));
        assert_eq!(pan_gains(50.0), (0.0, 1.0));
        assert_eq!(pan_gains(25.0), (0.5, 1.0));
    }
}
