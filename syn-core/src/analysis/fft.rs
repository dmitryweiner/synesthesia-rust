//! In-place iterative radix-2 complex FFT, ported from
//! `../synesthesia/src/analysis/fft.ts`. The inverse includes the 1/n
//! normalization, so forward-then-inverse is the identity.

/// Panics if the length is not a power of two, or the halves differ.
pub fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
    let n = re.len();
    assert!(n == im.len() && n > 0 && n.is_power_of_two(), "fft length must be a power of two (got {n})");

    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    let sign = if inverse { 1.0 } else { -1.0 };
    let mut len = 2;
    while len <= n {
        let ang = sign * std::f64::consts::TAU / len as f64;
        let (wi, wr) = ang.sin_cos();
        let half = len >> 1;
        let mut i = 0;
        while i < n {
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for k in 0..half {
                let a = i + k;
                let b = a + half;
                let xr = re[b] * cr - im[b] * ci;
                let xi = re[b] * ci + im[b] * cr;
                re[b] = re[a] - xr;
                im[b] = im[a] - xi;
                re[a] += xr;
                im[a] += xi;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
            i += len;
        }
        len <<= 1;
    }

    if inverse {
        let inv = 1.0 / n as f64;
        for i in 0..n {
            re[i] *= inv;
            im[i] *= inv;
        }
    }
}

pub fn next_pow2(n: usize) -> usize {
    n.next_power_of_two().max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sine_lands_in_one_bin() {
        let n = 1024;
        let bin = 37.0;
        let mut re: Vec<f64> =
            (0..n).map(|i| (std::f64::consts::TAU * bin * i as f64 / n as f64).sin()).collect();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im, false);
        let mags: Vec<f64> = (0..n / 2).map(|i| re[i].hypot(im[i])).collect();
        let peak = mags.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert_eq!(peak, bin as usize);
    }

    #[test]
    fn forward_then_inverse_is_the_identity() {
        let n = 256;
        let orig: Vec<f64> = (0..n).map(|i| ((i * 7 % 13) as f64 - 6.0) / 6.0).collect();
        let mut re = orig.clone();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im, false);
        fft(&mut re, &mut im, true);
        for (a, b) in re.iter().zip(&orig) {
            assert!((a - b).abs() < 1e-12, "{a} != {b}");
        }
    }
}
