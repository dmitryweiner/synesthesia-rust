//! The image side's simulation — `src/sim/` of the web app on the CPU, for
//! the terminal-sized grid GRAPHICS.md plans: a Gray–Scott field, the
//! Field variation offsets, the Flow advection, onset injection.
//!
//! One [`Sim::step`] is one web animation frame: refresh the paramfield and
//! the velocity (only if their inputs moved), `speed` reaction substeps, one
//! advection substep. How often it is called is the caller's clock, not the
//! redraw rate (GRAPHICS.md decision 4).

pub mod advect;
pub mod field;
pub mod fields;
pub mod noise;

use std::collections::BTreeMap;

use crate::dsp::rng::{Mulberry32, Rng};
use crate::state::{CardState, Params};

pub use field::Field;
use fields::{half_size, ParamField, Velocity};

/// Nominal frame time `evolve_t` advances by per step; it is an aesthetic
/// drift, not a clock (`EVOLVE_DT` in `engine.ts`).
pub const EVOLVE_DT: f64 = 1.0 / 60.0;

/// The paramfield and the velocity are redrawn at most once in this many
/// steps (5 times a second at 30 Hz). Their inputs drift slowly — `evolve_t`
/// moves by ~1e-4 a step, an LFO over tens of seconds — and redrawing them
/// every step was two thirds of a step's cost for no visible change.
/// `../synesthesia/TODO.md` §1, "or amortize".
pub const FIELD_REFRESH_STEPS: u64 = 6;

/// The largest grid side, whatever the terminal (GRAPHICS.md decision 3).
pub const MAX_GRID_SIDE: usize = 384;

/// The simulation grid for a picture of `pw`×`ph` pixels: twice each side,
/// so a Gray–Scott pattern — whose size is fixed in cells — reads as a
/// pattern and not as a few blobs; the long side capped.
pub fn grid_for_pixels(pw: usize, ph: usize) -> (usize, usize) {
    let (w, h) = (pw.max(2) * 2, ph.max(2) * 2);
    let long = w.max(h);
    if long <= MAX_GRID_SIDE {
        return (w, h);
    }
    let k = MAX_GRID_SIDE as f64 / long as f64;
    (((w as f64 * k).round() as usize).max(3), ((h as f64 * k).round() as usize).max(3))
}

fn param(p: &Params, k: &str, default: f64) -> f64 {
    p.get(k).copied().unwrap_or(default)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reaction {
    pub feed: f64,
    pub kill: f64,
    pub diff_u: f64,
    pub diff_v: f64,
    pub speed: f64,
}

impl Reaction {
    /// pmneila-style defaults: coral/mitosis growth, reliably alive.
    pub const DEFAULT: Self = Self { feed: 0.037, kill: 0.06, diff_u: 0.2097, diff_v: 0.105, speed: 10.0 };

    pub fn from_card(p: &Params) -> Self {
        let d = Self::DEFAULT;
        Self {
            feed: param(p, "feed", d.feed),
            kill: param(p, "kill", d.kill),
            diff_u: param(p, "diffU", d.diff_u),
            diff_v: param(p, "diffV", d.diff_v),
            speed: param(p, "speed", d.speed),
        }
    }

    pub fn substeps(&self) -> usize {
        self.speed.round().max(1.0) as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldVariation {
    pub feed_var_amount: f64,
    pub feed_var_scale: f64,
    pub feed_var_warp: f64,
    pub kill_var_amount: f64,
    pub kill_var_scale: f64,
    pub kill_var_warp: f64,
}

impl FieldVariation {
    /// Uniform feed/kill — what an off card means.
    pub const ZERO: Self = Self {
        feed_var_amount: 0.0,
        feed_var_scale: 1.0,
        feed_var_warp: 0.0,
        kill_var_amount: 0.0,
        kill_var_scale: 1.0,
        kill_var_warp: 0.0,
    };

    pub fn from_card(p: &Params) -> Self {
        let z = Self::ZERO;
        Self {
            feed_var_amount: param(p, "feedVarAmount", z.feed_var_amount),
            feed_var_scale: param(p, "feedVarScale", z.feed_var_scale),
            feed_var_warp: param(p, "feedVarWarp", z.feed_var_warp),
            kill_var_amount: param(p, "killVarAmount", z.kill_var_amount),
            kill_var_scale: param(p, "killVarScale", z.kill_var_scale),
            kill_var_warp: param(p, "killVarWarp", z.kill_var_warp),
        }
    }

    /// Does it perturb feed/kill at all? Scale and warp alone do not.
    pub fn active(&self) -> bool {
        self.feed_var_amount != 0.0 || self.kill_var_amount != 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flow {
    pub curl_strength: f64,
    pub curl_scale: f64,
    pub drift_x: f64,
    pub drift_y: f64,
    pub advect_amount: f64,
    pub evolve_rate: f64,
}

impl Flow {
    /// No flow at all (`curl_scale` 1 avoids a degenerate noise sample).
    pub const ZERO: Self = Self {
        curl_strength: 0.0,
        curl_scale: 1.0,
        drift_x: 0.0,
        drift_y: 0.0,
        advect_amount: 0.0,
        evolve_rate: 0.0,
    };

    pub fn from_card(p: &Params) -> Self {
        let z = Self::ZERO;
        Self {
            curl_strength: param(p, "curlStrength", z.curl_strength),
            curl_scale: param(p, "curlScale", z.curl_scale),
            drift_x: param(p, "driftX", z.drift_x),
            drift_y: param(p, "driftY", z.drift_y),
            advect_amount: param(p, "advectAmount", z.advect_amount),
            evolve_rate: param(p, "evolveRate", z.evolve_rate),
        }
    }

    /// Does advection move anything? With no amount or no velocity it is an
    /// identity copy, and the velocity field is never looked at.
    pub fn advect_active(&self) -> bool {
        self.advect_amount != 0.0 && (self.curl_strength != 0.0 || self.drift_x != 0.0 || self.drift_y != 0.0)
    }
}

/// Everything one step reads. Built from the *effective* card params — after
/// the LFOs and the audio couplings — by the caller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimParams {
    pub reaction: Reaction,
    pub field_variation: FieldVariation,
    pub flow: Flow,
}

impl SimParams {
    /// From card params as they are; an off Field variation or Flow card is
    /// its zero, as in the web app's frame loop.
    pub fn from_cards(cards: &BTreeMap<String, CardState>) -> Self {
        let card = |id: &str| cards.get(id).filter(|c| c.on).map(|c| &c.params);
        Self {
            reaction: cards.get("reaction").map_or(Reaction::DEFAULT, |c| Reaction::from_card(&c.params)),
            field_variation: card("fieldVariation").map_or(FieldVariation::ZERO, FieldVariation::from_card),
            flow: card("flow").map_or(Flow::ZERO, Flow::from_card),
        }
    }
}

/// A field that was never drawn (or was off) is due at once; after that,
/// every [`FIELD_REFRESH_STEPS`].
fn due(last: Option<u64>, now: u64) -> bool {
    last.is_none_or(|at| now - at >= FIELD_REFRESH_STEPS)
}

/// How often the expensive fields were drawn — for tests and the bench.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimStats {
    pub steps: u64,
    pub param_field_draws: u64,
    pub velocity_draws: u64,
}

pub struct Sim {
    field: Field,
    param_field: ParamField,
    velocity: Velocity,
    /// Per-cell feed and kill, and what they were built from.
    feed: Vec<f32>,
    kill: Vec<f32>,
    maps_key: Option<(f64, f64, u64)>,
    scratch: (Vec<f32>, Vec<f32>),
    /// The velocity per cell, in cells per step — rebuilt on each redraw.
    velocity_cells: (Vec<f32>, Vec<f32>),
    evolve_t: f64,
    rng: Mulberry32,
    steps: u64,
    /// The step each field was last refreshed on.
    param_field_at: Option<u64>,
    velocity_at: Option<u64>,
}

impl Sim {
    /// A `w`×`h` grid, seeded from `seed`.
    pub fn new(w: usize, h: usize, seed: u32) -> Self {
        let (fw, fh) = half_size(w, h);
        let mut sim = Self {
            field: Field::blank(w, h),
            param_field: ParamField::new(fw, fh),
            velocity: Velocity::new(fw, fh),
            feed: vec![0.0; w * h],
            kill: vec![0.0; w * h],
            maps_key: None,
            scratch: Default::default(),
            velocity_cells: Default::default(),
            evolve_t: 0.0,
            rng: Mulberry32::new(seed),
            steps: 0,
            param_field_at: None,
            velocity_at: None,
        };
        sim.reseed();
        sim
    }

    pub fn field(&self) -> &Field {
        &self.field
    }

    pub fn stats(&self) -> SimStats {
        SimStats {
            steps: self.steps,
            param_field_draws: self.param_field.draws,
            velocity_draws: self.velocity.draws,
        }
    }

    /// A fresh start: new spots, the pattern wiped.
    pub fn reseed(&mut self) {
        self.field.seed(&mut self.rng);
    }

    /// Fresh growth in a disc (UV, radius in height units, amount 0..1).
    pub fn inject(&mut self, x: f32, y: f32, radius: f32, amount: f32) {
        self.field.inject(x, y, radius, amount);
    }

    /// An onset hit with the `onsetToSeed` coupling at `amount`: new growth at
    /// a random spot away from the edges (`seedOnHit`). Returns where, for the
    /// ripple, or `None` when the coupling is too weak to act.
    pub fn seed_on_hit(&mut self, amount: f64) -> Option<(f32, f32)> {
        if amount < 0.02 {
            return None;
        }
        let x = (0.08 + self.rng.next() * 0.84) as f32;
        let y = (0.08 + self.rng.next() * 0.84) as f32;
        self.inject(x, y, (0.015 + 0.035 * amount) as f32, (0.4 + amount).min(1.0) as f32);
        Some((x, y))
    }

    /// One web animation frame.
    pub fn step(&mut self, p: &SimParams) {
        self.steps += 1;
        self.evolve_t += p.flow.evolve_rate * EVOLVE_DT;
        let aspect = self.field.aspect();

        let varied = p.field_variation.active();
        if varied && due(self.param_field_at, self.steps) {
            self.param_field.update(&p.field_variation, self.evolve_t, aspect);
            self.param_field_at = Some(self.steps);
        }
        if !varied {
            self.param_field_at = None;
        }
        let generation = if varied { self.param_field.draws } else { 0 };
        let key = (p.reaction.feed, p.reaction.kill, generation);
        if self.maps_key != Some(key) {
            self.build_maps(&p.reaction, varied);
            self.maps_key = Some(key);
        }

        let (du, dv) = (p.reaction.diff_u as f32, p.reaction.diff_v as f32);
        for _ in 0..p.reaction.substeps() {
            self.field.react(&self.feed, &self.kill, du, dv);
        }

        if p.flow.advect_active() {
            if due(self.velocity_at, self.steps) {
                if self.velocity.update(&p.flow, self.evolve_t, aspect) {
                    advect::velocity_map(
                        &self.velocity.tex,
                        self.field.w,
                        self.field.h,
                        &mut self.velocity_cells,
                    );
                }
                self.velocity_at = Some(self.steps);
            }
            let amount = p.flow.advect_amount as f32;
            advect::advect(&mut self.field, &self.velocity_cells, amount, &mut self.scratch);
        }
    }

    /// Per-cell feed/kill: the base plus the paramfield's offset, clamped —
    /// what `react.frag` computes per fragment per substep, done once.
    fn build_maps(&mut self, r: &Reaction, varied: bool) {
        let (feed, kill) = (r.feed as f32, r.kill as f32);
        if !varied {
            self.feed.fill(feed.clamp(0.0, 1.0));
            self.kill.fill(kill.clamp(0.0, 1.0));
            return;
        }
        let (w, h) = (self.field.w, self.field.h);
        for y in 0..h {
            let vy = (y as f32 + 0.5) / h as f32;
            for x in 0..w {
                let (df, dk) = self.param_field.tex.sample((x as f32 + 0.5) / w as f32, vy);
                self.feed[y * w + x] = (feed + df).clamp(0.0, 1.0);
                self.kill[y * w + x] = (kill + dk).clamp(0.0, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::presets;
    use rayon::prelude::*;

    const W: usize = 48;
    const H: usize = 32;

    fn variance(v: &[f32]) -> f64 {
        let n = v.len() as f64;
        let mean = v.iter().map(|&x| f64::from(x)).sum::<f64>() / n;
        v.iter().map(|&x| (f64::from(x) - mean).powi(2)).sum::<f64>() / n
    }

    #[test]
    fn the_grid_is_twice_the_picture_with_its_long_side_capped() {
        assert_eq!(grid_for_pixels(120, 60), (240, 120));
        assert_eq!(grid_for_pixels(300, 100), (384, 128));
        assert_eq!(grid_for_pixels(0, 0), (4, 4));
    }

    #[test]
    fn the_same_seed_grows_the_same_field() {
        let p = SimParams::from_cards(&presets()[0].state.visual.cards);
        let (mut a, mut b) = (Sim::new(W, H, 9), Sim::new(W, H, 9));
        for _ in 0..20 {
            a.step(&p);
            b.step(&p);
        }
        assert_eq!(a.field().v(), b.field().v());
        assert_ne!(Sim::new(W, H, 10).field().v(), Sim::new(W, H, 9).field().v());
    }

    #[test]
    fn off_cards_and_zero_amounts_skip_their_fields() {
        let mut p = SimParams::from_cards(&presets()[0].state.visual.cards);
        p.flow.evolve_rate = 0.0;
        p.field_variation =
            FieldVariation { feed_var_amount: 0.0, kill_var_amount: 0.0, ..p.field_variation };
        p.flow.advect_amount = 0.0;
        let mut sim = Sim::new(W, H, 1);
        for _ in 0..5 {
            sim.step(&p);
        }
        assert_eq!(sim.stats().param_field_draws, 0);
        assert_eq!(sim.stats().velocity_draws, 0);

        // Advection with no amount is an identity: same field as no flow at all.
        let mut still = Sim::new(W, H, 1);
        let q = SimParams { flow: Flow::ZERO, ..p };
        for _ in 0..5 {
            still.step(&q);
        }
        assert_eq!(sim.field().v(), still.field().v());
    }

    #[test]
    fn a_frozen_field_is_drawn_once_and_an_evolving_one_at_the_refresh_rate() {
        let mut p = SimParams::from_cards(&presets()[0].state.visual.cards);
        assert!(p.field_variation.active() && p.flow.advect_active());
        p.flow.evolve_rate = 0.0;
        let mut sim = Sim::new(W, H, 1);
        for _ in 0..4 {
            sim.step(&p);
        }
        assert_eq!((sim.stats().param_field_draws, sim.stats().velocity_draws), (1, 1));
        // Evolving, they are redrawn — but only every FIELD_REFRESH_STEPS.
        p.flow.evolve_rate = 0.01;
        for _ in 0..FIELD_REFRESH_STEPS * 3 {
            sim.step(&p);
        }
        assert_eq!((sim.stats().param_field_draws, sim.stats().velocity_draws), (4, 4));
    }

    #[test]
    fn every_preset_stays_bounded_at_the_extremes_of_its_sliders() {
        // The slider corners that push the explicit scheme hardest: the
        // largest diffusion and the fastest flow, with the extremes of feed
        // and kill.
        let corners: Vec<(usize, f64, f64)> =
            (0..presets().len()).flat_map(|i| [(i, 0.01, 0.03), (i, 0.09, 0.075)]).collect();
        corners.par_iter().for_each(|&(i, feed, kill)| {
            let mut p = SimParams::from_cards(&presets()[i].state.visual.cards);
            p.reaction = Reaction { feed, kill, diff_u: 0.4, diff_v: 0.2, speed: 40.0 };
            p.flow.curl_strength = 0.03;
            p.flow.advect_amount = 1.0;
            let mut sim = Sim::new(W, H, 3);
            for _ in 0..50 {
                sim.step(&p);
            }
            let f = sim.field();
            assert!(
                f.u().iter().chain(f.v()).all(|x| x.is_finite() && (0.0..=1.0).contains(x)),
                "{} at feed {feed} kill {kill}",
                presets()[i].name
            );
        });
    }

    #[test]
    fn every_preset_is_still_alive_after_ten_seconds() {
        // 300 steps is ten seconds at the 30 Hz the app will step at.
        let dead: Vec<String> = presets()
            .par_iter()
            .filter_map(|preset| {
                let p = SimParams::from_cards(&preset.state.visual.cards);
                let mut sim = Sim::new(W, H, 1);
                for _ in 0..300 {
                    sim.step(&p);
                }
                let var = variance(sim.field().v());
                (var < 1e-4).then(|| format!("{} (variance of v {var:.2e})", preset.name))
            })
            .collect();
        assert!(dead.is_empty(), "died: {dead:?}");
    }
}
