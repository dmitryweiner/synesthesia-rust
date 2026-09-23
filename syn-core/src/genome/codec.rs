//! `AppState` ⇄ `Genome`. Everything evolution may touch is a gene; what it
//! may not (the point's name, the master gain) is not, and decoding fills
//! those from the defaults.

use crate::modmatrix::{LfoDef, LfoShape, ModRoute};
use crate::schema::{schema, GeneKind};
use crate::state::AppState;

use super::genes::{
    gene_from_value, gene_index, genes, mod_target_index, mod_targets, read_value, route_slots, Genome,
};

fn set_value(g: &mut Genome, id: &str, real: f64) {
    let i = gene_index(id);
    g[i] = gene_from_value(&genes()[i], real);
}

fn set_raw(g: &mut Genome, id: &str, raw: f64) {
    g[gene_index(id)] = raw;
}

fn get_raw(g: &Genome, id: &str) -> f64 {
    g[gene_index(id)]
}

fn choice_index<T: PartialEq>(list: &[T], v: &T) -> f64 {
    list.iter().position(|x| x == v).unwrap_or(0) as f64
}

fn lfo_shape_name(shape: LfoShape) -> &'static str {
    match shape {
        LfoShape::Sine => "sine",
        LfoShape::Triangle => "triangle",
        LfoShape::Saw => "saw",
        LfoShape::Square => "square",
        LfoShape::Random => "random",
    }
}

fn lfo_shape_from(name: &str) -> LfoShape {
    match name {
        "triangle" => LfoShape::Triangle,
        "saw" => LfoShape::Saw,
        "square" => LfoShape::Square,
        "random" => LfoShape::Random,
        _ => LfoShape::Sine,
    }
}

fn always_on(card: &str) -> bool {
    schema().always_on_card_ids.iter().any(|id| id == card)
}

pub fn genome_length() -> usize {
    genes().len()
}

pub fn encode_genome(state: &AppState) -> Genome {
    let s = schema();
    let mut g: Genome = vec![0.0; genes().len()];

    for f in &s.formulas {
        let snap = state.audio.formulas.get(&f.id);
        set_raw(&mut g, &format!("a.{}.enabled", f.id), f64::from(snap.is_some_and(|x| x.enabled)));
        for sl in &f.sliders {
            let v = snap.and_then(|x| x.params.get(&sl.k)).copied().unwrap_or(sl.value);
            set_value(&mut g, &format!("a.{}.{}", f.id, sl.k), v);
        }
    }

    let fx = &state.audio.fx;
    for on in &s.fx_on_keys {
        set_raw(&mut g, &format!("fx.{on}"), f64::from(fx.module_on(on)));
    }
    for p in &s.fx_mod_params {
        set_value(&mut g, &format!("fx.{p}"), fx.get(p).unwrap_or(0.0));
    }
    set_value(&mut g, "fx.reverbDecay", fx.reverb_decay);
    set_raw(&mut g, "fx.filterType", choice_index(&s.filter_types, &fx.filter_type));
    set_raw(&mut g, "fx.chorusMode", choice_index(&s.chorus_modes, &fx.chorus_mode));
    set_raw(&mut g, "fx.phaserStages", choice_index(&s.phaser_stages, &fx.phaser_stages));

    for c in &s.cards {
        let card = state.visual.cards.get(&c.id);
        if !always_on(&c.id) {
            set_raw(&mut g, &format!("v.{}.on", c.id), f64::from(card.is_some_and(|x| x.on)));
        }
        for sel in &c.selects {
            let v = card.and_then(|x| x.params.get(&sel.k)).copied().unwrap_or(sel.value);
            let idx = sel.options.iter().position(|o| o.v == v).unwrap_or(0);
            set_raw(&mut g, &format!("v.{}.{}", c.id, sel.k), idx as f64);
        }
        for sl in &c.sliders {
            let v = card.and_then(|x| x.params.get(&sl.k)).copied().unwrap_or(sl.value);
            set_value(&mut g, &format!("v.{}.{}", c.id, sl.k), v);
        }
    }

    for i in 0..s.lfo_count {
        let lfo = state.modulation.lfos.get(i);
        set_raw(
            &mut g,
            &format!("lfo.{i}.shape"),
            lfo.map_or(0.0, |l| choice_index(&s.lfo_shapes, &lfo_shape_name(l.shape).to_string())),
        );
        set_value(&mut g, &format!("lfo.{i}.rate"), lfo.map_or(0.05, |l| l.rate));
        set_value(&mut g, &format!("lfo.{i}.phase"), lfo.map_or(0.0, |l| l.phase));
    }

    let mut slot = 0usize;
    for r in &state.modulation.routes {
        if slot >= route_slots() {
            break;
        }
        let Some(ti) = mod_target_index(&r.target, &r.param) else { continue };
        if r.src >= s.lfo_count {
            continue;
        }
        set_raw(&mut g, &format!("route.{slot}.on"), 1.0);
        set_raw(&mut g, &format!("route.{slot}.src"), r.src as f64);
        set_raw(&mut g, &format!("route.{slot}.target"), ti as f64);
        set_value(&mut g, &format!("route.{slot}.depth"), r.depth);
        set_raw(&mut g, &format!("route.{slot}.exp"), f64::from(r.exp));
        slot += 1;
    }
    for s in slot..route_slots() {
        set_raw(&mut g, &format!("route.{s}.on"), 0.0);
        set_raw(&mut g, &format!("route.{s}.src"), 0.0);
        set_raw(&mut g, &format!("route.{s}.target"), 0.0);
        set_value(&mut g, &format!("route.{s}.depth"), 0.0);
        set_raw(&mut g, &format!("route.{s}.exp"), 0.0);
    }

    for k in &s.coupling_keys {
        set_value(&mut g, &format!("c.{k}"), state.coupling.get(k).copied().unwrap_or(0.0));
    }
    g
}

pub fn decode_genome(g: &Genome) -> AppState {
    let s = schema();
    let mut state = AppState::new();

    for f in &s.formulas {
        if let Some(snap) = state.audio.formulas.get_mut(&f.id) {
            snap.enabled = get_raw(g, &format!("a.{}.enabled", f.id)) == 1.0;
            for sl in &f.sliders {
                snap.params.insert(sl.k.clone(), read_value(g, &format!("a.{}.{}", f.id, sl.k)));
            }
        }
    }

    for on in &s.fx_on_keys {
        let v = get_raw(g, &format!("fx.{on}")) == 1.0;
        match on.as_str() {
            "filterOn" => state.audio.fx.filter_on = v,
            "chorusOn" => state.audio.fx.chorus_on = v,
            "reverbOn" => state.audio.fx.reverb_on = v,
            "limiterOn" => state.audio.fx.limiter_on = v,
            "delayOn" => state.audio.fx.delay_on = v,
            "phaserOn" => state.audio.fx.phaser_on = v,
            _ => {}
        }
    }
    for p in &s.fx_mod_params {
        let v = read_value(g, &format!("fx.{p}"));
        state.audio.fx.set(p, v);
    }
    state.audio.fx.reverb_decay = read_value(g, "fx.reverbDecay");
    let pick = |list: &[String], id: &str| -> String {
        let i = get_raw(g, id) as usize;
        list.get(i).or_else(|| list.first()).cloned().unwrap_or_default()
    };
    state.audio.fx.filter_type = pick(&s.filter_types, "fx.filterType");
    state.audio.fx.chorus_mode = pick(&s.chorus_modes, "fx.chorusMode");
    let stage_i = get_raw(g, "fx.phaserStages") as usize;
    state.audio.fx.phaser_stages =
        s.phaser_stages.get(stage_i).copied().unwrap_or_else(|| s.phaser_stages[1]);

    for c in &s.cards {
        if let Some(card) = state.visual.cards.get_mut(&c.id) {
            if !always_on(&c.id) {
                card.on = get_raw(g, &format!("v.{}.on", c.id)) == 1.0;
            }
            for sel in &c.selects {
                let i = get_raw(g, &format!("v.{}.{}", c.id, sel.k)) as usize;
                let v = sel.options.get(i).map_or(sel.value, |o| o.v);
                card.params.insert(sel.k.clone(), v);
            }
            for sl in &c.sliders {
                card.params.insert(sl.k.clone(), read_value(g, &format!("v.{}.{}", c.id, sl.k)));
            }
        }
    }

    state.modulation.lfos = (0..s.lfo_count)
        .map(|i| LfoDef {
            shape: lfo_shape_from(
                s.lfo_shapes
                    .get(get_raw(g, &format!("lfo.{i}.shape")) as usize)
                    .map_or("sine", String::as_str),
            ),
            rate: read_value(g, &format!("lfo.{i}.rate")),
            phase: read_value(g, &format!("lfo.{i}.phase")),
        })
        .collect();

    let mut routes = Vec::new();
    for i in 0..route_slots() {
        if get_raw(g, &format!("route.{i}.on")) != 1.0 {
            continue;
        }
        let Some(t) = mod_targets().get(get_raw(g, &format!("route.{i}.target")) as usize) else { continue };
        routes.push(ModRoute {
            src: get_raw(g, &format!("route.{i}.src")) as usize,
            target: t.target.clone(),
            param: t.param.clone(),
            depth: read_value(g, &format!("route.{i}.depth")),
            exp: get_raw(g, &format!("route.{i}.exp")) == 1.0,
        });
    }
    state.modulation.routes = routes;

    for k in &s.coupling_keys {
        state.coupling.insert(k.clone(), read_value(g, &format!("c.{k}")));
    }
    state
}

/// Whether a value could be a genome of this build.
pub fn is_valid_genome(g: &[f64]) -> bool {
    if g.len() != genes().len() {
        return false;
    }
    g.iter().zip(genes()).all(|(v, d)| {
        if !v.is_finite() {
            return false;
        }
        match d.kind {
            GeneKind::Cont => (0.0..=1.0).contains(v),
            _ => v.fract() == 0.0 && *v >= d.min && *v <= d.max,
        }
    })
}
