//! The two smooth fields the reaction and the advection read: the feed/kill
//! offsets (`paramfield.frag`) and the velocity (`velocity.frag`).
//!
//! Both are pure functions of their params and `evolve_t`, both are smooth by
//! construction, and each cell costs ten fbm evaluations. So, like the web
//! engine after it learned this on a software rasterizer, they live at half
//! the grid's side, are sampled bilinearly, and are redrawn only when the
//! params they were drawn from change (`src/sim/engine.ts`, `grid.ts`).

use super::noise::{curl, warped_fbm};
use super::{FieldVariation, Flow};

const MIN_SIDE: usize = 32;

/// Half the grid's side, never below 32 cells (`fieldGridSize`).
pub fn half_size(w: usize, h: usize) -> (usize, usize) {
    (
        (w as f64 / 2.0).round().max(MIN_SIDE as f64) as usize,
        (h as f64 / 2.0).round().max(MIN_SIDE as f64) as usize,
    )
}

/// A two-channel texture with `LINEAR` filtering and `CLAMP_TO_EDGE`.
#[derive(Clone, Debug)]
pub struct Texture2 {
    pub w: usize,
    pub h: usize,
    pub a: Vec<f32>,
    pub b: Vec<f32>,
}

impl Texture2 {
    pub fn new(w: usize, h: usize) -> Self {
        Self { w, h, a: vec![0.0; w * h], b: vec![0.0; w * h] }
    }

    /// Bilinear sample at UV `(x, y)`, texel centers at `(i + 0.5) / w`.
    #[inline]
    pub fn sample(&self, x: f32, y: f32) -> (f32, f32) {
        let (i0, i1, fx) = taps(x, self.w);
        let (j0, j1, fy) = taps(y, self.h);
        let lerp2 = |c: &[f32]| {
            let top = c[j0 * self.w + i0] * (1.0 - fx) + c[j0 * self.w + i1] * fx;
            let bot = c[j1 * self.w + i0] * (1.0 - fx) + c[j1 * self.w + i1] * fx;
            top * (1.0 - fy) + bot * fy
        };
        (lerp2(&self.a), lerp2(&self.b))
    }

    fn fill(&mut self, f: impl Fn(f32, f32) -> (f32, f32)) {
        for y in 0..self.h {
            let vy = (y as f32 + 0.5) / self.h as f32;
            for x in 0..self.w {
                let (a, b) = f((x as f32 + 0.5) / self.w as f32, vy);
                self.a[y * self.w + x] = a;
                self.b[y * self.w + x] = b;
            }
        }
    }
}

/// The two texels either side of UV coordinate `t` on an axis of `n`
/// texels, and the weight of the second.
#[inline]
pub(crate) fn taps(t: f32, n: usize) -> (usize, usize, f32) {
    let p = t * n as f32 - 0.5;
    let p0 = p.floor();
    let f = p - p0;
    let last = n as isize - 1;
    let i0 = (p0 as isize).clamp(0, last) as usize;
    let i1 = (p0 as isize + 1).clamp(0, last) as usize;
    (i0, i1, f)
}

/// A texture plus the inputs it was last drawn from.
#[derive(Clone, Debug)]
pub struct Cached<const N: usize> {
    pub tex: Texture2,
    key: Option<[f64; N]>,
    /// How many times it has been drawn — what the tests watch.
    pub draws: u64,
}

impl<const N: usize> Cached<N> {
    pub fn new(w: usize, h: usize) -> Self {
        Self { tex: Texture2::new(w, h), key: None, draws: 0 }
    }

    /// Redraws with `draw` unless `key` is what it already holds. True when
    /// it drew.
    fn refresh(&mut self, key: [f64; N], draw: impl FnOnce(&mut Texture2)) -> bool {
        if self.key == Some(key) {
            return false;
        }
        draw(&mut self.tex);
        self.key = Some(key);
        self.draws += 1;
        true
    }
}

pub type ParamField = Cached<8>;
pub type Velocity = Cached<6>;

impl ParamField {
    /// Feed offset in `a`, kill offset in `b`. The caller skips this when the
    /// card does nothing ([`FieldVariation::active`]).
    pub fn update(&mut self, f: &FieldVariation, evolve_t: f64, aspect: f32) -> bool {
        let key = [
            f.feed_var_amount,
            f.feed_var_scale,
            f.feed_var_warp,
            f.kill_var_amount,
            f.kill_var_scale,
            f.kill_var_warp,
            evolve_t,
            f64::from(aspect),
        ];
        self.refresh(key, |tex| {
            let t = evolve_t as f32;
            let (fa, fs, fw) = (f.feed_var_amount as f32, f.feed_var_scale as f32, f.feed_var_warp as f32);
            let (ka, ks, kw) = (f.kill_var_amount as f32, f.kill_var_scale as f32, f.kill_var_warp as f32);
            tex.fill(|x, y| {
                let (x, y) = (x * aspect, y);
                let fnoise = warped_fbm(x * fs + t, y * fs + t, fw, 4) * 2.0 - 1.0;
                let knoise = warped_fbm(x * ks + t + 37.0, y * ks + t + 37.0, kw, 4) * 2.0 - 1.0;
                (fnoise * fa, knoise * ka)
            });
        })
    }
}

impl Velocity {
    /// Curl noise plus drift, in UV per step, `x` already divided by the
    /// aspect. Y grows downwards here, so a positive `drift_y` is "down" as it
    /// is on the slider — the web engine negates it for its Y-up textures.
    pub fn update(&mut self, flow: &Flow, evolve_t: f64, aspect: f32) -> bool {
        let key =
            [flow.curl_strength, flow.curl_scale, flow.drift_x, flow.drift_y, evolve_t, f64::from(aspect)];
        self.refresh(key, |tex| {
            let t = evolve_t as f32;
            let (strength, scale) = (flow.curl_strength as f32, flow.curl_scale as f32);
            let (dx, dy) = (flow.drift_x as f32, flow.drift_y as f32);
            tex.fill(|x, y| {
                let (cx, cy) = curl(x * aspect * scale + t, y * scale + t, 0.05);
                ((cx * strength + dx) / aspect, cy * strength + dy)
            });
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_size_halves_and_keeps_a_floor() {
        assert_eq!(half_size(240, 120), (120, 60));
        assert_eq!(half_size(40, 20), (32, 32));
    }

    #[test]
    fn bilinear_sampling_hits_texel_centers_and_clamps_outside() {
        let mut t = Texture2::new(4, 2);
        t.a = vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
        assert_eq!(t.sample(0.125, 0.25).0, 0.0);
        assert_eq!(t.sample(0.375, 0.75).0, 5.0);
        assert!((t.sample(0.25, 0.25).0 - 0.5).abs() < 1e-6);
        assert_eq!(t.sample(-3.0, -3.0).0, 0.0);
        assert_eq!(t.sample(9.0, 9.0).0, 7.0);
    }

    #[test]
    fn a_field_is_drawn_once_per_distinct_input() {
        let mut pf = ParamField::new(32, 32);
        let f = FieldVariation { feed_var_amount: 0.015, ..FieldVariation::ZERO };
        assert!(pf.update(&f, 0.0, 2.0));
        assert!(!pf.update(&f, 0.0, 2.0));
        assert!(pf.update(&f, 0.1, 2.0));
        assert_eq!(pf.draws, 2);
        let amp = pf.tex.a.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(amp > 0.0 && amp <= 0.015, "feed offsets reach {amp}");
        assert!(pf.tex.b.iter().all(|&v| v == 0.0), "kill amount 0 draws zeros");
    }

    #[test]
    fn pure_drift_is_uniform_and_points_down() {
        let mut vel = Velocity::new(32, 32);
        let flow = Flow { drift_y: 0.01, ..Flow::ZERO };
        vel.update(&flow, 0.0, 2.0);
        assert!(vel.tex.a.iter().all(|&v| v == 0.0));
        assert!(vel.tex.b.iter().all(|&v| v == 0.01));
    }
}
