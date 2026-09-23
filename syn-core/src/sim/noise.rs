//! Hash, value noise, fbm, domain warp and curl — `src/sim/shaders/common.glsl`
//! in `f32`, with GLSL's `fract`/`mix` semantics, so the fields have the same
//! character as the web app's. Not bit-exact with a GPU (nothing is), and it
//! does not need to be: these only perturb feed/kill and steer the flow.

#[inline]
fn fract(x: f32) -> f32 {
    x - x.floor()
}

#[inline]
fn mix(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

#[inline]
pub fn hash21(px: f32, py: f32) -> f32 {
    let mut x = fract(px * 123.34);
    let mut y = fract(py * 456.21);
    let d = x * (x + 45.32) + y * (y + 45.32);
    x += d;
    y += d;
    fract(x * y)
}

#[inline]
pub fn noise2(px: f32, py: f32) -> f32 {
    let (ix, iy) = (px.floor(), py.floor());
    let (fx, fy) = (px - ix, py - iy);
    let a = hash21(ix, iy);
    let b = hash21(ix + 1.0, iy);
    let c = hash21(ix, iy + 1.0);
    let d = hash21(ix + 1.0, iy + 1.0);
    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uy = fy * fy * (3.0 - 2.0 * fy);
    mix(mix(a, b, ux), mix(c, d, ux), uy)
}

pub fn fbm(px: f32, py: f32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut freq) = (0.0, 0.5, 1.0);
    for _ in 0..octaves.min(8) {
        sum += amp * noise2(px * freq, py * freq);
        freq *= 2.0;
        amp *= 0.5;
    }
    sum
}

/// `fbm(p + warp·fbm2(p))` (Quilez) — the "folded layers" of Field variation.
pub fn warped_fbm(px: f32, py: f32, warp: f32, octaves: u32) -> f32 {
    let qx = fbm(px, py, octaves);
    let qy = fbm(px + 5.2, py + 1.3, octaves);
    fbm(px + warp * qx, py + warp * qy, octaves)
}

/// 2D curl of an fbm potential by central differences: divergence-free, so
/// advected material neither bunches up nor evaporates.
pub fn curl(px: f32, py: f32, eps: f32) -> (f32, f32) {
    let n1 = fbm(px, py + eps, 4);
    let n2 = fbm(px, py - eps, 4);
    let n3 = fbm(px + eps, py, 4);
    let n4 = fbm(px - eps, py, 4);
    let dx = (n3 - n4) / (2.0 * eps);
    let dy = (n1 - n2) / (2.0 * eps);
    (dy, -dx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_noise_stay_in_the_unit_interval() {
        for i in 0..2000 {
            let (x, y) = (i as f32 * 0.731 - 300.0, i as f32 * 0.377 + 11.0);
            let h = hash21(x, y);
            let n = noise2(x, y);
            assert!((0.0..1.0).contains(&h), "hash {h} at {x},{y}");
            assert!((0.0..=1.0).contains(&n), "noise {n} at {x},{y}");
        }
    }

    #[test]
    fn noise_is_continuous_across_lattice_lines() {
        for i in 0..50 {
            let y = i as f32 * 0.37;
            let a = noise2(3.0 - 1e-4, y);
            let b = noise2(3.0 + 1e-4, y);
            assert!((a - b).abs() < 1e-2, "jump {a} → {b}");
        }
    }

    #[test]
    fn fbm_of_four_octaves_is_bounded_by_its_amplitudes() {
        for i in 0..500 {
            let f = fbm(i as f32 * 0.13, i as f32 * 0.07, 4);
            assert!((0.0..=0.9375).contains(&f));
        }
    }

    #[test]
    fn the_curl_field_has_no_divergence() {
        // Measured with the same central differences the curl is built from,
        // the divergence cancels term by term — what is left is rounding.
        let e = 0.05;
        for i in 0..40 {
            let (x, y) = (0.3 + i as f32 * 0.21, 0.9 + i as f32 * 0.13);
            let dvx = (curl(x + e, y, e).0 - curl(x - e, y, e).0) / (2.0 * e);
            let dvy = (curl(x, y + e, e).1 - curl(x, y - e, e).1) / (2.0 * e);
            assert!((dvx + dvy).abs() < 1e-2, "divergence {} at {x},{y}", dvx + dvy);
        }
    }
}
