//! The point: `AppState` v1, byte-compatible with the web app (PLAN.md
//! decision 2). Field names and shapes follow
//! `../synesthesia/src/state/schema.ts` exactly, so a point written here opens
//! there and the other way round.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::modmatrix::ModState;
use crate::schema::schema;

pub type Params = BTreeMap<String, f64>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormulaSnapshot {
    pub enabled: bool,
    pub params: Params,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FxState {
    pub filter_on: bool,
    pub filter_type: String,
    pub filter_freq: f64,
    pub filter_q: f64,
    pub filter_gain: f64,
    pub filter_vowel: f64,
    pub filter_comb_fb: f64,
    pub chorus_on: bool,
    pub chorus_mode: String,
    pub chorus_rate: f64,
    pub chorus_depth: f64,
    pub chorus_mix: f64,
    pub chorus_fb: f64,
    pub reverb_on: bool,
    pub reverb_decay: f64,
    pub reverb_mix: f64,
    pub limiter_on: bool,
    pub limiter_thr: f64,
    pub limiter_rel: f64,
    pub delay_on: bool,
    pub delay_time: f64,
    pub delay_fb: f64,
    pub delay_mix: f64,
    pub phaser_on: bool,
    pub phaser_rate: f64,
    pub phaser_depth: f64,
    pub phaser_stages: f64,
    pub phaser_fb: f64,
    pub phaser_mix: f64,
}

impl Default for FxState {
    /// The web app's `DEFAULT_FX`, read from the dumped schema.
    fn default() -> Self {
        serde_json::from_value(schema_default_fx()).expect("defaultFx matches FxState")
    }
}

fn schema_default_fx() -> serde_json::Value {
    let raw: serde_json::Value =
        serde_json::from_str(include_str!("../../assets/schema.json")).expect("schema.json");
    raw.get("defaultFx").cloned().expect("schema.json has defaultFx")
}

/// A modulatable FX field, read and written by name (what a `ModRoute` aims at).
impl FxState {
    pub fn get(&self, key: &str) -> Option<f64> {
        Some(match key {
            "filterFreq" => self.filter_freq,
            "filterQ" => self.filter_q,
            "filterGain" => self.filter_gain,
            "filterVowel" => self.filter_vowel,
            "filterCombFb" => self.filter_comb_fb,
            "chorusRate" => self.chorus_rate,
            "chorusDepth" => self.chorus_depth,
            "chorusMix" => self.chorus_mix,
            "chorusFb" => self.chorus_fb,
            "reverbMix" => self.reverb_mix,
            "reverbDecay" => self.reverb_decay,
            "delayTime" => self.delay_time,
            "delayFb" => self.delay_fb,
            "delayMix" => self.delay_mix,
            "phaserRate" => self.phaser_rate,
            "phaserDepth" => self.phaser_depth,
            "phaserFb" => self.phaser_fb,
            "phaserMix" => self.phaser_mix,
            "phaserStages" => self.phaser_stages,
            "limiterThr" => self.limiter_thr,
            "limiterRel" => self.limiter_rel,
            _ => return None,
        })
    }

    pub fn set(&mut self, key: &str, v: f64) {
        match key {
            "filterFreq" => self.filter_freq = v,
            "filterQ" => self.filter_q = v,
            "filterGain" => self.filter_gain = v,
            "filterVowel" => self.filter_vowel = v,
            "filterCombFb" => self.filter_comb_fb = v,
            "chorusRate" => self.chorus_rate = v,
            "chorusDepth" => self.chorus_depth = v,
            "chorusMix" => self.chorus_mix = v,
            "chorusFb" => self.chorus_fb = v,
            "reverbMix" => self.reverb_mix = v,
            "reverbDecay" => self.reverb_decay = v,
            "delayTime" => self.delay_time = v,
            "delayFb" => self.delay_fb = v,
            "delayMix" => self.delay_mix = v,
            "phaserRate" => self.phaser_rate = v,
            "phaserDepth" => self.phaser_depth = v,
            "phaserFb" => self.phaser_fb = v,
            "phaserMix" => self.phaser_mix = v,
            "phaserStages" => self.phaser_stages = v,
            "limiterThr" => self.limiter_thr = v,
            "limiterRel" => self.limiter_rel = v,
            _ => {}
        }
    }

    /// Is this module switched on? Used to skip routes aimed at a dead module.
    pub fn module_on(&self, on_key: &str) -> bool {
        match on_key {
            "filterOn" => self.filter_on,
            "chorusOn" => self.chorus_on,
            "reverbOn" => self.reverb_on,
            "limiterOn" => self.limiter_on,
            "delayOn" => self.delay_on,
            "phaserOn" => self.phaser_on,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioState {
    pub master_gain: f64,
    pub fx: FxState,
    pub formulas: BTreeMap<String, FormulaSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CardState {
    pub on: bool,
    pub params: Params,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VisualState {
    pub cards: BTreeMap<String, CardState>,
}

/// One point: the sound, the picture (`visual`, drawn by `crate::sim`), the
/// LFOs routed onto both, and the sound → image couplings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    pub v: u32,
    pub audio: AudioState,
    pub visual: VisualState,
    #[serde(rename = "mod")]
    pub modulation: ModState,
    pub coupling: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset_name: Option<String>,
}

impl AppState {
    /// A fresh point: every formula off at its slider defaults, default FX,
    /// four resting LFOs, no routes.
    pub fn new() -> Self {
        let s = schema();
        let formulas = s
            .formulas
            .iter()
            .map(|f| {
                let params = f.sliders.iter().map(|sl| (sl.k.clone(), sl.value)).collect();
                (f.id.clone(), FormulaSnapshot { enabled: false, params })
            })
            .collect();
        let cards = crate::schema::default_cards();
        AppState {
            v: 1,
            audio: AudioState { master_gain: s.default_master_gain, fx: FxState::default(), formulas },
            visual: VisualState { cards },
            modulation: ModState { lfos: vec![s.default_lfo; s.lfo_count], routes: Vec::new() },
            coupling: crate::schema::default_coupling(),
            preset_name: None,
        }
    }

    /// Enabled formulas, in schema order.
    pub fn enabled_formulas(&self) -> Vec<&str> {
        schema()
            .formula_ids
            .iter()
            .filter(|id| self.audio.formulas.get(*id).is_some_and(|f| f.enabled))
            .map(String::as_str)
            .collect()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// One built-in preset, as the web app has it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Preset {
    pub name: String,
    pub state: AppState,
}

/// The 12 built-in presets, dumped from the web app (PLAN.md decision 2).
pub fn presets() -> &'static [Preset] {
    static PRESETS: std::sync::OnceLock<Vec<Preset>> = std::sync::OnceLock::new();
    PRESETS.get_or_init(|| {
        serde_json::from_str(include_str!("../../assets/presets.json")).expect("assets/presets.json")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_twelve_presets_parse_and_round_trip() {
        let ps = presets();
        assert_eq!(ps.len(), 12);
        for p in ps {
            assert!(!p.state.enabled_formulas().is_empty(), "{} has no sound", p.name);
            let json = serde_json::to_string(&p.state).unwrap();
            let back: AppState = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, &p.state, "{} does not round-trip", p.name);
        }
    }

    #[test]
    fn a_fresh_point_matches_the_web_defaults() {
        let s = AppState::new();
        assert_eq!(s.v, 1);
        assert_eq!(s.audio.formulas.len(), 21);
        assert_eq!(s.modulation.lfos.len(), 4);
        assert!(s.audio.fx.limiter_on);
        assert_eq!(s.audio.fx.filter_type, "lowpass");
        assert_eq!(s.visual.cards.len(), 4);
    }

    #[test]
    fn fx_fields_are_reachable_by_the_names_routes_use() {
        let mut fx = FxState::default();
        for key in &schema().fx_mod_params {
            assert!(fx.get(key).is_some(), "no getter for {key}");
            fx.set(key, 0.25);
            assert_eq!(fx.get(key), Some(0.25), "setter for {key}");
        }
    }
}
