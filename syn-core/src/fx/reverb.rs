//! Feedback delay network reverb.
//!
//! The web app convolves with exponentially decaying noise; a convolution of
//! that length costs more than the whole rest of the chain on this machine, so
//! the port uses an 8-line FDN with a Householder feedback matrix and a damping
//! one-pole in each line (PLAN.md decision 3: our own DSP, stated up front).
//! Tails therefore differ from the browser's; density and decay time do not.

use super::delayline::DelayLine;

const LINES: usize = 8;
/// Mutually prime lengths in milliseconds — no common period, so echoes do not
/// line up into a flutter.
/// Calibrated against the browser, not against a textbook: a normalized
/// `ConvolverNode` fed with decaying noise returns a wet signal well below the
/// dry one (measured wet/dry 0.16 at decay 0.5 s, 0.39 at 8 s), and every
/// preset's Reverb Mix was tuned against that. Matching the shape but not the
/// level would put points 16 dB out — which is exactly what the first version
/// did.
const NORM_K: f64 = 0.88;
const LENGTHS_MS: [f64; LINES] = [23.1, 29.7, 37.3, 43.9, 53.1, 61.7, 71.3, 79.9];

#[derive(Clone, Debug)]
pub struct Fdn {
    lines: Vec<DelayLine>,
    delays: [f64; LINES],
    feedback: [f64; LINES],
    damp_state: [f64; LINES],
    damp: f64,
    /// Keeps the wet level independent of the decay time, the way the
    /// browser's normalized convolver does — without it a long tail comes out
    /// several dB louder than the same point in the web app (measured).
    norm: f64,
    sr: f64,
}

impl Fdn {
    pub fn new(sr: f64) -> Self {
        let mut delays = [0.0; LINES];
        let lines = LENGTHS_MS
            .iter()
            .enumerate()
            .map(|(i, ms)| {
                delays[i] = ms * 1e-3 * sr;
                DelayLine::new(delays[i] as usize + 4)
            })
            .collect();
        let mut fdn = Self {
            lines,
            delays,
            feedback: [0.0; LINES],
            damp_state: [0.0; LINES],
            damp: 0.3,
            norm: 1.0,
            sr,
        };
        fdn.set_decay(2.8);
        fdn
    }

    pub fn clear(&mut self) {
        for l in &mut self.lines {
            l.clear();
        }
        self.damp_state = [0.0; LINES];
    }

    /// `decay` is the web app's Reverb Decay slider, read as an RT60 in seconds.
    pub fn set_decay(&mut self, decay: f64) {
        let rt60 = decay.clamp(0.1, 8.0);
        for i in 0..LINES {
            let seconds = self.delays[i] / self.sr;
            self.feedback[i] = 10f64.powf(-3.0 * seconds / rt60);
        }
        // Longer tails are darker, as a real room is — a gentle one-pole
        // rolloff in every line, expressed as a cutoff rather than as a raw
        // coefficient (a small coefficient here is a *very* dark filter, which
        // is how the first version lost 20 dB of wet level).
        let cutoff = (6000.0 - 500.0 * rt60).clamp(1500.0, 6000.0);
        self.damp = 1.0 - (-std::f64::consts::TAU * cutoff / self.sr).exp();
        // Energy in a recirculating line grows as 1/(1-g²); undo that so the
        // wet output keeps the level of what went in.
        let mean_g2 = self.feedback.iter().map(|g| g * g).sum::<f64>() / LINES as f64;
        self.norm = (1.0 - mean_g2).sqrt().max(0.05) * NORM_K;
    }

    #[inline]
    pub fn tick(&mut self, x: f64) -> f64 {
        let mut read = [0.0f64; LINES];
        let mut sum = 0.0;
        for (i, r) in read.iter_mut().enumerate() {
            let v = self.lines[i].read(self.delays[i]);
            // one-pole damping inside the loop
            self.damp_state[i] += self.damp * (v - self.damp_state[i]);
            *r = self.damp_state[i] * self.feedback[i];
            sum += *r;
        }
        // Householder: y_i = read_i - 2/N · Σ read
        let corr = 2.0 / LINES as f64 * sum;
        for (i, line) in self.lines.iter_mut().enumerate() {
            line.write(x + read[i] - corr);
        }
        sum * self.norm / LINES as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt60_of(decay: f64) -> f64 {
        let sr = 48000.0;
        let mut r = Fdn::new(sr);
        r.set_decay(decay);
        let mut peak: f64 = 0.0;
        let mut last_loud = 0usize;
        for i in 0..(sr * 12.0) as usize {
            let y = r.tick(if i == 0 { 1.0 } else { 0.0 }).abs();
            if i < (sr * 0.2) as usize {
                peak = peak.max(y);
            } else if y > peak * 0.001 {
                last_loud = i;
            }
        }
        last_loud as f64 / sr
    }

    #[test]
    fn the_tail_lasts_about_as_long_as_the_decay_asks() {
        for decay in [1.0, 3.0, 6.0] {
            let t = rt60_of(decay);
            assert!((t - decay).abs() < decay * 0.45, "decay {decay} → -60 dB at {t:.2}s");
        }
    }

    /// The wet level must not run away with the decay time: a longer tail is
    /// longer, not louder. The absolute level is the browser's (see NORM_K).
    #[test]
    fn the_wet_level_barely_moves_with_the_decay() {
        let sr = 22050.0;
        let mut levels = Vec::new();
        for decay in [0.5, 2.8, 8.0] {
            let mut r = Fdn::new(sr);
            r.set_decay(decay);
            let mut rng = 12345u32;
            let mut energy_in = 0.0;
            let mut energy_out = 0.0;
            for i in 0..(sr * 4.0) as usize {
                rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let x = f64::from(rng >> 8) / f64::from(1u32 << 23) - 1.0;
                let y = r.tick(x);
                if i > (sr * 1.0) as usize {
                    energy_in += x * x;
                    energy_out += y * y;
                }
            }
            levels.push((energy_out / energy_in).sqrt());
        }
        for (decay, level) in [0.5, 2.8, 8.0].iter().zip(&levels) {
            assert!((0.05..=0.30).contains(level), "decay {decay}: wet/dry {level:.2}");
        }
    }

    #[test]
    fn it_stays_stable() {
        let mut r = Fdn::new(48000.0);
        r.set_decay(8.0);
        let mut worst: f64 = 0.0;
        for i in 0..48000 * 20 {
            let x = if i < 48000 { (i as f64 * 0.01).sin() } else { 0.0 };
            worst = worst.max(r.tick(x).abs());
        }
        assert!(worst < 4.0, "reverb blew up: {worst}");
    }
}
