//! Parameter schema: slider ranges, defaults, FX metadata — parsed once from
//! `assets/schema.json`, which `scripts/dump-presets.mjs` dumps out of the web
//! app. The numbers therefore have exactly one source of truth (PLAN.md
//! decision 2); nothing here is re-typed by hand.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::modmatrix::{LfoDef, ParamRanges};
use crate::state::CardState;

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

/// Vowel formants (F1, F2, F3 in Hz) and their relative amplitudes, in the
/// order A E I O U — the Vowel slider morphs between them.
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Vowel {
    pub f: [f64; 3],
    pub a: [f64; 3],
}

#[derive(Clone, Debug, Deserialize)]
pub struct SelectOption {
    pub v: f64,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SelectDef {
    pub k: String,
    pub name: String,
    pub value: f64,
    pub options: Vec<SelectOption>,
}

/// A visual card (the image side), carried through the point, mutated by
/// evolution (PLAN.md decision 2) and drawn by `crate::sim`.
#[derive(Clone, Debug, Deserialize)]
pub struct CardDef {
    pub id: String,
    pub title: String,
    pub tag: String,
    pub desc: String,
    pub sliders: Vec<SliderDef>,
    #[serde(default)]
    pub selects: Vec<SelectDef>,
}

/// One gene of the genome, as the web app derives it from these same schemas.
/// The list is dumped rather than re-derived, so the two apps cannot disagree
/// about gene order — share-compatibility depends on it (PLAN.md decision 2).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneDef {
    pub id: String,
    pub label: String,
    pub kind: GeneKind,
    /// Gene family for structural mutation: `a.<formula>`, `fx`, `v.<card>`,
    /// `lfo.<i>`, `route.<i>`, `coupling`.
    pub group: String,
    pub min: f64,
    pub max: f64,
    pub step: Option<f64>,
    #[serde(default)]
    pub exp: bool,
    /// The bool gene that gates this one (a disabled formula's params, an off
    /// card's, an unused route slot).
    pub active_if: Option<String>,
    /// Normalized value at which the gene has no effect, so a morph can fade
    /// it in and out instead of jumping.
    pub neutral: Option<f64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GeneKind {
    Cont,
    Bool,
    Choice,
}

/// A parameter a modulation route can aim at.
#[derive(Clone, Debug, Deserialize)]
pub struct ModTarget {
    pub target: String,
    pub param: String,
    #[serde(default)]
    pub exp: bool,
    pub label: String,
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
    pub cards: Vec<CardDef>,
    pub always_on_card_ids: Vec<String>,
    pub vowels: Vec<Vowel>,
    pub genes: Vec<GeneDef>,
    pub mod_targets: Vec<ModTarget>,
    pub route_slots: usize,
    pub lfo_shapes: Vec<String>,
    pub lfo_rate_range: [f64; 2],
    pub fx_on_keys: Vec<String>,
    pub default_coupling: BTreeMap<String, f64>,
    pub default_state: serde_json::Value,
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

/// Visual cards at their defaults — which cards start on is taken from the
/// dumped default point, not guessed.
pub fn default_cards() -> BTreeMap<String, CardState> {
    let on_by_default = |id: &str| -> bool {
        schema()
            .default_state
            .pointer(&format!("/visual/cards/{id}/on"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    };
    schema()
        .cards
        .iter()
        .map(|c| {
            let params = c.sliders.iter().map(|s| (s.k.clone(), s.value)).collect();
            (c.id.clone(), CardState { on: on_by_default(&c.id), params })
        })
        .collect()
}

/// Formants at slider position `v` (0..1), interpolated between the vowels.
pub fn vowel_formants(v: f64) -> Vowel {
    let vowels = &schema().vowels;
    let pos = v.clamp(0.0, 1.0) * (vowels.len() - 1) as f64;
    let i0 = (pos.floor() as usize).min(vowels.len() - 2);
    let fr = pos - i0 as f64;
    let (a, b) = (&vowels[i0], &vowels[i0 + 1]);
    let lerp = |x: f64, y: f64| x + (y - x) * fr;
    Vowel {
        f: [lerp(a.f[0], b.f[0]), lerp(a.f[1], b.f[1]), lerp(a.f[2], b.f[2])],
        a: [lerp(a.a[0], b.a[0]), lerp(a.a[1], b.a[1]), lerp(a.a[2], b.a[2])],
    }
}

pub fn default_coupling() -> BTreeMap<String, f64> {
    schema().default_coupling.clone()
}

pub fn card_def(id: &str) -> Option<&'static CardDef> {
    schema().cards.iter().find(|c| c.id == id)
}

/// Slider ranges of one visual card.
pub fn card_ranges(id: &str) -> ParamRanges {
    card_def(id)
        .map(|def| def.sliders.iter().map(|s| (s.k.clone(), [s.min, s.max])).collect())
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
