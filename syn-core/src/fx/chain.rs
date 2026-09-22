//! The chain itself: topology switches, per-block parameter smoothing and the
//! per-sample loop.
//!
//! Every numeric parameter is smoothed with the web app's time constant
//! (30 ms). Values arrive at control rate both from LFO routes and from genome
//! morphs, and stepping them would zipper — the phaser's feedback loop rings on
//! it audibly.

use crate::schema::{self, vowel_formants};
use crate::state::FxState;

use super::biquad::{Biquad, BiquadType, Coeffs};
use super::delayline::DelayLine;
use super::limiter::Limiter;
use super::reverb::Fdn;

/// Smoothing time constant for FX parameters, seconds (web: `FX_SMOOTH_TC`).
const SMOOTH_TC: f64 = 0.03;
const MAX_PHASER_STAGES: usize = 8;
const FORMANT_BANDS: usize = 3;
/// All-pass centre sweep, kept strictly positive: [200, 200 + 3600·depth].
const PHASER_F_LO: f64 = 200.0;

#[derive(Clone, Copy, Debug, Default)]
struct Smooth {
    v: f64,
}

impl Smooth {
    fn jump(&mut self, target: f64) {
        self.v = target;
    }

    #[inline]
    fn glide(&mut self, target: f64, coef: f64) -> f64 {
        self.v += coef * (target - self.v);
        self.v
    }
}

/// Everything the chain smooths, in one place so `process` reads like the
/// signal path rather than like bookkeeping.
#[derive(Clone, Copy, Debug, Default)]
struct Smoothed {
    filter_freq: Smooth,
    filter_q: Smooth,
    filter_gain: Smooth,
    filter_vowel: Smooth,
    comb_fb: Smooth,
    chorus_rate: Smooth,
    chorus_depth: Smooth,
    chorus_mix: Smooth,
    chorus_fb: Smooth,
    phaser_rate: Smooth,
    phaser_depth: Smooth,
    phaser_fb: Smooth,
    phaser_mix: Smooth,
    delay_time: Smooth,
    delay_fb: Smooth,
    delay_mix: Smooth,
    reverb_mix: Smooth,
    limiter_thr: Smooth,
    limiter_rel: Smooth,
}

pub struct FxChain {
    sr: f64,
    fx: FxState,
    s: Smoothed,

    filter: Biquad,
    formants: [Biquad; FORMANT_BANDS],
    formant_amp: [f64; FORMANT_BANDS],
    comb: DelayLine,

    chorus: DelayLine,
    chorus_phase: f64,

    phaser: [Biquad; MAX_PHASER_STAGES],
    phaser_phase: f64,
    phaser_fb_state: f64,

    delay: DelayLine,
    reverb: Fdn,
    limiter: Limiter,
    last_reverb_decay: f64,
}

impl FxChain {
    pub fn new(sr: f64) -> Self {
        let mut chain = Self {
            sr,
            fx: FxState::default(),
            s: Smoothed::default(),
            filter: Biquad::new(),
            formants: [Biquad::new(); FORMANT_BANDS],
            formant_amp: [0.0; FORMANT_BANDS],
            comb: DelayLine::new((sr * 0.05) as usize + 4),
            chorus: DelayLine::new((sr * 0.2) as usize + 4),
            chorus_phase: 0.0,
            phaser: [Biquad::new(); MAX_PHASER_STAGES],
            phaser_phase: 0.0,
            phaser_fb_state: 0.0,
            delay: DelayLine::new((sr * 3.0) as usize + 4),
            reverb: Fdn::new(sr),
            limiter: Limiter::new(sr),
            last_reverb_decay: f64::NAN,
        };
        let fx = FxState::default();
        chain.reset(&fx);
        chain
    }

    /// New target values; the chain glides to them.
    pub fn set(&mut self, fx: &FxState) {
        self.fx = fx.clone();
    }

    /// Hard switch (a new point): jump every parameter and drop every tail, so
    /// the old reverb and delay cannot bleed into the new sound.
    pub fn reset(&mut self, fx: &FxState) {
        self.fx = fx.clone();
        self.s.filter_freq.jump(fx.filter_freq);
        self.s.filter_q.jump(fx.filter_q);
        self.s.filter_gain.jump(fx.filter_gain);
        self.s.filter_vowel.jump(fx.filter_vowel);
        self.s.comb_fb.jump(fx.filter_comb_fb);
        self.s.chorus_rate.jump(fx.chorus_rate);
        self.s.chorus_depth.jump(fx.chorus_depth);
        self.s.chorus_mix.jump(fx.chorus_mix);
        self.s.chorus_fb.jump(fx.chorus_fb);
        self.s.phaser_rate.jump(fx.phaser_rate);
        self.s.phaser_depth.jump(fx.phaser_depth);
        self.s.phaser_fb.jump(fx.phaser_fb);
        self.s.phaser_mix.jump(fx.phaser_mix);
        self.s.delay_time.jump(fx.delay_time);
        self.s.delay_fb.jump(fx.delay_fb);
        self.s.delay_mix.jump(fx.delay_mix);
        self.s.reverb_mix.jump(fx.reverb_mix);
        self.s.limiter_thr.jump(fx.limiter_thr);
        self.s.limiter_rel.jump(fx.limiter_rel);

        self.filter.reset();
        for f in &mut self.formants {
            f.reset();
        }
        for f in &mut self.phaser {
            f.reset();
        }
        self.comb.clear();
        self.chorus.clear();
        self.delay.clear();
        self.reverb.clear();
        self.limiter.clear();
        self.chorus_phase = 0.0;
        self.phaser_phase = 0.0;
        self.phaser_fb_state = 0.0;
        self.last_reverb_decay = f64::NAN;
    }

    pub fn limiter_reduction_db(&self) -> f64 {
        self.limiter.reduction_db()
    }

    /// Runs one block in place.
    pub fn process(&mut self, buf: &mut [f32]) {
        let sr = self.sr;
        let dt = buf.len() as f64 / sr;
        let coef = 1.0 - (-dt / SMOOTH_TC).exp();
        let fx = self.fx.clone();

        // --- block-rate parameters -----------------------------------------
        let filter_freq = self.s.filter_freq.glide(fx.filter_freq, coef);
        let filter_q = self.s.filter_q.glide(fx.filter_q, coef);
        let filter_gain = self.s.filter_gain.glide(fx.filter_gain, coef);
        let filter_vowel = self.s.filter_vowel.glide(fx.filter_vowel, coef);
        let comb_fb = self.s.comb_fb.glide(fx.filter_comb_fb, coef).clamp(0.0, 0.95);
        let chorus_rate = self.s.chorus_rate.glide(fx.chorus_rate, coef);
        let chorus_depth = self.s.chorus_depth.glide(fx.chorus_depth, coef);
        let chorus_mix = self.s.chorus_mix.glide(fx.chorus_mix, coef).clamp(0.0, 1.0);
        let chorus_fb = self.s.chorus_fb.glide(fx.chorus_fb, coef).clamp(0.0, 0.95);
        let phaser_rate = self.s.phaser_rate.glide(fx.phaser_rate, coef);
        let phaser_depth = self.s.phaser_depth.glide(fx.phaser_depth, coef);
        let phaser_fb = self.s.phaser_fb.glide(fx.phaser_fb, coef).clamp(0.0, 0.9);
        let phaser_mix = self.s.phaser_mix.glide(fx.phaser_mix, coef).clamp(0.0, 1.0);
        let delay_time = self.s.delay_time.glide(fx.delay_time, coef).clamp(0.001, 3.0);
        let delay_fb = self.s.delay_fb.glide(fx.delay_fb, coef).clamp(0.0, 0.95);
        let delay_mix = self.s.delay_mix.glide(fx.delay_mix, coef).clamp(0.0, 1.0);
        let reverb_mix = self.s.reverb_mix.glide(fx.reverb_mix, coef).clamp(0.0, 1.0);
        let limiter_thr = self.s.limiter_thr.glide(fx.limiter_thr, coef);
        let limiter_rel = self.s.limiter_rel.glide(fx.limiter_rel, coef);

        let mode = filter_mode(&fx.filter_type);
        match mode {
            FilterMode::Biquad => {
                let kind = BiquadType::parse(&fx.filter_type).unwrap_or(BiquadType::Lowpass);
                self.filter.set(kind, filter_freq.clamp(20.0, 20000.0), filter_q, filter_gain, sr);
            }
            FilterMode::Formant => {
                let v = vowel_formants(filter_vowel);
                let scale = filter_freq.clamp(200.0, 4000.0) / 1000.0;
                let q = (filter_q * 6.0).clamp(4.0, 28.0);
                for i in 0..FORMANT_BANDS {
                    let f = (v.f[i] * scale).clamp(50.0, 8000.0);
                    self.formants[i].set(BiquadType::Bandpass, f, q, 0.0, sr);
                    self.formant_amp[i] = v.a[i];
                }
            }
            FilterMode::Comb => {}
        }

        let comb_delay = (sr / filter_freq.clamp(20.0, 2000.0)).clamp(2.0, self.comb.capacity() as f64 - 2.0);
        let chorus_base_ms = if fx.chorus_mode == "flanger" { 2.0 } else { 12.0 };
        let chorus_inc = std::f64::consts::TAU * chorus_rate / sr;
        let phaser_inc = std::f64::consts::TAU * phaser_rate / sr;
        let stages = (fx.phaser_stages.floor() as usize).clamp(1, MAX_PHASER_STAGES);
        let p_hi = PHASER_F_LO + 3600.0 * phaser_depth;
        let p_centre = 0.5 * (PHASER_F_LO + p_hi);
        let p_half = 0.5 * (p_hi - PHASER_F_LO);
        let delay_samples = delay_time * sr;

        let decay_moved = !matches!(
            (fx.reverb_decay - self.last_reverb_decay).abs().partial_cmp(&0.05),
            Some(std::cmp::Ordering::Less)
        );
        if fx.reverb_on && decay_moved {
            self.last_reverb_decay = fx.reverb_decay;
            self.reverb.set_decay(fx.reverb_decay);
        }
        self.limiter.set(limiter_thr, limiter_rel);

        // --- per sample ------------------------------------------------------
        for sample in buf.iter_mut() {
            let mut x = f64::from(*sample);

            if fx.filter_on {
                x = match mode {
                    FilterMode::Biquad => self.filter.tick(x),
                    FilterMode::Formant => {
                        let mut sum = 0.0;
                        for i in 0..FORMANT_BANDS {
                            sum += self.formants[i].tick(x) * self.formant_amp[i];
                        }
                        sum
                    }
                    FilterMode::Comb => {
                        let delayed = self.comb.read(comb_delay);
                        self.comb.write(x + delayed * comb_fb);
                        delayed
                    }
                };
            }

            if fx.chorus_on {
                let lfo = self.chorus_phase.sin();
                self.chorus_phase += chorus_inc;
                let d = ((chorus_base_ms + chorus_depth * lfo) * 1e-3 * sr).max(1.0);
                let wet = self.chorus.read(d);
                self.chorus.write(x + wet * chorus_fb);
                x = x * (1.0 - chorus_mix) + wet * chorus_mix;
            }

            if fx.phaser_on {
                let lfo = self.phaser_phase.sin();
                self.phaser_phase += phaser_inc;
                let f = (p_centre + p_half * lfo).clamp(20.0, sr * 0.45);
                let coeffs = Coeffs::new(BiquadType::Allpass, f, 0.5, 0.0, sr);
                let mut y = x + self.phaser_fb_state * phaser_fb;
                for stage in self.phaser.iter_mut().take(stages) {
                    stage.set_coeffs(coeffs);
                    y = stage.tick(y);
                }
                self.phaser_fb_state = y;
                x = x * (1.0 - phaser_mix) + y * phaser_mix;
            }

            if fx.delay_on {
                let wet = self.delay.read(delay_samples);
                self.delay.write(x + wet * delay_fb);
                x = x * (1.0 - delay_mix) + wet * delay_mix;
            }

            if fx.reverb_on {
                let wet = self.reverb.tick(x);
                x = x * (1.0 - reverb_mix) + wet * reverb_mix;
            }

            if fx.limiter_on {
                x = self.limiter.tick(x);
            }

            *sample = x as f32;
        }

        self.chorus_phase %= std::f64::consts::TAU;
        self.phaser_phase %= std::f64::consts::TAU;
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FilterMode {
    Biquad,
    Formant,
    Comb,
}

fn filter_mode(t: &str) -> FilterMode {
    match t {
        "formant" => FilterMode::Formant,
        "comb" => FilterMode::Comb,
        _ => FilterMode::Biquad,
    }
}

/// Every filter type the schema allows must map to a mode we implement.
pub fn filter_types_covered() -> bool {
    schema::schema().filter_types.iter().all(|t| match filter_mode(t) {
        FilterMode::Biquad => BiquadType::parse(t).is_some(),
        _ => true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len() as f64).sqrt()
    }

    fn tone(n: usize, sr: f64, freq: f64, amp: f64) -> Vec<f32> {
        (0..n).map(|i| (amp * (std::f64::consts::TAU * freq * i as f64 / sr).sin()) as f32).collect()
    }

    #[test]
    fn every_filter_type_in_the_schema_is_implemented() {
        assert!(filter_types_covered());
    }

    #[test]
    fn a_silent_input_stays_silent_with_everything_on() {
        let sr = 48000.0;
        let fx = FxState {
            filter_on: true,
            chorus_on: true,
            phaser_on: true,
            delay_on: true,
            reverb_on: true,
            limiter_on: true,
            ..FxState::default()
        };
        let mut chain = FxChain::new(sr);
        chain.reset(&fx);
        let mut buf = vec![0.0f32; 4096];
        for _ in 0..20 {
            chain.process(&mut buf);
        }
        assert!(buf.iter().all(|v| v.abs() < 1e-9), "silence grew a tail");
    }

    #[test]
    fn the_chain_is_stable_with_every_module_at_full_feedback() {
        let sr = 48000.0;
        let fx = FxState {
            filter_on: true,
            filter_type: "comb".into(),
            filter_comb_fb: 0.95,
            chorus_on: true,
            chorus_fb: 0.95,
            phaser_on: true,
            phaser_fb: 0.9,
            delay_on: true,
            delay_fb: 0.9,
            reverb_on: true,
            reverb_decay: 8.0,
            limiter_on: true,
            ..FxState::default()
        };
        let mut chain = FxChain::new(sr);
        chain.reset(&fx);
        let mut worst: f64 = 0.0;
        for block in 0..200 {
            let mut buf = if block < 50 { tone(1024, sr, 220.0, 0.3) } else { vec![0.0f32; 1024] };
            chain.process(&mut buf);
            worst = worst.max(buf.iter().fold(0.0f64, |m, v| m.max(f64::from(v.abs()))));
        }
        assert!(worst.is_finite() && worst < 2.0, "chain blew up: {worst}");
    }

    #[test]
    fn a_lowpass_takes_the_top_off() {
        let sr = 48000.0;
        let fx = FxState { filter_on: true, filter_freq: 300.0, limiter_on: false, ..FxState::default() };
        let mut chain = FxChain::new(sr);
        chain.reset(&fx);
        let mut high = tone(48000, sr, 6000.0, 0.5);
        chain.process(&mut high);
        let mut low = tone(48000, sr, 100.0, 0.5);
        chain.reset(&fx);
        chain.process(&mut low);
        assert!(rms(&high) < 0.05 * rms(&low), "high {} low {}", rms(&high), rms(&low));
    }

    #[test]
    fn the_delay_repeats_after_its_time() {
        let sr = 48000.0;
        let fx = FxState {
            delay_on: true,
            delay_time: 0.25,
            delay_fb: 0.5,
            delay_mix: 1.0,
            limiter_on: false,
            ..FxState::default()
        };
        let mut chain = FxChain::new(sr);
        chain.reset(&fx);
        let mut buf = vec![0.0f32; (sr * 0.6) as usize];
        buf[0] = 1.0;
        chain.process(&mut buf);
        let at = (sr * 0.25) as usize;
        assert!(buf[at].abs() > 0.5, "no echo at 250 ms: {}", buf[at]);
        let at2 = (sr * 0.5) as usize;
        assert!(buf[at2].abs() > 0.2, "no second echo: {}", buf[at2]);
    }
}
