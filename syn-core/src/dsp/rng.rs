//! Randomness for the generators and (later) for evolution.
//!
//! `mulberry32` is ported bit-for-bit from `../synesthesia/src/dsp/rng.ts`:
//! the golden takes were rendered with it, so any deviation shows up as a
//! failing test rather than as "noise that sounds a bit different".

/// A random source: `next()` yields `[0, 1)`.
pub trait Rng {
    fn next(&mut self) -> f64;
}

/// The JS `mulberry32`, including its `Math.imul` (wrapping 32-bit) steps.
#[derive(Clone, Debug)]
pub struct Mulberry32 {
    a: u32,
}

impl Mulberry32 {
    pub fn new(seed: u32) -> Self {
        Self { a: seed }
    }
}

impl Rng for Mulberry32 {
    fn next(&mut self) -> f64 {
        self.a = self.a.wrapping_add(0x6d2b_79f5);
        let a = self.a;
        let mut t = (a ^ (a >> 15)).wrapping_mul(a | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}

/// Standard normal sample (Box–Muller), guarding against `ln(0)`.
pub fn gaussian(rng: &mut impl Rng) -> f64 {
    let mut u = rng.next();
    while u <= 1e-12 {
        u = rng.next();
    }
    let v = rng.next();
    (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values taken from the TypeScript `mulberry32(12345)`.
    #[test]
    fn matches_the_typescript_stream() {
        let mut r = Mulberry32::new(12345);
        let got: Vec<f64> = (0..4).map(|_| r.next()).collect();
        for (g, w) in got.iter().zip([
            0.979_728_267_760_947_4,
            0.306_752_264_499_664_3,
            0.484_205_421_525_985,
            0.817_934_412_509_203,
        ]) {
            assert!((g - w).abs() < 1e-15, "{g} != {w}");
        }
    }
}
