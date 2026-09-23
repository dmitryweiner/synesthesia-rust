//! What the picture listens to.
//!
//! The browser reads an `AnalyserNode` every animation frame and turns it into
//! loudness, swell, brightness, three band levels and an onset envelope
//! (`../synesthesia/src/audio/features.ts`). The engine here does the analysis
//! itself — a Blackman-windowed FFT with the same size, smoothing and dB range
//! as the node — and publishes the same features under the same names, so the
//! TUI today and a renderer later read one bus (PLAN.md decision 5).

use crate::analysis::fft::fft;

/// Matches `analyserNode.fftSize` in the web app.
pub const FFT_SIZE: usize = 2048;
const SMOOTHING: f64 = 0.5;
const MIN_DB: f64 = -100.0;
const MAX_DB: f64 = -30.0;
/// Analysis frames per second — the rate the browser's animation loop ran at.
pub const FPS: f64 = 60.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioFeatures {
    /// 0..1, smoothed RMS.
    pub loudness: f64,
    /// -1..1, loudness against its ~4 s average (0 = steady).
    pub swell: f64,
    /// 0..1, spectral centroid on a log-frequency scale.
    pub brightness: f64,
    /// 0..1, decaying spike on spectral energy jumps.
    pub onset: f64,
    /// 0..1, 30–250 Hz.
    pub low: f64,
    /// 0..1, 250–2000 Hz.
    pub mid: f64,
    /// 0..1, 2–10 kHz.
    pub high: f64,
}

/// `AnalyserNode` in the shape the engine uses it: feed samples, get byte
/// spectra at [`FPS`].
pub struct Analyser {
    sr: f64,
    window: Vec<f64>,
    ring: Vec<f32>,
    write: usize,
    filled: usize,
    since_frame: f64,
    hop: f64,
    smooth: Vec<f64>,
    re: Vec<f64>,
    im: Vec<f64>,
    /// The most recent byte spectrum (`getByteFrequencyData`).
    pub bytes: Vec<u8>,
    /// RMS of the most recent analysis window (`getFloatTimeDomainData`).
    pub rms: f64,
}

impl Analyser {
    pub fn new(sample_rate: f64) -> Self {
        let window = (0..FFT_SIZE)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / FFT_SIZE as f64;
                0.42 - 0.5 * a.cos() + 0.08 * (2.0 * a).cos()
            })
            .collect();
        Self {
            sr: sample_rate,
            window,
            ring: vec![0.0; FFT_SIZE],
            write: 0,
            filled: 0,
            since_frame: 0.0,
            hop: sample_rate / FPS,
            smooth: vec![0.0; FFT_SIZE / 2],
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
            bytes: vec![0; FFT_SIZE / 2],
            rms: 0.0,
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.sr
    }

    /// Feeds a block. Returns true when a new spectrum was computed.
    pub fn push(&mut self, block: &[f32]) -> bool {
        for s in block {
            self.ring[self.write] = *s;
            self.write = (self.write + 1) % FFT_SIZE;
            self.filled = (self.filled + 1).min(FFT_SIZE);
        }
        self.since_frame += block.len() as f64;
        if self.since_frame < self.hop || self.filled < FFT_SIZE {
            return false;
        }
        self.since_frame -= self.hop;
        self.analyse();
        true
    }

    fn analyse(&mut self) {
        let mut sum = 0.0;
        for i in 0..FFT_SIZE {
            let v = f64::from(self.ring[(self.write + i) % FFT_SIZE]);
            sum += v * v;
            self.re[i] = v * self.window[i];
            self.im[i] = 0.0;
        }
        self.rms = (sum / FFT_SIZE as f64).sqrt();
        fft(&mut self.re, &mut self.im, false);
        for b in 0..FFT_SIZE / 2 {
            let mag = self.re[b].hypot(self.im[b]) / FFT_SIZE as f64;
            self.smooth[b] = SMOOTHING * self.smooth[b] + (1.0 - SMOOTHING) * mag;
            let db = if self.smooth[b] > 0.0 { 20.0 * self.smooth[b].log10() } else { f64::NEG_INFINITY };
            let scaled = 255.0 * (db - MIN_DB) / (MAX_DB - MIN_DB);
            self.bytes[b] = scaled.clamp(0.0, 255.0) as u8;
        }
    }
}

// Smoothing is time-based (τ in seconds), as in the web app.
const LOUD_ATTACK_TAU: f64 = 0.024;
const LOUD_RELEASE_TAU: f64 = 0.2;
const BRIGHT_TAU: f64 = 0.075;
const BAND_TAU: f64 = 0.058;
const ONSET_DECAY: f64 = 0.85; // per 1/60 s
const ONSET_BANDS: usize = 20;
const ONSET_K: f64 = 3.0;
const ONSET_MIN_RISE: f64 = 1.0;
const ONSET_SCALE: f64 = 6.0;
const ONSET_TAU: f64 = 1.0;
const LOUD_GAIN: f64 = 2.2;
const SWELL_TAU: f64 = 4.0;
const SWELL_GAIN: f64 = 10.0;
const BAND_FLOOR: f64 = 0.2;
const BAND_SPAN: f64 = 0.55;
const BANDS: [(f64, f64); 3] = [(30.0, 250.0), (250.0, 2000.0), (2000.0, 10000.0)];

fn band_level(bins: &[u8], hz_per_bin: f64, lo: f64, hi: f64) -> f64 {
    let i0 = (lo / hz_per_bin).floor().max(0.0) as usize;
    let i1 = ((hi / hz_per_bin).ceil() as usize).min(bins.len());
    if i1 <= i0 {
        return 0.0;
    }
    let sum: f64 = bins[i0..i1].iter().map(|b| f64::from(*b)).sum();
    ((sum / (i1 - i0) as f64 / 255.0 - BAND_FLOOR) / BAND_SPAN).clamp(0.0, 1.0)
}

pub fn spectral_centroid(bins: &[u8], sample_rate: f64) -> f64 {
    let hz_per_bin = sample_rate / 2.0 / bins.len() as f64;
    let mut num = 0.0;
    let mut den = 0.0;
    for (i, b) in bins.iter().enumerate() {
        num += i as f64 * hz_per_bin * f64::from(*b);
        den += f64::from(*b);
    }
    if den > 0.0 {
        num / den
    } else {
        0.0
    }
}

pub struct FeatureTracker {
    sample_rate: f64,
    prev_bands: Option<[f64; ONSET_BANDS]>,
    rise_avg: f64,
    rise_dev: f64,
    loud: f64,
    slow_loud: f64,
    slow_loud_set: bool,
    bright: f64,
    onset: f64,
    bands: [f64; 3],
}

impl FeatureTracker {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            sample_rate,
            prev_bands: None,
            rise_avg: 0.0,
            rise_dev: 0.0,
            loud: 0.0,
            slow_loud: 0.0,
            slow_loud_set: false,
            bright: 0.0,
            onset: 0.0,
            bands: [0.0; 3],
        }
    }

    /// One analysis frame; `dt` (seconds) drives every time-based average.
    pub fn update(&mut self, rms: f64, bins: &[u8], dt: f64) -> AudioFeatures {
        let ease = |tau: f64| 1.0 - (-dt / tau).exp();
        let loud_target = (rms * LOUD_GAIN).min(1.0);
        let tau = if loud_target > self.loud { LOUD_ATTACK_TAU } else { LOUD_RELEASE_TAU };
        self.loud += (loud_target - self.loud) * ease(tau);

        let centroid = spectral_centroid(bins, self.sample_rate);
        let b = if centroid > 0.0 { (centroid / 50.0).ln() / (8000.0f64 / 50.0).ln() } else { 0.0 };
        self.bright += (b.clamp(0.0, 1.0) - self.bright) * ease(BRIGHT_TAU);

        let hz_per_bin = self.sample_rate / 2.0 / bins.len().max(1) as f64;
        let mut levels = [0.0f64; ONSET_BANDS];
        for (b, level) in levels.iter_mut().enumerate() {
            let lo = 60.0 * (10000.0f64 / 60.0).powf(b as f64 / ONSET_BANDS as f64);
            let hi = 60.0 * (10000.0f64 / 60.0).powf((b + 1) as f64 / ONSET_BANDS as f64);
            let i0 = ((lo / hz_per_bin).floor() as usize).min(bins.len() - 1);
            let i1 = ((hi / hz_per_bin).ceil() as usize).clamp(i0 + 1, bins.len());
            let sum: f64 = bins[i0..i1].iter().map(|v| f64::from(*v)).sum();
            *level = sum / (i1 - i0) as f64;
        }
        let mut rise = 0.0;
        if let Some(prev) = &self.prev_bands {
            for b in 0..ONSET_BANDS {
                rise += (levels[b] - prev[b]).max(0.0);
            }
        }
        rise /= ONSET_BANDS as f64;
        self.prev_bands = Some(levels);
        let strength =
            ((rise - self.rise_avg - ONSET_K * self.rise_dev - ONSET_MIN_RISE) / ONSET_SCALE).clamp(0.0, 1.0);
        let a = (dt / ONSET_TAU).min(1.0);
        self.rise_avg += (rise - self.rise_avg) * a;
        self.rise_dev += ((rise - self.rise_avg).abs() - self.rise_dev) * a;
        self.onset = strength.max(self.onset * ONSET_DECAY.powf(dt * 60.0));

        if !self.slow_loud_set {
            self.slow_loud = self.loud;
            self.slow_loud_set = true;
        }
        self.slow_loud += (self.loud - self.slow_loud) * (dt / SWELL_TAU).min(1.0);
        let swell = ((self.loud - self.slow_loud) * SWELL_GAIN).tanh();

        for (i, (lo, hi)) in BANDS.iter().enumerate() {
            let level = band_level(bins, hz_per_bin, *lo, *hi);
            self.bands[i] += (level - self.bands[i]) * ease(BAND_TAU);
        }

        AudioFeatures {
            loudness: self.loud,
            swell,
            brightness: self.bright,
            onset: self.onset,
            low: self.bands[0],
            mid: self.bands[1],
            high: self.bands[2],
        }
    }
}

const ONSET_ON: f64 = 0.35;
const ONSET_REARM: f64 = 0.2;
const ONSET_REFRACTORY: f64 = 0.12;

/// Turns the decaying onset envelope into discrete hits (rising edges).
pub struct OnsetDetector {
    armed: bool,
    last: f64,
}

impl Default for OnsetDetector {
    fn default() -> Self {
        Self { armed: true, last: f64::NEG_INFINITY }
    }
}

impl OnsetDetector {
    /// True once per hit; `t` is in seconds.
    pub fn update(&mut self, onset: f64, t: f64) -> bool {
        if !self.armed {
            if onset < ONSET_REARM {
                self.armed = true;
            }
            return false;
        }
        if onset >= ONSET_ON && t - self.last >= ONSET_REFRACTORY {
            self.armed = false;
            self.last = t;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, sr: f64, freq: f64, amp: f64) -> Vec<f32> {
        (0..n).map(|i| (amp * (std::f64::consts::TAU * freq * i as f64 / sr).sin()) as f32).collect()
    }

    #[test]
    fn a_loud_tone_reads_louder_than_a_quiet_one() {
        let sr = 48000.0;
        let run = |amp: f64| {
            let mut an = Analyser::new(sr);
            let mut tr = FeatureTracker::new(sr);
            let mut f = AudioFeatures::default();
            for block in tone(sr as usize, sr, 440.0, amp).chunks(128) {
                if an.push(block) {
                    f = tr.update(an.rms, &an.bytes, 1.0 / FPS);
                }
            }
            f
        };
        let quiet = run(0.05);
        let loud = run(0.5);
        assert!(loud.loudness > quiet.loudness * 3.0, "{} vs {}", loud.loudness, quiet.loudness);
        assert!(loud.loudness <= 1.0);
    }

    #[test]
    fn brightness_follows_the_pitch() {
        let sr = 48000.0;
        let run = |freq: f64| {
            let mut an = Analyser::new(sr);
            let mut tr = FeatureTracker::new(sr);
            let mut f = AudioFeatures::default();
            for block in tone(sr as usize, sr, freq, 0.3).chunks(128) {
                if an.push(block) {
                    f = tr.update(an.rms, &an.bytes, 1.0 / FPS);
                }
            }
            f
        };
        let low = run(120.0);
        let high = run(4000.0);
        println!("low tone:  {low:?}\nhigh tone: {high:?}");
        assert!(high.brightness > low.brightness + 0.3, "{} vs {}", high.brightness, low.brightness);
        assert!(low.low > high.low, "bass band: {} vs {}", low.low, high.low);
    }

    #[test]
    fn a_burst_after_silence_is_an_onset() {
        let sr = 48000.0;
        let mut an = Analyser::new(sr);
        let mut tr = FeatureTracker::new(sr);
        let mut det = OnsetDetector::default();
        let mut hits = 0;
        let mut t = 0.0;
        let silence = vec![0.0f32; (sr * 0.5) as usize];
        let burst = tone((sr * 0.25) as usize, sr, 800.0, 0.5);
        let mut signal = silence.clone();
        signal.extend_from_slice(&burst);
        signal.extend_from_slice(&silence);
        signal.extend_from_slice(&burst);
        for block in signal.chunks(128) {
            if an.push(block) {
                let f = tr.update(an.rms, &an.bytes, 1.0 / FPS);
                t += 1.0 / FPS;
                if det.update(f.onset, t) {
                    hits += 1;
                }
            }
        }
        assert!((2..=4).contains(&hits), "two bursts gave {hits} hits");
    }
}
