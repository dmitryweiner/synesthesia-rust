//! Fractality-guided proposals (PLAN.md decision 8).
//!
//! While the user listens to a settled point, the scout renders that point and
//! a few 👍 and 👎 candidates offline, scores each with
//! [`analyze_sound`](crate::analysis::fractal::analyze_sound) and, on a press,
//! hands out the best one for that direction. A candidate much quieter than
//! the parent is penalized — the search must not sink into silence one step at
//! a time.
//!
//! In the browser this had to run on a 24 s / 8 kHz surrogate to be affordable
//! (ρ = 0.73 against a full render). Here a render is ~20× realtime and the
//! candidates go out across the little cores, so they are scored at full
//! quality instead.

use rayon::prelude::*;

use crate::analysis::fractal::{analyze_sound, SoundAnalysis};
use crate::engine::render_offline;
use crate::state::AppState;

use super::codec::decode_genome;
use super::genes::Genome;

/// Quieter than the parent by up to this much: no penalty.
const FREE_DROP_DB: f64 = 6.0;
const PENALTY_PER_DB: f64 = 1.0 / 24.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScoutKind {
    Like,
    Dislike,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub kind: ScoutKind,
    pub genome: Genome,
    pub analysis: SoundAnalysis,
    /// Score after the loudness penalty — what the pick is made on.
    pub adjusted: f64,
}

#[derive(Clone, Debug)]
pub struct ScoutResult {
    /// The explorer version these candidates were prepared for. Anything that
    /// commits a change makes them stale.
    pub version: u64,
    pub parent: SoundAnalysis,
    pub candidates: Vec<Candidate>,
    /// Wall time the whole job took, in seconds.
    pub seconds: f64,
}

impl ScoutResult {
    /// The best candidate for a direction, if one was scored.
    pub fn best(&self, kind: ScoutKind) -> Option<&Candidate> {
        self.candidates.iter().filter(|c| c.kind == kind).max_by(|a, b| a.adjusted.total_cmp(&b.adjusted))
    }

    pub fn ready(&self, kind: ScoutKind) -> usize {
        self.candidates.iter().filter(|c| c.kind == kind).count()
    }
}

/// Fractal score minus a penalty for being much quieter than the parent;
/// silence ranks last.
pub fn adjusted_score(a: &SoundAnalysis, parent: &SoundAnalysis) -> f64 {
    if a.silent {
        return -1.0;
    }
    if parent.silent {
        return a.score;
    }
    let drop = parent.loudness - a.loudness;
    a.score - (drop - FREE_DROP_DB).max(0.0) * PENALTY_PER_DB
}

/// What a render costs the scout.
#[derive(Clone, Copy, Debug)]
pub struct ScoutSettings {
    pub seconds: f64,
    pub sample_rate: f64,
    /// Master gain of the point being listened to — a candidate is judged at
    /// the level it would actually play at.
    pub master_gain: f64,
    pub seed: u32,
}

impl Default for ScoutSettings {
    fn default() -> Self {
        Self { seconds: 30.0, sample_rate: 22050.0, master_gain: 0.75, seed: 1 }
    }
}

fn state_of(g: &Genome, master_gain: f64) -> AppState {
    let mut s = decode_genome(g);
    s.audio.master_gain = master_gain;
    s
}

fn score(g: &Genome, set: &ScoutSettings) -> SoundAnalysis {
    let samples = render_offline(&state_of(g, set.master_gain), set.seconds, set.sample_rate, set.seed);
    analyze_sound(&samples, set.sample_rate)
}

/// Renders and scores the parent and every candidate, in parallel.
pub fn run(
    version: u64,
    parent: &Genome,
    likes: &[Genome],
    dislikes: &[Genome],
    set: ScoutSettings,
) -> ScoutResult {
    let started = std::time::Instant::now();
    let jobs: Vec<(ScoutKind, &Genome)> = likes
        .iter()
        .map(|g| (ScoutKind::Like, g))
        .chain(dislikes.iter().map(|g| (ScoutKind::Dislike, g)))
        .collect();

    let (parent_analysis, scored): (SoundAnalysis, Vec<(ScoutKind, SoundAnalysis)>) = rayon::join(
        || score(parent, &set),
        || jobs.par_iter().map(|(kind, g)| (*kind, score(g, &set))).collect(),
    );

    let candidates = jobs
        .iter()
        .zip(scored)
        .map(|((kind, g), (_, analysis))| Candidate {
            kind: *kind,
            genome: (*g).clone(),
            adjusted: adjusted_score(&analysis, &parent_analysis),
            analysis,
        })
        .collect();

    ScoutResult { version, parent: parent_analysis, candidates, seconds: started.elapsed().as_secs_f64() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::rng::Mulberry32;
    use crate::genome::codec::encode_genome;
    use crate::genome::explorer::{Explorer, ExplorerOptions};
    use crate::state::presets;

    fn tiny() -> ScoutSettings {
        ScoutSettings { seconds: 2.0, sample_rate: 8000.0, master_gain: 0.75, seed: 5 }
    }

    #[test]
    fn it_scores_every_candidate_and_picks_the_best_of_each_direction() {
        let mut rng = Mulberry32::new(17);
        let ex = Explorer::new(encode_genome(&presets()[0].state), ExplorerOptions::default());
        let likes: Vec<Genome> = (0..2).map(|_| ex.propose_like(&mut rng)).collect();
        let dislikes: Vec<Genome> = (0..2).map(|_| ex.propose_dislike(&mut rng)).collect();

        let result = run(ex.version, &ex.current, &likes, &dislikes, tiny());
        assert_eq!(result.ready(ScoutKind::Like), 2);
        assert_eq!(result.ready(ScoutKind::Dislike), 2);
        let best = result.best(ScoutKind::Like).expect("a pick");
        for c in result.candidates.iter().filter(|c| c.kind == ScoutKind::Like) {
            assert!(best.adjusted >= c.adjusted);
        }
        assert!(!result.parent.silent, "the parent preset should not be silent");
    }

    #[test]
    fn a_much_quieter_candidate_is_penalized_and_silence_ranks_last() {
        let loud = SoundAnalysis {
            silent: false,
            loudness: -20.0,
            env_beta: 1.0,
            centroid_beta: 1.0,
            env_higuchi: 1.5,
            box_dim: 1.6,
            score: 0.9,
        };
        let quiet = SoundAnalysis { loudness: -50.0, ..loud };
        let silent = SoundAnalysis { silent: true, score: 0.0, ..loud };
        assert_eq!(adjusted_score(&loud, &loud), 0.9);
        let penalty = 0.9 - adjusted_score(&quiet, &loud);
        assert!((penalty - (30.0 - FREE_DROP_DB) * PENALTY_PER_DB).abs() < 1e-12, "{penalty}");
        assert!(adjusted_score(&silent, &loud) < adjusted_score(&quiet, &loud));
    }

    #[test]
    fn a_stale_result_is_recognisable_by_its_version() {
        let mut rng = Mulberry32::new(2);
        let mut ex = Explorer::new(encode_genome(&presets()[1].state), ExplorerOptions::default());
        let likes = vec![ex.propose_like(&mut rng)];
        let result = run(ex.version, &ex.current, &likes, &[], tiny());
        assert_eq!(result.version, ex.version);
        ex.like(None, &mut rng);
        assert_ne!(result.version, ex.version, "a committed change makes the candidates stale");
    }
}
