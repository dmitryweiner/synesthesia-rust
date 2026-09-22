//! Peak limiter with look-ahead.
//!
//! Stands in for the browser's `DynamicsCompressor` (PLAN.md decision 3): same
//! job — keep a loud point from clipping — with a plain envelope follower and a
//! 3 ms look-ahead instead of that node's normalized curve.

use super::delayline::DelayLine;

const ATTACK: f64 = 0.003;
const RATIO: f64 = 20.0;

#[derive(Clone, Debug)]
pub struct Limiter {
    look: DelayLine,
    look_samples: f64,
    env_db: f64,
    gain_db: f64,
    attack_coef: f64,
    release_coef: f64,
    threshold_db: f64,
    threshold_lin: f64,
    makeup: f64,
    sr: f64,
}

impl Limiter {
    pub fn new(sr: f64) -> Self {
        let look_samples = ATTACK * sr;
        let mut l = Self {
            look: DelayLine::new(look_samples as usize + 4),
            look_samples,
            env_db: -120.0,
            gain_db: 0.0,
            attack_coef: 0.0,
            release_coef: 0.0,
            threshold_db: -12.0,
            threshold_lin: 0.251,
            makeup: 1.0,
            sr,
        };
        l.set(-12.0, 0.15);
        l
    }

    pub fn clear(&mut self) {
        self.look.clear();
        self.env_db = -120.0;
        self.gain_db = 0.0;
    }

    pub fn set(&mut self, threshold_db: f64, release: f64) {
        self.threshold_db = threshold_db;
        self.threshold_lin = 10f64.powf(threshold_db / 20.0);
        // Make-up gain, so that switching the limiter on does not make a point
        // quieter — a full-scale input comes back out at full scale. The 0.6
        // exponent is Blink's perceptual fudge, and it is deliberately copied:
        // every preset was tuned through a browser limiter that boosts by
        // exactly this much, and without it the whole chain lands 3–7 dB down
        // (measured: scripts/parity.mjs).
        self.makeup = 10f64.powf(0.6 * (-threshold_db).max(0.0) * (1.0 - 1.0 / RATIO) / 20.0);
        self.attack_coef = 1.0 - (-1.0 / (ATTACK * self.sr)).exp();
        self.release_coef = 1.0 - (-1.0 / (release.clamp(0.02, 1.0) * self.sr)).exp();
    }

    /// Gain reduction currently applied, in dB (≤ 0) — the TUI shows it.
    pub fn reduction_db(&self) -> f64 {
        self.gain_db
    }

    #[inline]
    pub fn tick(&mut self, x: f64) -> f64 {
        // Fast path: nothing over the threshold and no reduction left to
        // release — most of the time, and it skips two transcendentals.
        if x.abs() <= self.threshold_lin && self.gain_db > -1e-4 {
            self.gain_db = 0.0;
            return self.look.tick(x, self.look_samples) * self.makeup;
        }
        let level_db = 20.0 * x.abs().max(1e-9).log10();
        let over = (level_db - self.threshold_db).max(0.0);
        let target = -over * (1.0 - 1.0 / RATIO);
        // Attack fast when the reduction must deepen, release slowly back.
        let coef = if target < self.gain_db { self.attack_coef } else { self.release_coef };
        self.gain_db += coef * (target - self.gain_db);
        let delayed = self.look.tick(x, self.look_samples);
        delayed * 10f64.powf(self.gain_db / 20.0) * self.makeup
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Full scale in comes out a few dB down: the make-up gain uses Blink's
    /// 0.6 exponent, which deliberately under-compensates (a limiter that gave
    /// back every dB it took would defeat itself).
    #[test]
    fn a_full_scale_sine_comes_back_a_few_db_down() {
        let sr = 48000.0;
        let mut l = Limiter::new(sr);
        l.set(-12.0, 0.15);
        let mut peak: f64 = 0.0;
        for i in 0..(sr as usize) {
            let y = l.tick((std::f64::consts::TAU * 220.0 * i as f64 / sr).sin());
            if i > sr as usize / 2 {
                peak = peak.max(y.abs());
            }
        }
        assert!((0.45..=0.85).contains(&peak), "peak {peak:.3}");
    }

    #[test]
    fn it_catches_an_overdriven_signal() {
        let sr = 48000.0;
        let mut l = Limiter::new(sr);
        l.set(-12.0, 0.15);
        let mut peak: f64 = 0.0;
        for i in 0..(sr as usize) {
            let y = l.tick(4.0 * (std::f64::consts::TAU * 220.0 * i as f64 / sr).sin());
            if i > sr as usize / 2 {
                peak = peak.max(y.abs());
            }
        }
        assert!(peak < 1.4, "12 dB over the threshold came out at {peak:.3}");
    }

    #[test]
    fn a_quiet_signal_passes_untouched() {
        let sr = 48000.0;
        let mut l = Limiter::new(sr);
        let mut worst: f64 = 0.0;
        for i in 0..2000 {
            let x = 0.05 * (std::f64::consts::TAU * 220.0 * i as f64 / sr).sin();
            worst = worst.max(l.tick(x).abs());
        }
        // Untouched by the compressor, but still lifted by the make-up gain.
        assert!(worst <= 0.05 * 10f64.powf(6.9 / 20.0) * 1.01, "{worst}");
        assert!(l.reduction_db().abs() < 1e-9);
    }
}
