//! The gene list and the mapping between normalized genes and real values.
//!
//! The list itself comes from `assets/schema.json` (dumped from the web app),
//! so gene order — which share compatibility depends on — cannot drift.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::schema::{schema, GeneDef, GeneKind, ModTarget};

/// A point as the search sees it: one number per gene, continuous genes in
/// [0,1], bools 0/1, choices an integer index.
pub type Genome = Vec<f64>;

pub fn genes() -> &'static [GeneDef] {
    &schema().genes
}

pub fn mod_targets() -> &'static [ModTarget] {
    &schema().mod_targets
}

pub fn route_slots() -> usize {
    schema().route_slots
}

fn index_map() -> &'static HashMap<&'static str, usize> {
    static MAP: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();
    MAP.get_or_init(|| genes().iter().enumerate().map(|(i, g)| (g.id.as_str(), i)).collect())
}

pub fn gene_index_of(id: &str) -> Option<usize> {
    index_map().get(id).copied()
}

/// Index of a gene id. Panics for an unknown id — that is a programming
/// mistake, not user data.
pub fn gene_index(id: &str) -> usize {
    gene_index_of(id).unwrap_or_else(|| panic!("unknown gene {id}"))
}

pub fn gene_by_id(id: &str) -> Option<&'static GeneDef> {
    gene_index_of(id).map(|i| &genes()[i])
}

pub fn mod_target_index(target: &str, param: &str) -> Option<usize> {
    mod_targets().iter().position(|t| t.target == target && t.param == param)
}

/// Normalized gene value → real parameter value. Bool and choice pass through.
pub fn gene_value(def: &GeneDef, x: f64) -> f64 {
    if def.kind != GeneKind::Cont {
        return x;
    }
    let t = x.clamp(0.0, 1.0);
    if def.exp && def.min > 0.0 {
        def.min * (def.max / def.min).powf(t)
    } else {
        def.min + t * (def.max - def.min)
    }
}

/// Real parameter value → normalized gene value.
pub fn gene_from_value(def: &GeneDef, v: f64) -> f64 {
    if def.kind != GeneKind::Cont {
        return v;
    }
    if def.max == def.min {
        return 0.0;
    }
    if def.exp && def.min > 0.0 {
        let safe = v.clamp(def.min, def.max);
        ((safe / def.min).ln() / (def.max / def.min).ln()).clamp(0.0, 1.0)
    } else {
        ((v - def.min) / (def.max - def.min)).clamp(0.0, 1.0)
    }
}

/// Whether gene `i` currently matters — its gate, if it has one, is on.
pub fn is_gene_active(genome: &[f64], i: usize) -> bool {
    match genes()[i].active_if.as_deref() {
        None => true,
        Some(gate) => match gene_index_of(gate) {
            None => true,
            Some(gi) => genome[gi] == 1.0,
        },
    }
}

/// Real value of a gene, rounded when its slider steps in whole units.
pub fn read_value(genome: &[f64], id: &str) -> f64 {
    let i = gene_index(id);
    let def = &genes()[i];
    let v = gene_value(def, genome[i]);
    if def.step.is_some_and(|s| s >= 1.0) {
        v.round()
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gene_list_is_the_one_the_web_app_derives() {
        let g = genes();
        assert_eq!(g.len(), 237, "gene count changed — share compatibility depends on it");
        assert_eq!(g[0].id, "a.additive.enabled");
        assert_eq!(gene_index("fx.filterOn"), gene_index_of("fx.filterOn").unwrap());
        assert_eq!(route_slots(), 12);
        assert_eq!(mod_targets().len(), 122);
    }

    #[test]
    fn values_round_trip_through_the_normalization() {
        for def in genes().iter().filter(|d| d.kind == GeneKind::Cont) {
            for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let real = gene_value(def, t);
                let back = gene_from_value(def, real);
                assert!((back - t).abs() < 1e-9, "{}: {t} → {real} → {back}", def.id);
            }
        }
    }

    #[test]
    fn a_gate_decides_whether_its_genes_matter() {
        let mut g = vec![0.0; genes().len()];
        let gate = gene_index("a.additive.enabled");
        let gain = gene_index("a.additive.gain");
        assert!(!is_gene_active(&g, gain));
        g[gate] = 1.0;
        assert!(is_gene_active(&g, gain));
        assert!(is_gene_active(&g, gate), "an ungated gene is always active");
    }
}
