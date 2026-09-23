//! Semi-Lagrangian advection (`advect.frag`): every cell takes the state
//! from where it came from this step, bilinearly, clamped at the border.
//!
//! The shader samples the half-resolution velocity texture per fragment; here
//! that sample is taken once per velocity redraw, into a full-resolution map
//! in cells per step ([`velocity_map`]), which is most of what a cell costs.

use super::field::Field;
use super::fields::Texture2;

/// The velocity at every cell centre, converted from UV to cells.
pub fn velocity_map(vel: &Texture2, w: usize, h: usize, out: &mut (Vec<f32>, Vec<f32>)) {
    out.0.resize(w * h, 0.0);
    out.1.resize(w * h, 0.0);
    for y in 0..h {
        let vy = (y as f32 + 0.5) / h as f32;
        for x in 0..w {
            let (dx, dy) = vel.sample((x as f32 + 0.5) / w as f32, vy);
            out.0[y * w + x] = dx * w as f32;
            out.1[y * w + x] = dy * h as f32;
        }
    }
}

/// Moves `field` along `vel` (cells per step, from [`velocity_map`]) by
/// `amount` steps' worth.
pub fn advect(
    field: &mut Field,
    vel: &(Vec<f32>, Vec<f32>),
    amount: f32,
    scratch: &mut (Vec<f32>, Vec<f32>),
) {
    let (w, h) = (field.w, field.h);
    assert!(vel.0.len() == w * h && vel.1.len() == w * h);
    let (nu, nv) = scratch;
    nu.resize(w * h, 0.0);
    nv.resize(w * h, 0.0);
    let (xmax, ymax) = ((w - 1) as f32, (h - 1) as f32);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            // A texel centre sits at an integer here, so the source point in
            // texel space is just the cell minus its displacement; clamping
            // it is CLAMP_TO_EDGE.
            let px = (x as f32 - vel.0[i] * amount).clamp(0.0, xmax);
            let py = (y as f32 - vel.1[i] * amount).clamp(0.0, ymax);
            let (x0, y0) = (px as usize, py as usize);
            let (fx, fy) = (px - x0 as f32, py - y0 as f32);
            let x1 = (x0 + 1).min(w - 1);
            let y1 = (y0 + 1).min(h - 1);
            let (a, b, c, d) = (y0 * w + x0, y0 * w + x1, y1 * w + x0, y1 * w + x1);
            let lerp2 = |s: &[f32]| {
                let top = s[a] + (s[b] - s[a]) * fx;
                let bot = s[c] + (s[d] - s[c]) * fx;
                top + (bot - top) * fy
            };
            nu[i] = lerp2(&field.u);
            nv[i] = lerp2(&field.v);
        }
    }
    std::mem::swap(&mut field.u, nu);
    std::mem::swap(&mut field.v, nv);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::rng::Mulberry32;

    #[test]
    fn a_whole_cell_of_drift_moves_the_field_by_one_cell() {
        let mut f = Field::blank(40, 20);
        f.seed(&mut Mulberry32::new(5));
        let before = f.clone();
        let mut tex = Texture2::new(20, 10);
        tex.a.fill(1.0 / 40.0); // one column per step, to the right
        let mut vel = Default::default();
        velocity_map(&tex, 40, 20, &mut vel);
        advect(&mut f, &vel, 1.0, &mut Default::default());
        for y in 0..20 {
            for x in 1..40 {
                let (now, was) = (f.v()[y * 40 + x], before.v()[y * 40 + x - 1]);
                assert!((now - was).abs() < 1e-5, "({x},{y}): {now} vs {was}");
            }
        }
    }

    #[test]
    fn no_velocity_is_an_exact_copy() {
        let mut f = Field::blank(30, 20);
        f.seed(&mut Mulberry32::new(2));
        let before = f.clone();
        let vel = (vec![0.0; 600], vec![0.0; 600]);
        advect(&mut f, &vel, 1.0, &mut Default::default());
        assert_eq!(f.u(), before.u());
        assert_eq!(f.v(), before.v());
    }
}
