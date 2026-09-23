//! The directed search of PLAN.md decision 1, ported from
//! `../synesthesia/src/genome/explorer.ts`:
//!
//! * **like** — anchor here and continue along the step that led here, with a
//!   smaller spread;
//! * **dislike** — go back to the anchor and step elsewhere, avoiding the
//!   dimensions that were just rejected, with a larger spread;
//! * **surprise** — jump near a given point and restart the search there;
//! * **undo** — pop the last change.
//!
//! `propose_like` / `propose_dislike` preview a press without committing, so
//! the scout can score several candidates before one is taken.

use std::collections::HashSet;

use crate::dsp::rng::Rng;
use crate::schema::GeneKind;

use super::evolve::{diff_dims, mutate, repair, MutateOptions};
use super::genes::{genes, Genome};

const SIGMA_SHRINK: f64 = 0.8;
const SIGMA_GROW: f64 = 1.35;
const MOMENTUM_WEIGHT: f64 = 0.6;
const MOMENTUM_CAP: f64 = 0.25;
const LIKE_STRUCTURAL: f64 = 0.12;
const DISLIKE_STRUCTURAL: f64 = 0.3;
const SURPRISE_SIGMA: f64 = 0.08;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExplorerAction {
    Load,
    Like,
    Dislike,
    Surprise,
    Undo,
}

pub struct ExplorerOptions {
    pub sigma0: f64,
    pub sigma_min: f64,
    pub sigma_max: f64,
    /// Continuous genes kicked per proposal.
    pub k: usize,
    pub history_depth: usize,
}

impl Default for ExplorerOptions {
    fn default() -> Self {
        Self { sigma0: 0.12, sigma_min: 0.04, sigma_max: 0.35, k: 6, history_depth: 5 }
    }
}

pub struct Explorer {
    pub current: Genome,
    pub anchor: Genome,
    pub sigma: f64,
    pub last_action: ExplorerAction,
    /// Bumped on every committed change, so a stale proposal can be spotted.
    pub version: u64,
    history: Vec<Genome>,
    opts: ExplorerOptions,
}

impl Explorer {
    pub fn new(initial: Genome, opts: ExplorerOptions) -> Self {
        Self {
            current: initial.clone(),
            anchor: initial,
            sigma: opts.sigma0,
            last_action: ExplorerAction::Load,
            version: 0,
            history: Vec::new(),
            opts,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }

    pub fn undo_depth(&self) -> usize {
        self.history.len()
    }

    fn push(&mut self) {
        self.history.push(self.current.clone());
        if self.history.len() > self.opts.history_depth {
            self.history.remove(0);
        }
    }

    /// What a like would set: momentum along the step that led here, and a
    /// smaller spread.
    fn next_like(&self) -> (Option<Genome>, f64) {
        let mut delta = vec![0.0; self.current.len()];
        let mut any = false;
        for i in diff_dims(&self.anchor, &self.current) {
            if genes()[i].kind != GeneKind::Cont {
                continue;
            }
            delta[i] = (self.current[i] - self.anchor[i]).clamp(-MOMENTUM_CAP, MOMENTUM_CAP);
            any = true;
        }
        (any.then_some(delta), (self.sigma * SIGMA_SHRINK).max(self.opts.sigma_min))
    }

    /// What a dislike would set: the rejected dimensions, and a larger spread.
    fn next_dislike(&self) -> (HashSet<usize>, f64) {
        let rejected = diff_dims(&self.anchor, &self.current)
            .into_iter()
            .filter(|i| genes()[*i].kind == GeneKind::Cont)
            .collect();
        (rejected, (self.sigma * SIGMA_GROW).min(self.opts.sigma_max))
    }

    /// A candidate for "more of this" — leaves the explorer untouched.
    pub fn propose_like(&self, rng: &mut dyn Rng) -> Genome {
        let (momentum, sigma) = self.next_like();
        let opts = MutateOptions {
            sigma,
            k: self.opts.k,
            structural_prob: LIKE_STRUCTURAL,
            momentum: momentum.as_deref(),
            momentum_weight: MOMENTUM_WEIGHT,
            avoid: None,
        };
        repair(&mutate(&self.current, rng, &opts), rng)
    }

    /// A candidate for "not this" — leaves the explorer untouched.
    pub fn propose_dislike(&self, rng: &mut dyn Rng) -> Genome {
        let (rejected, sigma) = self.next_dislike();
        let opts = MutateOptions {
            sigma,
            k: self.opts.k,
            structural_prob: DISLIKE_STRUCTURAL,
            avoid: Some(&rejected),
            ..MutateOptions::default()
        };
        repair(&mutate(&self.anchor, rng, &opts), rng)
    }

    /// "Keep going this way." Commits `proposal` when the scout picked one.
    pub fn like(&mut self, proposal: Option<Genome>, rng: &mut dyn Rng) -> &Genome {
        let next = proposal.unwrap_or_else(|| self.propose_like(rng));
        let (_, sigma) = self.next_like();
        self.push();
        self.anchor = self.current.clone();
        self.sigma = sigma;
        self.current = next;
        self.last_action = ExplorerAction::Like;
        self.version += 1;
        &self.current
    }

    /// "Go back and try elsewhere."
    pub fn dislike(&mut self, proposal: Option<Genome>, rng: &mut dyn Rng) -> &Genome {
        let next = proposal.unwrap_or_else(|| self.propose_dislike(rng));
        let (_, sigma) = self.next_dislike();
        self.sigma = sigma;
        self.push();
        self.current = next;
        self.last_action = ExplorerAction::Dislike;
        self.version += 1;
        &self.current
    }

    /// Jumps near `target` — a random preset — and restarts the search there.
    pub fn surprise(&mut self, target: &Genome, rng: &mut dyn Rng) -> &Genome {
        self.push();
        let opts =
            MutateOptions { sigma: SURPRISE_SIGMA, k: 4, structural_prob: 0.0, ..MutateOptions::default() };
        let next = mutate(target, rng, &opts);
        self.current = repair(&next, rng);
        self.anchor = self.current.clone();
        self.sigma = self.opts.sigma0;
        self.last_action = ExplorerAction::Surprise;
        self.version += 1;
        &self.current
    }

    /// Loads a point: fresh start, history cleared.
    pub fn load(&mut self, g: Genome) {
        self.current = g.clone();
        self.anchor = g;
        self.sigma = self.opts.sigma0;
        self.history.clear();
        self.last_action = ExplorerAction::Load;
        self.version += 1;
    }

    /// Reverts the last change; `None` when there is nothing to undo.
    pub fn undo(&mut self) -> Option<&Genome> {
        let prev = self.history.pop()?;
        self.current = prev.clone();
        self.anchor = prev;
        self.last_action = ExplorerAction::Undo;
        self.version += 1;
        Some(&self.current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::rng::Mulberry32;
    use crate::genome::codec::{encode_genome, is_valid_genome};
    use crate::state::presets;

    fn start() -> (Explorer, Mulberry32) {
        let g = encode_genome(&presets()[0].state);
        (Explorer::new(g, ExplorerOptions::default()), Mulberry32::new(42))
    }

    #[test]
    fn a_like_moves_and_narrows_the_search() {
        let (mut ex, mut rng) = start();
        let before = ex.current.clone();
        let sigma = ex.sigma;
        ex.like(None, &mut rng);
        assert_ne!(ex.current, before, "a proposal that changes nothing is not a proposal");
        assert_eq!(ex.anchor, before, "the liked point becomes the anchor");
        assert!(ex.sigma < sigma, "the spread narrows");
        assert!(is_valid_genome(&ex.current));
    }

    #[test]
    fn a_dislike_widens_the_search_and_avoids_what_was_rejected() {
        let (mut ex, mut rng) = start();
        ex.like(None, &mut rng);
        let rejected: Vec<usize> = diff_dims(&ex.anchor, &ex.current)
            .into_iter()
            .filter(|i| genes()[*i].kind == GeneKind::Cont)
            .collect();
        let anchor = ex.anchor.clone();
        let sigma = ex.sigma;
        ex.dislike(None, &mut rng);
        assert!(ex.sigma > sigma, "the spread widens");
        for i in rejected {
            assert_eq!(ex.current[i], anchor[i], "gene {} was rejected, it must stay put", genes()[i].id);
        }
    }

    #[test]
    fn undo_walks_back_five_steps_and_then_stops() {
        let (mut ex, mut rng) = start();
        let first = ex.current.clone();
        for _ in 0..7 {
            ex.like(None, &mut rng);
        }
        let mut steps = 0;
        while ex.undo().is_some() {
            steps += 1;
        }
        assert_eq!(steps, 5, "history depth is five");
        assert_ne!(ex.current, first, "the oldest steps fell off the end");
    }

    #[test]
    fn surprise_lands_near_the_target_and_resets_the_spread() {
        let (mut ex, mut rng) = start();
        ex.like(None, &mut rng);
        let target = encode_genome(&presets()[7].state);
        ex.surprise(&target, &mut rng);
        assert_eq!(ex.anchor, ex.current);
        assert_eq!(ex.sigma, ExplorerOptions::default().sigma0);
        let moved = diff_dims(&target, &ex.current).len();
        assert!(moved < target.len() / 2, "surprise should land near the target, moved {moved} genes");
    }

    #[test]
    fn a_committed_proposal_is_what_the_scout_chose() {
        let (mut ex, mut rng) = start();
        let chosen = ex.propose_like(&mut rng);
        ex.like(Some(chosen.clone()), &mut rng);
        assert_eq!(ex.current, chosen);
        assert_eq!(ex.version, 1);
    }
}
