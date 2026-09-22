//! Modulation matrix (pure). An LFO is a function of absolute time, so the
//! audio thread and every visualizer stay in sync by reading the same clock
//! instead of messaging each other (PLAN.md decision 7).
//!
//! Ported from `../synesthesia/src/dsp/mod.ts`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

const TWO_PI: f64 = std::f64::consts::TAU;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LfoShape {
    Sine,
    Triangle,
    Saw,
    Square,
    /// Sample & hold: a pure function of the cycle number.
    Random,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct LfoDef {
    pub shape: LfoShape,
    /// Hz — slow, ~0.005–8.
    pub rate: f64,
    /// 0–1, a fraction of a cycle.
    pub phase: f64,
}

impl Default for LfoDef {
    fn default() -> Self {
        Self { shape: LfoShape::Sine, rate: 0.05, phase: 0.0 }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ModRoute {
    /// Index into the LFO pool.
    pub src: usize,
    /// `"fx"`, a formula id, or a visual card id — the three never collide.
    pub target: String,
    /// Slider key or `FxState` field.
    pub param: String,
    /// Bipolar fraction of the range, [-1, 1].
    pub depth: f64,
    /// Exponential (octave) mapping for frequency-like params.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exp: bool,
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct ModState {
    pub lfos: Vec<LfoDef>,
    pub routes: Vec<ModRoute>,
}

/// param → [min, max]; the ranges live in the schema and are passed in.
pub type ParamRanges = BTreeMap<String, [f64; 2]>;

/// Deterministic integer hash → [0,1) (one mulberry32 step), so sample & hold
/// is a pure function of the cycle number.
fn hash01(n: f64) -> f64 {
    let a = (n as i64 as i32 as u32).wrapping_add(0x6d2b_79f5);
    let mut t = (a ^ (a >> 15)).wrapping_mul(a | 1);
    t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
    f64::from(t ^ (t >> 14)) / 4_294_967_296.0
}

/// LFO value at absolute time `t` (seconds) → [-1, 1].
pub fn lfo_value(lfo: &LfoDef, t: f64) -> f64 {
    let ph = lfo.rate * t + lfo.phase;
    let frac = ph - ph.floor();
    match lfo.shape {
        LfoShape::Sine => (TWO_PI * ph).sin(),
        LfoShape::Triangle => {
            if frac < 0.5 {
                4.0 * frac - 1.0
            } else {
                3.0 - 4.0 * frac
            }
        }
        LfoShape::Saw => 2.0 * frac - 1.0,
        LfoShape::Square => {
            if frac < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        LfoShape::Random => 2.0 * hash01(ph.floor()) - 1.0,
    }
}

/// Base plus a bipolar fraction of the range times the LFO, clamped.
/// Frequency-like params move in octaves.
pub fn effective_param(base: f64, l: f64, depth: f64, range: [f64; 2], exp: bool) -> f64 {
    let [min, max] = range;
    if exp && min > 0.0 && max > 0.0 {
        let octaves = (max / min).log2();
        (base * (depth * octaves * l).exp2()).clamp(min, max)
    } else {
        (base + depth * (max - min) * l).clamp(min, max)
    }
}

/// Applies every route aimed at `target` on top of `base` at time `t`.
/// Params without a route (or with a missing LFO or range) pass through.
pub fn effective_params(
    target: &str,
    base: &BTreeMap<String, f64>,
    lfos: &[LfoDef],
    routes: &[ModRoute],
    ranges: &ParamRanges,
    t: f64,
) -> BTreeMap<String, f64> {
    let mut out = base.clone();
    for route in routes.iter().filter(|r| r.target == target) {
        let (Some(lfo), Some(range)) = (lfos.get(route.src), ranges.get(&route.param)) else {
            continue;
        };
        let Some(&base_val) = base.get(&route.param) else { continue };
        out.insert(
            route.param.clone(),
            effective_param(base_val, lfo_value(lfo, t), route.depth, *range, route.exp),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_stay_in_range_and_start_where_they_should() {
        let t = |shape| LfoDef { shape, rate: 1.0, phase: 0.0 };
        assert!((lfo_value(&t(LfoShape::Triangle), 0.0) + 1.0).abs() < 1e-12);
        assert!((lfo_value(&t(LfoShape::Triangle), 0.25) - 0.0).abs() < 1e-12);
        assert!((lfo_value(&t(LfoShape::Saw), 0.75) - 0.5).abs() < 1e-12);
        assert_eq!(lfo_value(&t(LfoShape::Square), 0.75), -1.0);
        for shape in [LfoShape::Sine, LfoShape::Triangle, LfoShape::Saw, LfoShape::Square, LfoShape::Random] {
            for i in 0..1000 {
                let v = lfo_value(&t(shape), f64::from(i) * 0.013);
                assert!((-1.0..=1.0).contains(&v), "{shape:?} {v}");
            }
        }
    }

    #[test]
    fn exponential_mapping_is_symmetric_in_octaves() {
        let range = [20.0, 2000.0];
        let up = effective_param(200.0, 1.0, 0.5, range, true);
        let down = effective_param(200.0, -1.0, 0.5, range, true);
        assert!((up * down - 200.0 * 200.0).abs() < 1e-6, "{up} {down}");
    }

    #[test]
    fn linear_mapping_clamps_to_the_range() {
        assert_eq!(effective_param(0.9, 1.0, 1.0, [0.0, 1.0], false), 1.0);
        assert_eq!(effective_param(0.1, -1.0, 1.0, [0.0, 1.0], false), 0.0);
    }
}
