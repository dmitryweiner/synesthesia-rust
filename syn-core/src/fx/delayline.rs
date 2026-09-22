//! Fractional delay line with linear interpolation — the building block of the
//! comb filter, the chorus/flanger, the delay and the reverb.

#[derive(Clone, Debug)]
pub struct DelayLine {
    buf: Vec<f64>,
    write: usize,
}

impl DelayLine {
    /// Capacity in samples; reads are clamped to it.
    pub fn new(capacity: usize) -> Self {
        Self { buf: vec![0.0; capacity.max(2)], write: 0 }
    }

    pub fn clear(&mut self) {
        self.buf.fill(0.0);
        self.write = 0;
    }

    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// Reads `delay` samples back (fractional), then writes `x`.
    #[inline]
    pub fn tick(&mut self, x: f64, delay: f64) -> f64 {
        let y = self.read(delay);
        self.write(x);
        y
    }

    /// The sample written `delay` samples ago, counting the one about to be
    /// written as 0 — so `read` before `write` in a feedback loop gives exactly
    /// `delay` samples of delay.
    #[inline]
    pub fn read(&self, delay: f64) -> f64 {
        let n = self.buf.len();
        let d = delay.clamp(1.0, (n - 2) as f64);
        let i = d.floor();
        let frac = d - i;
        let i0 = (self.write + 1 + n - i as usize) % n;
        let i1 = (i0 + n - 1) % n;
        self.buf[i0] * (1.0 - frac) + self.buf[i1] * frac
    }

    #[inline]
    pub fn write(&mut self, x: f64) {
        self.write = (self.write + 1) % self.buf.len();
        self.buf[self.write] = x;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_impulse_comes_back_after_the_delay() {
        let mut d = DelayLine::new(64);
        let mut out = Vec::new();
        for i in 0..20 {
            out.push(d.tick(if i == 0 { 1.0 } else { 0.0 }, 8.0));
        }
        assert_eq!(out[8], 1.0, "{out:?}");
        assert!(out.iter().enumerate().all(|(i, v)| i == 8 || *v == 0.0));
    }

    #[test]
    fn a_half_sample_delay_splits_the_impulse() {
        let mut d = DelayLine::new(64);
        let mut out = Vec::new();
        for i in 0..20 {
            out.push(d.tick(if i == 0 { 1.0 } else { 0.0 }, 8.5));
        }
        assert!((out[8] - 0.5).abs() < 1e-12);
        assert!((out[9] - 0.5).abs() < 1e-12);
    }
}
