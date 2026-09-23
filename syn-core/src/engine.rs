//! The sound engine: generators → gate → mix → FX → master.
//!
//! Pure and blocking-free — it owns no device and no thread. `syn-audio` calls
//! [`Engine::render`] from the audio callback; the scout calls
//! [`render_offline`] from a worker thread.

use crate::dsp::gate::{apply_gate, gate_is_silent};
use crate::dsp::generator::FormulaGenerator;
use crate::dsp::rng::{Mulberry32, Rng};
use crate::features::{Analyser, AudioFeatures, FeatureTracker, OnsetDetector, FPS};
use crate::fx::FxChain;
use crate::modmatrix::{effective_param, lfo_value, LfoDef, ModRoute};
use crate::schema::{formula_ranges, schema};
use crate::state::{AppState, FxState};
use crate::{FormulaId, BLOCK};

/// Generator on/off smoothing, seconds (web: `GAIN_SMOOTH`).
const GAIN_SMOOTH: f64 = 0.02;
/// Master gain smoothing, seconds (web: `PARAM_SMOOTH`).
const MASTER_SMOOTH: f64 = 0.05;
/// Bands in the spectrum the engine publishes for the UI to draw.
pub const SPECTRUM_BANDS: usize = 64;

struct Slot {
    id: &'static str,
    gen: FormulaGenerator,
    enabled: bool,
    fade: f32,
    /// Smoothed mixer gain. The browser applies the formula's `gain` twice —
    /// once inside the worklet and once on the node feeding the mix bus — so
    /// the port does the same; halving it here would make every preset quiet.
    gain: f64,
}

pub struct Engine {
    sr: f64,
    slots: Vec<Slot>,
    fx: FxChain,
    state: AppState,
    master: f64,
    /// Absolute time in seconds — the one clock the LFOs and the visualizers
    /// read (PLAN.md decision 7).
    t: f64,
    scratch: Vec<f32>,

    // The feature bus (PLAN.md decision 5): the same numbers the browser's
    // analyser produced, published for whatever draws them.
    analyser: Analyser,
    tracker: FeatureTracker,
    onsets: OnsetDetector,
    features: AudioFeatures,
    spectrum: [u8; SPECTRUM_BANDS],
    hits: u64,
}

impl Engine {
    pub fn new(sr: f64, state: &AppState, seed: u32) -> Self {
        let mut e = Engine {
            sr,
            slots: Vec::new(),
            fx: FxChain::new(sr),
            state: state.clone(),
            master: state.audio.master_gain,
            t: 0.0,
            scratch: vec![0.0; BLOCK],
            analyser: Analyser::new(sr),
            tracker: FeatureTracker::new(sr),
            onsets: OnsetDetector::default(),
            features: AudioFeatures::default(),
            spectrum: [0; SPECTRUM_BANDS],
            hits: 0,
        };
        let mut seed_n = seed;
        for id in &schema().formula_ids {
            let fid = FormulaId::parse(id).expect("schema id is a formula");
            let snap = state.audio.formulas.get(id);
            let params = snap.map(|s| s.params.clone()).unwrap_or_default();
            let enabled = snap.is_some_and(|s| s.enabled);
            seed_n = seed_n.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let mut gen = FormulaGenerator::new(
                fid,
                sr,
                params,
                Box::new(Mulberry32::new(seed_n)) as Box<dyn Rng + Send>,
            );
            let routes = routes_for(state, id);
            gen.set_mod(&state.modulation.lfos, &routes, &formula_ranges(id));
            e.slots.push(Slot {
                id: fid.as_str(),
                gen,
                enabled,
                fade: if enabled { 1.0 } else { 0.0 },
                gain: if enabled { snap.map_or(0.0, gain_of) } else { 0.0 },
            });
        }
        e.fx.reset(&state.audio.fx);
        e
    }

    pub fn sample_rate(&self) -> f64 {
        self.sr
    }

    /// Seconds since the engine started — the LFO clock.
    pub fn time(&self) -> f64 {
        self.t
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// New target point: parameters glide, nothing is rebuilt. Use it for the
    /// genome morph, where every value moves continuously.
    pub fn set_state(&mut self, state: &AppState) {
        self.state = state.clone();
        for slot in &mut self.slots {
            if let Some(snap) = self.state.audio.formulas.get(slot.id) {
                slot.gen.set(&snap.params);
                slot.enabled = snap.enabled;
            }
            let routes = routes_for(&self.state, slot.id);
            slot.gen.set_mod(&self.state.modulation.lfos, &routes, &formula_ranges(slot.id));
        }
    }

    /// Hard switch to another point: drops every tail so the old reverb and
    /// delay cannot bleed into the new sound. The caller ducks around it.
    pub fn switch_to(&mut self, state: &AppState) {
        self.set_state(state);
        self.fx.reset(&state.audio.fx);
        self.master = state.audio.master_gain;
        for slot in &mut self.slots {
            slot.gen.reset();
            slot.fade = if slot.enabled { 1.0 } else { 0.0 };
            slot.gain =
                if slot.enabled { self.state.audio.formulas.get(slot.id).map_or(0.0, gain_of) } else { 0.0 };
        }
    }

    /// Renders one block (mono). `out.len()` should be [`crate::BLOCK`].
    pub fn render(&mut self, out: &mut [f32]) {
        let n = out.len();
        if self.scratch.len() < n {
            self.scratch.resize(n, 0.0);
        }
        out.fill(0.0);

        let dt = n as f64 / self.sr;
        let gain_coef = 1.0 - (-dt / GAIN_SMOOTH).exp();
        let master_coef = 1.0 - (-dt / MASTER_SMOOTH).exp();

        for slot in &mut self.slots {
            let target =
                if slot.enabled { self.state.audio.formulas.get(slot.id).map_or(0.0, gain_of) } else { 0.0 };
            slot.gain += gain_coef * (target - slot.gain);

            if gate_is_silent(slot.fade, slot.enabled) {
                continue;
            }
            let buf = &mut self.scratch[..n];
            slot.gen.fill(buf);
            slot.fade = apply_gate(buf, slot.fade, slot.enabled);
            let g = slot.gain as f32;
            for (o, s) in out.iter_mut().zip(buf.iter()) {
                *o += *s * g;
            }
        }

        // FX parameters follow the LFOs at block rate, as they do in the
        // browser (there at ~40 Hz from a timer; here exactly per block).
        let eff = modulate_fx(
            &self.state.audio.fx,
            &self.state.modulation.routes,
            &self.state.modulation.lfos,
            self.t,
        );
        self.fx.set(&eff);
        self.fx.process(out);

        let master_target = self.state.audio.master_gain;
        self.master += master_coef * (master_target - self.master);
        let master = self.master as f32;
        for s in out.iter_mut() {
            *s *= master;
        }

        self.t += dt;

        if self.analyser.push(out) {
            self.features = self.tracker.update(self.analyser.rms, &self.analyser.bytes, 1.0 / FPS);
            self.update_spectrum();
            if self.onsets.update(self.features.onset, self.t) {
                self.hits += 1;
            }
        }
    }

    pub fn limiter_reduction_db(&self) -> f64 {
        self.fx.limiter_reduction_db()
    }

    /// The latest features — loudness, swell, brightness, bands, onset.
    pub fn features(&self) -> AudioFeatures {
        self.features
    }

    /// A log-spaced summary of the spectrum, 0..255 per band.
    pub fn spectrum(&self) -> &[u8; SPECTRUM_BANDS] {
        &self.spectrum
    }

    /// Onset hits since the engine started.
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Folds the analyser's linear bins into log-spaced bands, the way an ear
    /// (and a spectrum display) groups them.
    fn update_spectrum(&mut self) {
        let bins = &self.analyser.bytes;
        let hz_per_bin = self.sr / 2.0 / bins.len() as f64;
        let (lo_hz, hi_hz) = (40.0, 16000.0f64.min(self.sr / 2.0 * 0.95));
        for (b, out) in self.spectrum.iter_mut().enumerate() {
            let lo = lo_hz * (hi_hz / lo_hz).powf(b as f64 / SPECTRUM_BANDS as f64);
            let hi = lo_hz * (hi_hz / lo_hz).powf((b + 1) as f64 / SPECTRUM_BANDS as f64);
            let i0 = ((lo / hz_per_bin) as usize).min(bins.len() - 1);
            let i1 = (((hi / hz_per_bin).ceil() as usize).max(i0 + 1)).min(bins.len());
            *out = bins[i0..i1].iter().copied().max().unwrap_or(0);
        }
    }
}

fn gain_of(s: &crate::state::FormulaSnapshot) -> f64 {
    s.params.get("gain").copied().unwrap_or(0.0)
}

fn routes_for(state: &AppState, target: &str) -> Vec<ModRoute> {
    state.modulation.routes.iter().filter(|r| r.target == target).cloned().collect()
}

/// Effective FX state at time `t`: the base with every allowlisted field that
/// has a route laid over it.
pub fn modulate_fx(base: &FxState, routes: &[ModRoute], lfos: &[LfoDef], t: f64) -> FxState {
    let ranges = &schema().fx_param_ranges;
    let mut eff = base.clone();
    for r in routes.iter().filter(|r| r.target == "fx") {
        let (Some(lfo), Some(range), Some(v)) = (lfos.get(r.src), ranges.get(&r.param), base.get(&r.param))
        else {
            continue;
        };
        eff.set(&r.param, effective_param(v, lfo_value(lfo, t), r.depth, *range, r.exp));
    }
    eff
}

/// Renders `seconds` of a point offline, faster than realtime. Used by the
/// scout and by `synesthesia render`.
pub fn render_offline(state: &AppState, seconds: f64, sr: f64, seed: u32) -> Vec<f32> {
    let n = (seconds * sr) as usize;
    let mut engine = Engine::new(sr, state, seed);
    let mut out = vec![0.0f32; n.div_ceil(BLOCK) * BLOCK];
    for block in out.chunks_mut(BLOCK) {
        engine.render(block);
    }
    out.truncate(n);
    out
}

#[cfg(test)]
mod tests {
    use crate::state::presets;

    use super::*;

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len().max(1) as f64).sqrt()
    }

    #[test]
    fn every_preset_makes_audible_sound_and_stays_in_range() {
        for p in presets() {
            let x = render_offline(&p.state, 2.0, 22050.0, 1);
            let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            let level = rms(&x);
            assert!(x.iter().all(|v| v.is_finite()), "{}: not finite", p.name);
            assert!(peak <= 1.5, "{}: peak {peak}", p.name);
            assert!(level > 1e-4, "{}: silent ({level:e})", p.name);
        }
    }

    #[test]
    fn a_point_with_nothing_enabled_is_silent() {
        let state = AppState::new();
        let x = render_offline(&state, 0.5, 22050.0, 1);
        assert!(x.iter().all(|v| v.abs() < 1e-9));
    }

    #[test]
    fn switching_points_does_not_click() {
        let sr = 48000.0;
        let a = &presets()[0].state;
        let b = &presets()[3].state;
        let mut engine = Engine::new(sr, a, 7);
        let mut buf = vec![0.0f32; BLOCK];
        for _ in 0..200 {
            engine.render(&mut buf);
        }
        let before = buf[BLOCK - 1];
        engine.switch_to(b);
        let mut after = vec![0.0f32; BLOCK];
        engine.render(&mut after);
        // A hard switch drops tails, so the first sample after it must be near
        // silence rather than a step away from the last one.
        assert!(after[0].abs() < 0.2, "step of {} → {}", before, after[0]);
    }

    #[test]
    fn it_publishes_features_while_it_plays() {
        let sr = 48000.0;
        let mut engine = Engine::new(sr, &presets()[10].state, 3);
        let mut buf = vec![0.0f32; BLOCK];
        for _ in 0..(sr as usize * 4 / BLOCK) {
            engine.render(&mut buf);
        }
        let f = engine.features();
        assert!(f.loudness > 0.0 && f.loudness <= 1.0, "loudness {}", f.loudness);
        assert!(f.brightness > 0.0, "brightness {}", f.brightness);
        assert!(engine.spectrum().iter().any(|b| *b > 0), "empty spectrum");
        assert!(engine.hits() > 0, "Bell spots struck no onset in 4 s");
    }

    #[test]
    fn the_lfo_clock_advances_with_the_audio() {
        let mut engine = Engine::new(48000.0, &presets()[0].state, 1);
        let mut buf = vec![0.0f32; BLOCK];
        for _ in 0..375 {
            engine.render(&mut buf);
        }
        assert!((engine.time() - 1.0).abs() < 1e-9, "{}", engine.time());
    }
}
