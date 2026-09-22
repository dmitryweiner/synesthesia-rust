//! Biquad filter, direct form 1.
//!
//! The coefficients follow the Web Audio spec's `BiquadFilterNode` — including
//! its quirk that lowpass and highpass read `Q` in decibels while the other
//! types read it plainly. The FX chain is our own (PLAN.md decision 3), but a
//! filter that resonates differently would change every preset that uses one,
//! and matching the convention costs one `exp`.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BiquadType {
    #[default]
    Lowpass,
    Highpass,
    Bandpass,
    Notch,
    Peaking,
    Lowshelf,
    Highshelf,
    Allpass,
}

impl BiquadType {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "lowpass" => Self::Lowpass,
            "highpass" => Self::Highpass,
            "bandpass" => Self::Bandpass,
            "notch" => Self::Notch,
            "peaking" => Self::Peaking,
            "lowshelf" => Self::Lowshelf,
            "highshelf" => Self::Highshelf,
            "allpass" => Self::Allpass,
            _ => return None,
        })
    }
}

/// Filter coefficients without the delay elements. The phaser runs up to eight
/// identical all-pass stages, so it computes these once per sample and lends
/// them to every stage instead of paying for `sin_cos` and two `powf` per
/// stage — that alone was most of the phaser's cost.
#[derive(Clone, Copy, Debug)]
pub struct Coeffs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Default for Coeffs {
    fn default() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    c: Coeffs,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears the delay elements (a preset switch must not carry a tail).
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }

    pub fn set(&mut self, kind: BiquadType, freq: f64, q: f64, gain_db: f64, sr: f64) {
        self.c = Coeffs::new(kind, freq, q, gain_db, sr);
    }

    /// Uses coefficients computed elsewhere, keeping this filter's own state.
    #[inline]
    pub fn set_coeffs(&mut self, c: Coeffs) {
        self.c = c;
    }
}

impl Coeffs {
    pub fn new(kind: BiquadType, freq: f64, q: f64, gain_db: f64, sr: f64) -> Self {
        let nyquist = sr * 0.5;
        let f0 = freq.clamp(1e-4, nyquist * 0.999);
        let w0 = std::f64::consts::TAU * f0 / sr;
        let (sin_w0, cos_w0) = w0.sin_cos();
        // Lowpass/highpass take Q in dB, the rest take it as a plain factor.
        let q_db = 10f64.powf(q / 20.0);
        let alpha_db = sin_w0 / (2.0 * q_db.max(1e-6));
        let alpha_q = sin_w0 / (2.0 * q.max(1e-4));
        let a = 10f64.powf(gain_db / 40.0);

        let (b0, b1, b2, a0, a1, a2) = match kind {
            BiquadType::Lowpass => {
                let b1 = 1.0 - cos_w0;
                (b1 * 0.5, b1, b1 * 0.5, 1.0 + alpha_db, -2.0 * cos_w0, 1.0 - alpha_db)
            }
            BiquadType::Highpass => {
                let s = 1.0 + cos_w0;
                (s * 0.5, -s, s * 0.5, 1.0 + alpha_db, -2.0 * cos_w0, 1.0 - alpha_db)
            }
            BiquadType::Bandpass => (alpha_q, 0.0, -alpha_q, 1.0 + alpha_q, -2.0 * cos_w0, 1.0 - alpha_q),
            BiquadType::Notch => (1.0, -2.0 * cos_w0, 1.0, 1.0 + alpha_q, -2.0 * cos_w0, 1.0 - alpha_q),
            BiquadType::Allpass => {
                (1.0 - alpha_q, -2.0 * cos_w0, 1.0 + alpha_q, 1.0 + alpha_q, -2.0 * cos_w0, 1.0 - alpha_q)
            }
            BiquadType::Peaking => (
                1.0 + alpha_q * a,
                -2.0 * cos_w0,
                1.0 - alpha_q * a,
                1.0 + alpha_q / a,
                -2.0 * cos_w0,
                1.0 - alpha_q / a,
            ),
            BiquadType::Lowshelf => {
                let s = 2.0 * a.sqrt() * alpha_q;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos_w0 + s),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0),
                    a * ((a + 1.0) - (a - 1.0) * cos_w0 - s),
                    (a + 1.0) + (a - 1.0) * cos_w0 + s,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0),
                    (a + 1.0) + (a - 1.0) * cos_w0 - s,
                )
            }
            BiquadType::Highshelf => {
                let s = 2.0 * a.sqrt() * alpha_q;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos_w0 + s),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0),
                    a * ((a + 1.0) + (a - 1.0) * cos_w0 - s),
                    (a + 1.0) - (a - 1.0) * cos_w0 + s,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos_w0),
                    (a + 1.0) - (a - 1.0) * cos_w0 - s,
                )
            }
        };

        let inv = 1.0 / a0;
        Self { b0: b0 * inv, b1: b1 * inv, b2: b2 * inv, a1: a1 * inv, a2: a2 * inv }
    }
}

impl Biquad {
    #[inline]
    pub fn tick(&mut self, x: f64) -> f64 {
        let c = &self.c;
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Magnitude response: feed a sine, compare settled RMS in to RMS out.
    /// RMS rather than peak, because a few samples per period never land on
    /// the crest and the peak ratio is then biased by the phase shift.
    fn gain_at(kind: BiquadType, cutoff: f64, q: f64, gain_db: f64, freq: f64, sr: f64) -> f64 {
        let mut f = Biquad::new();
        f.set(kind, cutoff, q, gain_db, sr);
        let n = (sr * 0.5) as usize;
        let (mut e_in, mut e_out) = (0.0, 0.0);
        for i in 0..n {
            let x = (std::f64::consts::TAU * freq * i as f64 / sr).sin();
            let y = f.tick(x);
            if i > n / 2 {
                e_in += x * x;
                e_out += y * y;
            }
        }
        (e_out / e_in).sqrt()
    }

    #[test]
    fn lowpass_passes_below_and_stops_above() {
        let sr = 48000.0;
        assert!((gain_at(BiquadType::Lowpass, 1000.0, 0.7, 0.0, 100.0, sr) - 1.0).abs() < 0.05);
        assert!(gain_at(BiquadType::Lowpass, 1000.0, 0.7, 0.0, 8000.0, sr) < 0.05);
    }

    #[test]
    fn highpass_is_the_mirror_of_lowpass() {
        let sr = 48000.0;
        let lo = gain_at(BiquadType::Highpass, 1000.0, 0.7, 0.0, 100.0, sr);
        let hi = gain_at(BiquadType::Highpass, 1000.0, 0.7, 0.0, 8000.0, sr);
        assert!(lo < 0.05, "100 Hz: {lo}");
        assert!((hi - 1.0).abs() < 0.05, "8000 Hz: {hi}");
    }

    #[test]
    fn bandpass_peaks_at_its_centre() {
        let sr = 48000.0;
        let peak = gain_at(BiquadType::Bandpass, 1000.0, 4.0, 0.0, 1000.0, sr);
        assert!((peak - 1.0).abs() < 0.05, "{peak}");
        assert!(gain_at(BiquadType::Bandpass, 1000.0, 4.0, 0.0, 200.0, sr) < 0.3);
    }

    #[test]
    fn allpass_keeps_the_amplitude() {
        let sr = 48000.0;
        for f in [100.0, 1000.0, 5000.0] {
            let g = gain_at(BiquadType::Allpass, 1000.0, 0.5, 0.0, f, sr);
            assert!((g - 1.0).abs() < 0.02, "{f} Hz: {g}");
        }
    }

    #[test]
    fn peaking_lifts_its_centre_by_the_gain() {
        let sr = 48000.0;
        let g = gain_at(BiquadType::Peaking, 1000.0, 2.0, 12.0, 1000.0, sr);
        assert!((g - 10f64.powf(12.0 / 20.0)).abs() < 0.2, "{g}");
    }
}
