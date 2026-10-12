//! A patch in prose, for agents that reason better from text than from a
//! list of values, and the guide to Reagent's modules agents read first.

use lspk_mod::commands::SlotView;
use lspk_mod::{Sources, Target};
use lspk_params::{ParamId, ParamSpace};

use crate::params::{
    self, CHANNEL_OUTPUTS, FILTER, FILTER_OUTPUTS, FILTER_TYPES, OSC, OSC_NAMES, QUALITIES,
    channel, filter, global, osc,
};

/// Reagent's guide for agents, served as the resource
/// `lspk://plugin-guide`.
pub const GUIDE: &str = include_str!("../guide.md");

/// The patch `value_of` reads, and its modulation `routes`, in a few
/// sentences: what plays, how it's routed, the filters, and what modulates
/// what.
#[must_use]
pub fn summary(
    space: &ParamSpace,
    sources: &Sources,
    routes: &[SlotView],
    value_of: &dyn Fn(ParamId) -> Option<f64>,
) -> String {
    let get = |id: ParamId| value_of(id).unwrap_or(0.0);
    let on = |id: ParamId| get(id) >= 0.5;
    let name_of = |id: ParamId| {
        space
            .descriptors()
            .into_iter()
            .find(|d| d.id == id)
            .map_or_else(|| format!("parameter {}", id.0), |d| d.name)
    };
    let mut parts = Vec::new();
    let mut playing = Vec::new();
    for (s, name) in OSC_NAMES.iter().enumerate() {
        let p = |param: u16| params::id(OSC, s as u8, param);
        if !on(p(channel::ON)) {
            continue;
        }
        let output = CHANNEL_OUTPUTS[(get(p(channel::OUTPUT)).round() as usize).min(3)];
        let tune = 12.0 * get(p(osc::OCTAVE)) + get(p(osc::SEMITONE)) + get(p(osc::SWEEP));
        let mut text = format!(
            "oscillator {name} at {:.0} % level, frame {:.0} %",
            get(p(channel::LEVEL)),
            get(p(osc::FRAME))
        );
        if tune != 0.0 || get(p(osc::FINE)) != 0.0 {
            text.push_str(&format!(
                ", tuned {tune:+.0} semitones {:+.0} cents",
                get(p(osc::FINE))
            ));
        }
        text.push_str(&format!(", to {}", output.to_lowercase()));
        playing.push(text);
    }
    if playing.is_empty() {
        parts.push("No oscillator is on, so the patch is silent.".to_owned());
    } else {
        parts.push(format!(
            "A wavetable synthesiser playing {}.",
            playing.join("; ")
        ));
    }
    for f in 0..params::FILTERS {
        let p = |param: u16| params::id(FILTER, f as u8, param);
        if !on(p(filter::ON)) {
            continue;
        }
        let kind =
            FILTER_TYPES[(get(p(filter::TYPE)).round() as usize).min(FILTER_TYPES.len() - 1)];
        let output = FILTER_OUTPUTS[(get(p(filter::OUTPUT)).round() as usize).min(3)];
        parts.push(format!(
            "Filter {} is a {kind} at {:.0} Hz, resonance {:.0} %, drive {:.0} %, to {}.",
            f + 1,
            get(p(filter::CUTOFF)),
            get(p(filter::RESONANCE)),
            get(p(filter::DRIVE)),
            if output == "Other filter" {
                format!("filter {} (in series)", 2 - f)
            } else {
                output.to_lowercase()
            }
        ));
    }
    let env = |param: u16| get(params::env_id(0, param));
    parts.push(format!(
        "Env 1 shapes each voice: attack {:.1} ms, decay {:.0} ms, sustain {}, release {:.0} ms.",
        env(params::env::ATTACK),
        env(params::env::DECAY),
        sustain_text(env(params::env::SUSTAIN)),
        env(params::env::RELEASE)
    ));
    let mono = on(global::MONO);
    parts.push(format!(
        "{} with up to {} voices, at {} quality.",
        if mono { "Mono" } else { "Polyphonic" },
        get(global::VOICES).round(),
        QUALITIES[(get(global::QUALITY).round() as usize).min(2)].to_lowercase()
    ));
    if !routes.is_empty() {
        let list: Vec<String> = routes
            .iter()
            .filter(|r| !r.bypass)
            .map(|r| {
                let source = r
                    .source
                    .as_deref()
                    .and_then(|k| sources.find(k).and_then(|id| sources.get(id)))
                    .map_or("nothing".to_owned(), |s| s.label.clone());
                let target = match r.destination {
                    Some(Target::Param(id)) => name_of(id),
                    Some(Target::LfoBus { lfo, bus }) => format!("LFO {} bus {}", lfo + 1, bus + 1),
                    None => "nothing".to_owned(),
                };
                format!("{source} to {target} by {:+.0} %", r.amount * 100.0)
            })
            .collect();
        parts.push(format!("Modulation: {}.", list.join(", ")));
    }
    parts.join(" ")
}

fn sustain_text(gain: f64) -> String {
    if gain <= 0.0 {
        "-inf dB".to_owned()
    } else {
        format!("{:.1} dB", 20.0 * gain.log10())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_init_patch_reads_as_a_saw_through_the_filter() {
        let space = params::space();
        let sources = crate::modulation::sources();
        let defaults: Vec<(ParamId, f64)> = space
            .descriptors()
            .iter()
            .map(|d| (d.id, d.default))
            .collect();
        let value_of = |id: ParamId| defaults.iter().find(|(i, _)| *i == id).map(|(_, v)| *v);
        let text = summary(&space, &sources, &[], &value_of);
        assert!(text.contains("oscillator A at 75 % level"), "{text}");
        assert!(text.contains("to filter"), "{text}");
        assert!(text.contains("up to 8 voices, at high quality"), "{text}");
        assert!(!text.contains("Filter 1 is"), "Filter 1 is off: {text}");
    }
}
