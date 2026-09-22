//! Parameter schema: slider ranges, defaults, FX metadata — parsed once from
//! `assets/schema.json`, which `scripts/dump-presets.mjs` dumps out of the web
//! app. The numbers therefore have exactly one source of truth (PLAN.md
//! decision 2); nothing here is re-typed by hand.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::modmatrix::{LfoDef, ParamRanges};

const SCHEMA_JSON: &str = include_str!("../../assets/schema.json");

#[derive(Clone, Debug, Deserialize)]
pub struct SliderDef {
    pub k: String,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub value: f64,
    /// Perceptually logarithmic (frequency-like): mutate and modulate in octaves.
    #[serde(default)]
    pub exp: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FormulaDef {
    pub id: String,
    pub title: String,
    pub tag: String,
    pub desc: String,
    pub sliders: Vec<SliderDef>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schema {
    pub formula_ids: Vec<String>,
    pub default_params: BTreeMap<String, f64>,
    pub formulas: Vec<FormulaDef>,
    pub max_enabled_formulas: usize,
    pub filter_types: Vec<String>,
    pub chorus_modes: Vec<String>,
    pub phaser_stages: Vec<f64>,
    pub fx_mod_params: Vec<String>,
    pub fx_param_ranges: BTreeMap<String, [f64; 2]>,
    pub fx_param_module: BTreeMap<String, String>,
    pub fx_exp_params: Vec<String>,
    pub reverb_decay_range: [f64; 2],
    pub lfo_count: usize,
    pub default_lfo: LfoDef,
    pub default_master_gain: f64,
    pub coupling_keys: Vec<String>,
    pub coupling_ranges: BTreeMap<String, [f64; 2]>,
    pub coupling_floor: f64,
}

/// The parsed schema (parsed on first use, then shared).
pub fn schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| serde_json::from_str(SCHEMA_JSON).expect("assets/schema.json is valid"))
}

/// The shared default pool every generator starts from, before its own
/// slider defaults and the point's values are laid on top.
pub fn default_params() -> BTreeMap<String, f64> {
    schema().default_params.clone()
}

pub fn formula_def(id: &str) -> Option<&'static FormulaDef> {
    schema().formulas.iter().find(|f| f.id == id)
}

/// The shared default pool plus this formula's own slider defaults — what a
/// fresh point starts from.
pub fn formula_defaults(id: &str) -> BTreeMap<String, f64> {
    let mut out = default_params();
    if let Some(def) = formula_def(id) {
        for s in &def.sliders {
            out.insert(s.k.clone(), s.value);
        }
    }
    out
}

/// Slider ranges of one formula, for modulation and mutation.
pub fn formula_ranges(id: &str) -> ParamRanges {
    formula_def(id)
        .map(|def| def.sliders.iter().map(|s| (s.k.clone(), [s.min, s.max])).collect())
        .unwrap_or_default()
}

/// Which slider keys of one formula are frequency-like.
pub fn formula_exp_params(id: &str) -> Vec<String> {
    formula_def(id)
        .map(|def| def.sliders.iter().filter(|s| s.exp).map(|s| s.k.clone()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dump_carries_every_formula() {
        let s = schema();
        assert_eq!(s.formula_ids.len(), 21);
        assert_eq!(s.formulas.len(), 21);
        for id in &s.formula_ids {
            let def = formula_def(id).unwrap_or_else(|| panic!("no def for {id}"));
            assert!(def.sliders.iter().any(|s| s.k == "gain"), "{id} has no gain");
        }
        assert_eq!(s.max_enabled_formulas, 5);
        assert_eq!(s.lfo_count, 4);
    }

    #[test]
    fn defaults_sit_inside_their_ranges() {
        for def in &schema().formulas {
            for s in &def.sliders {
                assert!(s.min <= s.value && s.value <= s.max, "{}.{} = {}", def.id, s.k, s.value);
            }
        }
    }
}
