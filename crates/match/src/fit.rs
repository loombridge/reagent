//! Curve fitting: which law a measured curve follows, and its constants.

use serde::Serialize;

/// The laws a curve is fitted to.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Law {
    /// `a + b·x`.
    Linear,
    /// `a + b·x^exponent`, for the exponent in 0.05 to 6.
    Power {
        /// The exponent.
        exponent: f64,
    },
    /// `a + b·ln x`.
    Log,
    /// `a·e^(b·x)`.
    Exp,
}

/// A law and its constants, with how far the curve is from the points.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Fit {
    /// The law.
    pub law: Law,
    /// Its first constant.
    pub a: f64,
    /// Its second.
    pub b: f64,
    /// The root-mean-square distance of the points from the curve, in the
    /// points' own unit.
    pub error: f64,
}

/// Least squares `y = a + b·t` over `(t, y)`.
fn line(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    let n = points.len() as f64;
    let (st, sy) = points
        .iter()
        .fold((0.0, 0.0), |(st, sy), (t, y)| (st + t, sy + y));
    let (mt, my) = (st / n, sy / n);
    let var: f64 = points.iter().map(|(t, _)| (t - mt).powi(2)).sum();
    if var <= 0.0 {
        return None;
    }
    let b = points.iter().map(|(t, y)| (t - mt) * (y - my)).sum::<f64>() / var;
    Some((my - b * mt, b))
}

/// The error of `y ≈ a + b·t`, over the points' `y`.
fn error(points: &[(f64, f64)], a: f64, b: f64) -> f64 {
    let sum: f64 = points.iter().map(|(t, y)| (y - (a + b * t)).powi(2)).sum();
    (sum / points.len() as f64).sqrt()
}

/// The law `points` (`x`, `y`) follow best, or `None` if no law fits (fewer
/// than three points, or all at one `x`).
///
/// A law replaces an earlier one in the order linear, power, log, exp only
/// if it halves the error, so noise doesn't turn a straight line into a
/// curve.
#[must_use]
pub fn fit(points: &[(f64, f64)]) -> Option<Fit> {
    if points.len() < 3 {
        return None;
    }
    let mut best: Option<Fit> = None;
    let mut offer = |candidate: Option<Fit>| {
        if let Some(c) = candidate.filter(|c| c.error.is_finite())
            && best.is_none_or(|b| c.error < b.error * 0.5)
        {
            best = Some(c);
        }
    };
    let by = |law, a, b| Fit {
        law,
        a,
        b,
        error: error(points, a, b),
    };
    offer(line(points).map(|(a, b)| by(Law::Linear, a, b)));
    // The power law's exponent is found on a grid of 0.01; each exponent's
    // constants are the straight line through (x^exponent, y).
    let power = (5..=600)
        .filter_map(|step| {
            let exponent = f64::from(step) / 100.0;
            let curved: Vec<(f64, f64)> = points
                .iter()
                .map(|(x, y)| (x.max(0.0).powf(exponent), *y))
                .collect();
            let (a, b) = line(&curved)?;
            Some(Fit {
                law: Law::Power { exponent },
                a,
                b,
                error: error(&curved, a, b),
            })
        })
        .min_by(|p, q| p.error.total_cmp(&q.error));
    offer(power);
    if points.iter().all(|(x, _)| *x > 0.0) {
        let logs: Vec<(f64, f64)> = points.iter().map(|(x, y)| (x.ln(), *y)).collect();
        offer(line(&logs).map(|(a, b)| Fit {
            law: Law::Log,
            a,
            b,
            error: error(&logs, a, b),
        }));
    }
    if points.iter().all(|(_, y)| *y > 0.0) {
        let logs: Vec<(f64, f64)> = points.iter().map(|(x, y)| (*x, y.ln())).collect();
        offer(line(&logs).map(|(ln_a, b)| {
            let a = ln_a.exp();
            let sum: f64 = points
                .iter()
                .map(|(x, y)| (y - a * (b * x).exp()).powi(2))
                .sum();
            Fit {
                law: Law::Exp,
                a,
                b,
                error: (sum / points.len() as f64).sqrt(),
            }
        }));
    }
    best
}
