//! Sound → image, the two ways the web app does it:
//! - card couplings (`src/coupling.ts`) offset visual card params every frame
//!   and show up with the simulation's lag;
//! - display couplings (`src/visualFx.ts`) act on the shown frame directly —
//!   exposure pulse, onset flash, spectrum tint — plus ripples from hits.

use std::collections::BTreeMap;

use crate::features::AudioFeatures;
use crate::schema::card_ranges;
use crate::state::Params;

#[derive(Clone, Copy, Debug)]
pub enum Feature {
    Loudness,
    Brightness,
    Onset,
}

impl Feature {
    fn of(self, f: &AudioFeatures) -> f64 {
        match self {
            Self::Loudness => f.loudness,
            Self::Brightness => f.brightness,
            Self::Onset => f.onset,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CouplingDef {
    pub key: &'static str,
    pub card: &'static str,
    pub param: &'static str,
    pub feature: Feature,
    /// Offset at feature = 1 and coupling = 1, in the param's units.
    pub scale: f64,
    /// The feature is used as `value − 0.5`, so its midpoint is neutral.
    pub centered: bool,
    /// Wraps around the slider range instead of clamping.
    pub wrap: bool,
}

pub const COUPLING_DEFS: [CouplingDef; 5] = [
    CouplingDef {
        key: "loudToFlow",
        card: "flow",
        param: "advectAmount",
        feature: Feature::Loudness,
        scale: 0.5,
        centered: false,
        wrap: false,
    },
    CouplingDef {
        key: "loudToCurl",
        card: "flow",
        param: "curlStrength",
        feature: Feature::Loudness,
        scale: 0.015,
        centered: false,
        wrap: false,
    },
    CouplingDef {
        key: "loudToGloss",
        card: "palette",
        param: "gloss",
        feature: Feature::Loudness,
        scale: 0.6,
        centered: false,
        wrap: false,
    },
    CouplingDef {
        key: "brightToShift",
        card: "palette",
        param: "shift",
        feature: Feature::Brightness,
        scale: 0.5,
        centered: true,
        wrap: true,
    },
    CouplingDef {
        key: "onsetToLight",
        card: "palette",
        param: "lightAngle",
        feature: Feature::Onset,
        scale: 1.5,
        centered: false,
        wrap: true,
    },
];

/// The coupling genes that act on the display or the field rather than on a
/// card param.
pub const EXPLICIT_COUPLING_KEYS: [&str; 4] =
    ["loudToPulse", "onsetToFlash", "onsetToSeed", "spectrumToTint"];

/// Card params with the coupling offsets applied (`applyCoupling`).
pub fn apply_coupling(
    cards: &BTreeMap<String, Params>,
    features: &AudioFeatures,
    coupling: &BTreeMap<String, f64>,
) -> BTreeMap<String, Params> {
    let mut out = cards.clone();
    for d in &COUPLING_DEFS {
        let c = coupling.get(d.key).copied().unwrap_or(0.0);
        let mut f = d.feature.of(features);
        if d.centered {
            f -= 0.5;
        }
        let delta = c * f * d.scale;
        if delta == 0.0 {
            continue;
        }
        let Some(value) = out.get_mut(d.card).and_then(|p| p.get_mut(d.param)) else { continue };
        let [lo, hi] =
            card_ranges(d.card).get(d.param).copied().unwrap_or([f64::NEG_INFINITY, f64::INFINITY]);
        let v = *value + delta;
        *value = if d.wrap && lo.is_finite() && hi.is_finite() {
            lo + (v - lo).rem_euclid(hi - lo)
        } else {
            v.clamp(lo, hi)
        };
    }
    out
}

/// What the display pass does on top of the field this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayFx {
    /// Brightness multiplier, 1 = unchanged.
    pub exposure: f32,
    /// Highlight flare, 0..1.
    pub flash: f32,
    /// Tint strengths for dark, mid and light tones, each 0..1.
    pub tint: [f32; 3],
}

impl DisplayFx {
    pub const NEUTRAL: Self = Self { exposure: 1.0, flash: 0.0, tint: [0.0; 3] };
}

/// Exposure ±60% at full swell and a full gene.
const PULSE_GAIN: f64 = 0.6;

pub fn display_coupling(f: &AudioFeatures, coupling: &BTreeMap<String, f64>) -> DisplayFx {
    let gene = |k: &str| coupling.get(k).copied().unwrap_or(0.0);
    let (pulse, flash, tint) = (gene("loudToPulse"), gene("onsetToFlash"), gene("spectrumToTint"));
    DisplayFx {
        exposure: (1.0 + PULSE_GAIN * pulse * f.swell) as f32,
        flash: (flash * f.onset) as f32,
        tint: [(tint * f.low) as f32, (tint * f.mid) as f32, (tint * f.high) as f32],
    }
}

pub const MAX_RIPPLES: usize = 4;
/// Seconds a ripple lives.
pub const RIPPLE_LIFE: f64 = 1.6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ripple {
    /// UV, 0..1, Y down.
    pub x: f32,
    pub y: f32,
    /// Seconds since the hit.
    pub age: f32,
    /// 0..1.
    pub amp: f32,
}

/// A small FIFO of live ripples, the oldest dropped first.
#[derive(Clone, Debug, Default)]
pub struct RippleSet {
    /// `(x, y, amp, t0)`.
    list: Vec<(f32, f32, f32, f64)>,
}

impl RippleSet {
    pub fn add(&mut self, x: f32, y: f32, amp: f32, t: f64) {
        self.list.push((x, y, amp, t));
        if self.list.len() > MAX_RIPPLES {
            self.list.remove(0);
        }
    }

    /// The ripples alive at `t`; the dead ones are dropped.
    pub fn active(&mut self, t: f64) -> Vec<Ripple> {
        self.list.retain(|r| t - r.3 < RIPPLE_LIFE);
        self.list.iter().map(|&(x, y, amp, t0)| Ripple { x, y, age: (t - t0).max(0.0) as f32, amp }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{card_def, default_coupling, schema};
    use crate::state::AppState;

    fn cards() -> BTreeMap<String, Params> {
        AppState::new().visual.cards.into_iter().map(|(id, c)| (id, c.params)).collect()
    }

    fn zero_coupling() -> BTreeMap<String, f64> {
        schema().coupling_keys.iter().map(|k| (k.clone(), 0.0)).collect()
    }

    fn features(f: impl FnOnce(&mut AudioFeatures)) -> AudioFeatures {
        let mut x = AudioFeatures::default();
        f(&mut x);
        x
    }

    fn max_of(card: &str, param: &str) -> f64 {
        card_def(card).unwrap().sliders.iter().find(|s| s.k == param).unwrap().max
    }

    #[test]
    fn card_and_explicit_couplings_cover_every_gene() {
        let mut covered: Vec<&str> =
            COUPLING_DEFS.iter().map(|d| d.key).chain(EXPLICIT_COUPLING_KEYS).collect();
        covered.sort_unstable();
        let mut keys: Vec<&str> = schema().coupling_keys.iter().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(covered, keys);
    }

    #[test]
    fn zero_coupling_or_silence_leaves_params_alone() {
        let base = cards();
        let loud = features(|f| {
            f.loudness = 1.0;
            f.brightness = 1.0;
            f.onset = 1.0;
        });
        assert_eq!(apply_coupling(&base, &loud, &zero_coupling()), base);
        let mut c = zero_coupling();
        c.insert("loudToFlow".into(), 1.0);
        assert_eq!(apply_coupling(&base, &AudioFeatures::default(), &c), base);
    }

    #[test]
    fn loudness_pushes_advection_curl_and_gloss_within_their_ranges() {
        let base = cards();
        let mut c = zero_coupling();
        for k in ["loudToFlow", "loudToCurl", "loudToGloss"] {
            c.insert(k.into(), 1.0);
        }
        let loud = features(|f| {
            f.loudness = 1.0;
            f.brightness = 0.5;
        });
        let out = apply_coupling(&base, &loud, &c);
        assert!(out["flow"]["advectAmount"] > base["flow"]["advectAmount"]);
        assert!(out["flow"]["curlStrength"] > base["flow"]["curlStrength"]);
        assert!(out["palette"]["gloss"] > base["palette"]["gloss"]);
        assert!(out["palette"]["gloss"] <= max_of("palette", "gloss"));
        assert!(out["flow"]["advectAmount"] <= max_of("flow", "advectAmount"));
        c.insert("loudToGloss".into(), -1.0);
        assert!(apply_coupling(&base, &loud, &c)["palette"]["gloss"] < base["palette"]["gloss"]);
    }

    #[test]
    fn brightness_shifts_the_hue_cyclically_and_onsets_turn_the_light() {
        let base = cards();
        let mut c = zero_coupling();
        c.insert("brightToShift".into(), 1.0);
        c.insert("onsetToLight".into(), 1.0);
        let hi = apply_coupling(
            &base,
            &features(|f| {
                f.brightness = 1.0;
                f.onset = 1.0;
            }),
            &c,
        );
        let (shift, light) = (hi["palette"]["shift"], hi["palette"]["lightAngle"]);
        assert!(shift != base["palette"]["shift"] && (0.0..1.0).contains(&shift));
        assert!(light != base["palette"]["lightAngle"] && (0.0..std::f64::consts::TAU).contains(&light));
        let mid = apply_coupling(&base, &features(|f| f.brightness = 0.5), &c);
        assert!((mid["palette"]["shift"] - base["palette"]["shift"]).abs() < 1e-12);
    }

    fn explicit(over: &[(&str, f64)]) -> BTreeMap<String, f64> {
        let mut c = default_coupling();
        for k in EXPLICIT_COUPLING_KEYS {
            c.insert(k.into(), 0.0);
        }
        for &(k, v) in over {
            c.insert(k.into(), v);
        }
        c
    }

    #[test]
    fn silence_or_zero_genes_give_a_neutral_display() {
        assert_eq!(display_coupling(&AudioFeatures::default(), &default_coupling()), DisplayFx::NEUTRAL);
        let loud = AudioFeatures {
            loudness: 1.0,
            swell: 1.0,
            onset: 1.0,
            low: 1.0,
            mid: 1.0,
            high: 1.0,
            brightness: 0.0,
        };
        assert_eq!(display_coupling(&loud, &explicit(&[])), DisplayFx::NEUTRAL);
    }

    #[test]
    fn exposure_follows_the_swell_both_ways_and_scales_with_the_gene() {
        let up = display_coupling(&features(|f| f.swell = 1.0), &explicit(&[("loudToPulse", 1.0)]));
        let down = display_coupling(&features(|f| f.swell = -1.0), &explicit(&[("loudToPulse", 1.0)]));
        let half = display_coupling(&features(|f| f.swell = 1.0), &explicit(&[("loudToPulse", 0.5)]));
        assert!(up.exposure > 1.2);
        assert!(down.exposure < 0.8 && down.exposure > 0.0);
        assert!(((half.exposure - 1.0) - (up.exposure - 1.0) / 2.0).abs() < 1e-6);
    }

    #[test]
    fn flash_follows_the_onset_and_tint_the_bands() {
        let f = display_coupling(&features(|f| f.onset = 0.5), &explicit(&[("onsetToFlash", 0.5)]));
        assert!((f.flash - 0.25).abs() < 1e-6);
        let bands = features(|f| {
            f.low = 1.0;
            f.mid = 0.5;
        });
        let d = display_coupling(&bands, &explicit(&[("spectrumToTint", 0.8)]));
        assert!((d.tint[0] - 0.8).abs() < 1e-6 && (d.tint[1] - 0.4).abs() < 1e-6 && d.tint[2] == 0.0);
    }

    #[test]
    fn ripples_age_and_expire() {
        let mut r = RippleSet::default();
        r.add(0.2, 0.3, 0.8, 10.0);
        assert_eq!(r.active(10.5), vec![Ripple { x: 0.2, y: 0.3, age: 0.5, amp: 0.8 }]);
        assert!(r.active(10.0 + RIPPLE_LIFE + 0.01).is_empty());
    }

    #[test]
    fn at_most_four_ripples_live_and_the_oldest_goes_first() {
        let mut r = RippleSet::default();
        for i in 0..MAX_RIPPLES + 2 {
            r.add(i as f32 / 10.0, 0.5, 1.0, i as f64 * 0.01);
        }
        let a = r.active(0.1);
        assert_eq!(a.len(), MAX_RIPPLES);
        assert!((a[0].x - 0.2).abs() < 1e-6);
    }
}
