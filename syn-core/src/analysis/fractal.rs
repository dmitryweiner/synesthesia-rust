//! "Fractality" of a sound, ported from
//! `../synesthesia/src/analysis/fractal.ts`.
//!
//! People tend to like sound whose fluctuations look alike across time scales
//! (Voss & Clarke, 1975): β ≈ 1 in both loudness and pitch contours, and a
//! spectrogram that is structured rather than empty or solid. Four measures,
//! all from one STFT, folded into a score in [0,1] — what the scout ranks
//! candidates by.

use super::fft::{fft, next_pow2};

const FRAME_SECONDS: f64 = 0.046;
const HOP_SECONDS: f64 = 0.02;
const FIT_MAX_HZ: f64 = 5.0;
const BANDS: usize = 64;
const SILENCE_DB: f64 = -60.0;
/// A steady tone's envelope ripples by ~1e-3 dB from frame alignment alone;
/// below these, a contour carries no structure to fit.
const STATIC_ENV_DB: f64 = 0.5;
const STATIC_CENTROID_OCT: f64 = 0.02;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundAnalysis {
    pub silent: bool,
    /// dBFS of the overall RMS.
    pub loudness: f64,
    /// β of the loudness envelope's 1/f^β spectrum (pink = 1).
    pub env_beta: f64,
    /// β of the spectral-centroid contour.
    pub centroid_beta: f64,
    /// Higuchi fractal dimension of the loudness envelope.
    pub env_higuchi: f64,
    /// Box-counting dimension of the salient spectrogram cells.
    pub box_dim: f64,
    pub score: f64,
}

fn mean(x: &[f64]) -> f64 {
    if x.is_empty() {
        0.0
    } else {
        x.iter().sum::<f64>() / x.len() as f64
    }
}

/// Least-squares slope of `ys` over `xs`.
fn slope(xs: &[f64], ys: &[f64]) -> f64 {
    if xs.len() < 2 {
        return f64::NAN;
    }
    let (mx, my) = (mean(xs), mean(ys));
    let mut num = 0.0;
    let mut den = 0.0;
    for (x, y) in xs.iter().zip(ys) {
        num += (x - mx) * (y - my);
        den += (x - mx) * (x - mx);
    }
    if den > 0.0 {
        num / den
    } else {
        f64::NAN
    }
}

fn std_dev(x: &[f64]) -> f64 {
    let m = mean(x);
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / x.len() as f64).sqrt()
}

/// β of a series' 1/f^β power spectrum: detrend, Hann window, FFT, power
/// averaged in log-spaced bins, log-log regression. NaN when there is nothing
/// to fit.
pub fn spectral_slope(series: &[f64], max_frac: f64) -> f64 {
    let n = series.len();
    if n < 64 {
        return f64::NAN;
    }
    let idx: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let b = slope(&idx, series);
    let a = mean(series) - b * (n - 1) as f64 / 2.0;
    let size = next_pow2(n);
    let mut re = vec![0.0; size];
    let mut im = vec![0.0; size];
    let mut energy = 0.0;
    for i in 0..n {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / (n - 1) as f64).cos();
        let v = (series[i] - (a + b * i as f64)) * w;
        re[i] = v;
        energy += v * v;
    }
    // NaN-safe: a constant or empty series has nothing to fit.
    if !matches!(energy.partial_cmp(&1e-18), Some(std::cmp::Ordering::Greater)) {
        return f64::NAN;
    }
    fft(&mut re, &mut im, false);
    let half = size / 2;
    // Skip the lowest bins (window leakage and the detrend) — fit from bin 2.
    let top = (half as f64 * max_frac.min(1.0)).floor().max(8.0);
    let lo = 2.0f64.ln();
    let hi = top.ln();
    let nb = (((hi - lo) * 4.0).round() as usize).clamp(8, 24);
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for k in 0..nb {
        let f0 = (lo + (hi - lo) * k as f64 / nb as f64).exp().floor();
        let f1 = (lo + (hi - lo) * (k + 1) as f64 / nb as f64).exp().floor().max(f0 + 1.0);
        let (mut p, mut c) = (0.0, 0usize);
        let mut f = f0 as usize;
        while (f as f64) < f1 && (f as f64) < top {
            p += re[f] * re[f] + im[f] * im[f];
            c += 1;
            f += 1;
        }
        if c == 0 || p <= 0.0 {
            continue;
        }
        xs.push(((f0 + f1 - 1.0) / 2.0).ln());
        ys.push((p / c as f64).ln());
    }
    let s = slope(&xs, &ys);
    if s.is_finite() {
        -s
    } else {
        f64::NAN
    }
}

/// Higuchi fractal dimension (1 = smooth curve, 2 = white noise).
pub fn higuchi_fd(series: &[f64], k_max: usize) -> f64 {
    let n = series.len();
    if n < k_max * 4 {
        return f64::NAN;
    }
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for k in 1..=k_max {
        let mut lk = 0.0;
        for m in 0..k {
            let steps = (n - 1 - m) / k;
            if steps < 1 {
                continue;
            }
            let mut len = 0.0;
            for i in 1..=steps {
                len += (series[m + i * k] - series[m + (i - 1) * k]).abs();
            }
            lk += (len * (n - 1) as f64) / (steps * k) as f64 / k as f64;
        }
        lk /= k as f64;
        if lk <= 0.0 {
            return f64::NAN;
        }
        xs.push((1.0 / k as f64).ln());
        ys.push(lk.ln());
    }
    slope(&xs, &ys)
}

/// Box-counting dimension of a binary image, box sizes 1, 2, 4, … up to a
/// quarter of the shorter side. 0 for an empty image.
pub fn box_count_dimension(img: &[u8], width: usize, height: usize) -> f64 {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let max_box = (width.min(height) / 4).max(1);
    let mut s = 1usize;
    while s <= max_box {
        let mut count = 0usize;
        let mut by = 0;
        while by < height {
            let mut bx = 0;
            while bx < width {
                let mut hit = false;
                'cell: for y in by..height.min(by + s) {
                    for x in bx..width.min(bx + s) {
                        if img[y * width + x] != 0 {
                            hit = true;
                            break 'cell;
                        }
                    }
                }
                if hit {
                    count += 1;
                }
                bx += s;
            }
            by += s;
        }
        if count == 0 {
            return 0.0;
        }
        xs.push((1.0 / s as f64).ln());
        ys.push((count as f64).ln());
        s *= 2;
    }
    let d = slope(&xs, &ys);
    if d.is_finite() {
        d
    } else {
        0.0
    }
}

/// Gaussian preference: 1 at the target, falling off with width; 0 for NaN.
pub fn preference(v: f64, target: f64, width: f64) -> f64 {
    if !v.is_finite() {
        return 0.0;
    }
    let z = (v - target) / width;
    (-z * z).exp()
}

fn fractal_score(silent: bool, env_beta: f64, centroid_beta: f64, box_dim: f64) -> f64 {
    if silent {
        return 0.0;
    }
    let env = preference(env_beta, 1.0, 0.7);
    let cen = preference(centroid_beta, 1.0, 0.7);
    let box_p = preference(box_dim, 1.6, 0.3);
    (2.0 * env + 2.0 * cen + box_p) / 5.0
}

struct Stft {
    env: Vec<f64>,
    centroid: Vec<f64>,
    bands: Vec<f64>,
    frames: usize,
}

fn stft(signal: &[f32], sr: f64) -> Stft {
    let frame = 2f64.powf((sr * FRAME_SECONDS).log2().round().max(6.0)) as usize;
    let hop = ((sr * HOP_SECONDS).round() as usize).max(1);
    let frames = if signal.len() >= frame { (signal.len() - frame) / hop + 1 } else { 0 };
    let mut env = Vec::with_capacity(frames);
    let mut centroid = Vec::with_capacity(frames);
    let mut bands = vec![0.0; frames * BANDS];
    let mut re = vec![0.0; frame];
    let mut im = vec![0.0; frame];
    let hann: Vec<f64> = (0..frame)
        .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / (frame - 1) as f64).cos())
        .collect();
    let half = frame / 2;
    let hz_per_bin = sr / frame as f64;
    let (f_lo, f_hi) = (50.0, sr / 2.0);
    let edges: Vec<usize> = (0..=BANDS)
        .map(|b| {
            let hz = f_lo * (f_hi / f_lo).powf(b as f64 / BANDS as f64);
            ((hz / hz_per_bin).round() as usize).clamp(1, half)
        })
        .collect();

    for f in 0..frames {
        let off = f * hop;
        let mut sq = 0.0;
        for i in 0..frame {
            let v = f64::from(signal[off + i]);
            sq += v * v;
            re[i] = v * hann[i];
            im[i] = 0.0;
        }
        env.push((sq / frame as f64).sqrt());
        fft(&mut re, &mut im, false);
        let (mut num, mut den) = (0.0, 0.0);
        for k in 1..half {
            let mag = re[k].hypot(im[k]);
            num += k as f64 * hz_per_bin * mag;
            den += mag;
        }
        centroid.push(if den > 1e-9 { num / den } else { f64::NAN });
        for b in 0..BANDS {
            let k0 = edges[b];
            let k1 = edges[b + 1].max(k0 + 1);
            let mut p = 0.0;
            for k in k0..k1.min(half) {
                p += re[k] * re[k] + im[k] * im[k];
            }
            bands[f * BANDS + b] = 10.0 * (p / (k1 - k0) as f64 + 1e-12).log10();
        }
    }
    Stft { env, centroid, bands, frames }
}

/// Full analysis of a mono signal. A few seconds are needed for it to mean
/// anything; twenty are better.
pub fn analyze_sound(signal: &[f32], sr: f64) -> SoundAnalysis {
    let sq: f64 = signal.iter().map(|v| f64::from(*v) * f64::from(*v)).sum();
    let rms = if signal.is_empty() { 0.0 } else { (sq / signal.len() as f64).sqrt() };
    let loudness = 20.0 * (rms + 1e-12).log10();
    if loudness < SILENCE_DB {
        return SoundAnalysis {
            silent: true,
            loudness,
            env_beta: f64::NAN,
            centroid_beta: f64::NAN,
            env_higuchi: f64::NAN,
            box_dim: 0.0,
            score: 0.0,
        };
    }
    let s = stft(signal, sr);

    // Loudness contour in dB, floored so silent gaps do not dominate the fit.
    let env_db: Vec<f64> = s.env.iter().map(|v| (20.0 * (v + 1e-12).log10()).max(SILENCE_DB)).collect();
    // Timbre contour in octaves; a silent frame keeps the last known value.
    let mut last = f64::NAN;
    let cen: Vec<f64> = s
        .centroid
        .iter()
        .map(|c| {
            if c.is_finite() && *c > 0.0 {
                last = c.log2();
            }
            if last.is_finite() {
                last
            } else {
                0.0
            }
        })
        .collect();

    // Salient cells: the loudest 20% of the spectrogram.
    let mut sorted = s.bands.clone();
    sorted.sort_by(f64::total_cmp);
    let thr = sorted.get((sorted.len() as f64 * 0.8) as usize).copied().unwrap_or(0.0);
    let img: Vec<u8> = s.bands.iter().map(|v| u8::from(*v > thr)).collect();

    let fit_frac = FIT_MAX_HZ / (0.5 / HOP_SECONDS);
    let env_beta =
        if std_dev(&env_db) < STATIC_ENV_DB { f64::NAN } else { spectral_slope(&env_db, fit_frac) };
    let centroid_beta =
        if std_dev(&cen) < STATIC_CENTROID_OCT { f64::NAN } else { spectral_slope(&cen, fit_frac) };
    let box_dim = box_count_dimension(&img, BANDS, s.frames);
    SoundAnalysis {
        silent: false,
        loudness,
        env_beta,
        centroid_beta,
        env_higuchi: higuchi_fd(&env_db, 16),
        box_dim,
        score: fractal_score(false, env_beta, centroid_beta, box_dim),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::rng::{Mulberry32, Rng};

    fn noise(n: usize, beta: f64) -> Vec<f64> {
        // 1/f^β series by summing octave-spaced sines with the right weights —
        // enough to check the slope estimator's sign and rough scale.
        let mut rng = Mulberry32::new(9);
        let mut out = vec![0.0; n];
        for octave in 0..10 {
            let f = 2f64.powi(octave) / n as f64 * 8.0;
            let amp = 1.0 / (2f64.powi(octave)).powf(beta / 2.0);
            let phase = rng.next() * std::f64::consts::TAU;
            for (i, v) in out.iter_mut().enumerate() {
                *v += amp * (std::f64::consts::TAU * f * i as f64 + phase).sin();
            }
        }
        out
    }

    #[test]
    fn the_slope_tells_white_from_pink_from_brown() {
        let white = spectral_slope(&noise(4096, 0.0), 1.0);
        let pink = spectral_slope(&noise(4096, 1.0), 1.0);
        let brown = spectral_slope(&noise(4096, 2.0), 1.0);
        println!("β estimates: white {white:.2}, pink {pink:.2}, brown {brown:.2}");
        assert!(white < pink && pink < brown, "{white} {pink} {brown}");
    }

    #[test]
    fn higuchi_separates_a_smooth_curve_from_noise() {
        let mut rng = Mulberry32::new(4);
        let smooth: Vec<f64> = (0..2048).map(|i| (i as f64 * 0.01).sin()).collect();
        let rough: Vec<f64> = (0..2048).map(|_| rng.next()).collect();
        let (a, b) = (higuchi_fd(&smooth, 16), higuchi_fd(&rough, 16));
        assert!(a < 1.3, "smooth curve: {a}");
        assert!(b > 1.7, "noise: {b}");
    }

    #[test]
    fn box_counting_is_between_a_line_and_a_filled_square() {
        let (w, h) = (64, 64);
        let mut line = vec![0u8; w * h];
        for i in 0..64 {
            line[i * w + i] = 1;
        }
        let full = vec![1u8; w * h];
        let empty = vec![0u8; w * h];
        let (l, f) = (box_count_dimension(&line, w, h), box_count_dimension(&full, w, h));
        assert!((l - 1.0).abs() < 0.2, "a line should be ~1: {l}");
        assert!((f - 2.0).abs() < 0.2, "a filled square should be ~2: {f}");
        assert_eq!(box_count_dimension(&empty, w, h), 0.0);
    }

    #[test]
    fn silence_scores_zero_and_says_so() {
        let a = analyze_sound(&vec![0.0f32; 48000], 48000.0);
        assert!(a.silent);
        assert_eq!(a.score, 0.0);
    }

    #[test]
    fn a_steady_tone_is_not_fractal() {
        let sr = 22050.0;
        let tone: Vec<f32> = (0..(sr as usize * 4))
            .map(|i| (0.3 * (std::f64::consts::TAU * 220.0 * i as f64 / sr).sin()) as f32)
            .collect();
        let a = analyze_sound(&tone, sr);
        assert!(!a.silent);
        assert!(a.score < 0.35, "a steady tone scored {}", a.score);
    }
}
