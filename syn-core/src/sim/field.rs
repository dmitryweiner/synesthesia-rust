//! The Gray–Scott state and the three passes that write it directly:
//! `seed.frag`, `inject.frag` and `react.frag`.
//!
//! Storage is row-major with row 0 at the *top* (the web's textures are
//! Y-up). Nothing here depends on the orientation except the drift, which
//! the caller passes already pointing down (see `fields.rs`).

use super::fields::taps;
use crate::dsp::rng::{Mulberry32, Rng};

const MAX_SPOTS: usize = 24;

/// GLSL `smoothstep`, including the reversed-edges form the shaders use to
/// draw a disc that is 1 inside and fades out.
#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Clone, Debug)]
pub struct Field {
    pub(crate) w: usize,
    pub(crate) h: usize,
    pub(crate) u: Vec<f32>,
    pub(crate) v: Vec<f32>,
    nu: Vec<f32>,
    nv: Vec<f32>,
}

impl Field {
    /// `u = 1, v = 0` everywhere: the substrate with nothing growing in it,
    /// a fixed point of the reaction.
    pub fn blank(w: usize, h: usize) -> Self {
        assert!(w >= 3 && h >= 3, "a {w}x{h} grid has no interior");
        let n = w * h;
        Self { w, h, u: vec![1.0; n], v: vec![0.0; n], nu: vec![0.0; n], nv: vec![0.0; n] }
    }

    pub fn width(&self) -> usize {
        self.w
    }

    pub fn height(&self) -> usize {
        self.h
    }

    pub fn u(&self) -> &[f32] {
        &self.u
    }

    pub fn v(&self) -> &[f32] {
        &self.v
    }

    pub fn aspect(&self) -> f32 {
        self.w as f32 / self.h as f32
    }

    /// A fresh random start (`SimEngine.reseed`): 19–24 spots of radius
    /// 0.02–0.05 in height units, each a disc of `v` in a dip of `u`.
    pub fn seed(&mut self, rng: &mut Mulberry32) {
        let count = MAX_SPOTS - (rng.next() * 6.0).floor() as usize;
        let spots: Vec<(f32, f32)> = (0..count).map(|_| (rng.next() as f32, rng.next() as f32)).collect();
        let radius = (0.02 + rng.next() * 0.03) as f32;
        let aspect = self.aspect();
        for y in 0..self.h {
            let vy = (y as f32 + 0.5) / self.h as f32;
            for x in 0..self.w {
                let vx = (x as f32 + 0.5) / self.w as f32;
                let (mut u, mut v) = (1.0f32, 0.0f32);
                for &(sx, sy) in &spots {
                    let d = ((vx - sx) * aspect).hypot(vy - sy);
                    let blob = smoothstep(radius, radius * 0.2, d);
                    v = v.max(blob * 0.5);
                    u = u + (0.5 - u) * blob;
                }
                self.u[y * self.w + x] = u;
                self.v[y * self.w + x] = v;
            }
        }
    }

    /// The same pattern on a `w`×`h` grid, resampled bilinearly — a
    /// resized terminal keeps its picture instead of starting over.
    pub fn resampled(&self, w: usize, h: usize) -> Self {
        let mut out = Self::blank(w, h);
        for y in 0..h {
            let (j0, j1, fy) = taps((y as f32 + 0.5) / h as f32, self.h);
            for x in 0..w {
                let (i0, i1, fx) = taps((x as f32 + 0.5) / w as f32, self.w);
                let lerp2 = |c: &[f32]| {
                    let top = c[j0 * self.w + i0] + (c[j0 * self.w + i1] - c[j0 * self.w + i0]) * fx;
                    let bot = c[j1 * self.w + i0] + (c[j1 * self.w + i1] - c[j1 * self.w + i0]) * fx;
                    top + (bot - top) * fy
                };
                out.u[y * w + x] = lerp2(&self.u);
                out.v[y * w + x] = lerp2(&self.v);
            }
        }
        out
    }

    /// Fresh "ink" in a disc at `(cx, cy)` (UV, 0..1), `radius` in height
    /// units, `amount` 0..1 of the disc converted (`inject.frag`).
    pub fn inject(&mut self, cx: f32, cy: f32, radius: f32, amount: f32) {
        let aspect = self.aspect();
        // Only the disc's bounding box can change.
        let (rx, ry) = (radius / aspect, radius);
        let x0 = (((cx - rx) * self.w as f32).floor().max(0.0)) as usize;
        let x1 = (((cx + rx) * self.w as f32).ceil() as usize).min(self.w);
        let y0 = (((cy - ry) * self.h as f32).floor().max(0.0)) as usize;
        let y1 = (((cy + ry) * self.h as f32).ceil() as usize).min(self.h);
        for y in y0..y1 {
            let vy = (y as f32 + 0.5) / self.h as f32;
            for x in x0..x1 {
                let vx = (x as f32 + 0.5) / self.w as f32;
                let d = ((vx - cx) * aspect).hypot(vy - cy);
                let blob = smoothstep(radius, radius * 0.3, d) * amount;
                let i = y * self.w + x;
                self.v[i] = self.v[i].max(blob * 0.5);
                self.u[i] += (0.5 - self.u[i]) * blob;
            }
        }
    }

    /// One Gray–Scott substep with `dt = 1`, a 9-point weighted Laplacian
    /// and a zero-flux border (`react.frag`).
    pub fn react(&mut self, rates: &Rates, diff_u: f32, diff_v: f32) {
        let (w, h) = (self.w, self.h);
        assert!(rates.feed_off.len() == w * h && rates.kill_off.len() == w * h);
        for y in 0..h {
            let row = y * w..(y + 1) * w;
            if y == 0 || y == h - 1 {
                for x in 0..w {
                    self.edge_cell(x, y, rates, diff_u, diff_v);
                }
                continue;
            }
            self.edge_cell(0, y, rates, diff_u, diff_v);
            self.edge_cell(w - 1, y, rates, diff_u, diff_v);
            let up = (y - 1) * w..y * w;
            let dn = (y + 1) * w..(y + 2) * w;
            interior_row(
                Rows { up: &self.u[up.clone()], mid: &self.u[row.clone()], dn: &self.u[dn.clone()] },
                Rows { up: &self.v[up], mid: &self.v[row.clone()], dn: &self.v[dn] },
                RowRates {
                    feed: rates.feed,
                    kill: rates.kill,
                    feed_off: &rates.feed_off[row.clone()],
                    kill_off: &rates.kill_off[row.clone()],
                },
                (diff_u, diff_v),
                &mut self.nu[row.clone()],
                &mut self.nv[row],
            );
        }
        std::mem::swap(&mut self.u, &mut self.nu);
        std::mem::swap(&mut self.v, &mut self.nv);
    }

    /// The same update with every tap clamped into the grid — the border.
    fn edge_cell(&mut self, x: usize, y: usize, rates: &Rates, diff_u: f32, diff_v: f32) {
        let (w, h) = (self.w as isize, self.h as isize);
        let at = |dx: isize, dy: isize| {
            let cx = (x as isize + dx).clamp(0, w - 1);
            let cy = (y as isize + dy).clamp(0, h - 1);
            (cy * w + cx) as usize
        };
        let lap = |f: &[f32]| {
            -f[at(0, 0)]
                + 0.05 * (f[at(-1, -1)] + f[at(1, -1)] + f[at(-1, 1)] + f[at(1, 1)])
                + 0.2 * (f[at(0, -1)] + f[at(-1, 0)] + f[at(1, 0)] + f[at(0, 1)])
        };
        let i = at(0, 0);
        let fk = combine(rates.feed, rates.kill, rates.feed_off[i], rates.kill_off[i]);
        let (nu, nv) = update(self.u[i], self.v[i], lap(&self.u), lap(&self.v), fk, (diff_u, diff_v));
        self.nu[i] = nu;
        self.nv[i] = nv;
    }
}

/// Feed and kill: the card's values plus the Field variation's offset per
/// cell (all zeros when the card is off). The sum is clamped per cell, as
/// `react.frag` does per fragment — so an LFO sweeping feed changes two
/// numbers and not two maps.
pub struct Rates<'a> {
    pub feed: f32,
    pub kill: f32,
    pub feed_off: &'a [f32],
    pub kill_off: &'a [f32],
}

/// [`Rates`] cut down to one row.
struct RowRates<'a> {
    feed: f32,
    kill: f32,
    feed_off: &'a [f32],
    kill_off: &'a [f32],
}

struct Rows<'a> {
    up: &'a [f32],
    mid: &'a [f32],
    dn: &'a [f32],
}

impl Rows<'_> {
    #[inline(always)]
    fn lap(&self, x: usize) -> f32 {
        let corners = (self.up[x - 1] + self.up[x + 1]) + (self.dn[x - 1] + self.dn[x + 1]);
        let sides = (self.up[x] + self.dn[x]) + (self.mid[x - 1] + self.mid[x + 1]);
        corners.mul_add(0.05, sides.mul_add(0.2, -self.mid[x]))
    }
}

#[inline(always)]
fn combine(feed: f32, kill: f32, df: f32, dk: f32) -> (f32, f32) {
    ((feed + df).clamp(0.0, 1.0), (kill + dk).clamp(0.0, 1.0))
}

#[inline(always)]
fn update(
    u: f32,
    v: f32,
    lap_u: f32,
    lap_v: f32,
    (feed, kill): (f32, f32),
    (diff_u, diff_v): (f32, f32),
) -> (f32, f32) {
    // Fused multiply-adds: one `fmla` each on aarch64, which Rust never
    // contracts `a * b + c` into by itself.
    let r = u * v * v;
    let du = diff_u.mul_add(lap_u, feed.mul_add(1.0 - u, -r));
    let dv = diff_v.mul_add(lap_v, (-(feed + kill)).mul_add(v, r));
    ((u + du).clamp(0.0, 1.0), (v + dv).clamp(0.0, 1.0))
}

/// Columns 1..w-1 of one interior row. Every slice is exactly one row long,
/// so after the asserts the compiler can drop the bounds checks and
/// vectorize the loop.
fn interior_row(u: Rows, v: Rows, r: RowRates, diff: (f32, f32), nu: &mut [f32], nv: &mut [f32]) {
    let w = u.mid.len();
    assert!(u.up.len() == w && u.dn.len() == w && v.up.len() == w && v.mid.len() == w && v.dn.len() == w);
    assert!(r.feed_off.len() == w && r.kill_off.len() == w && nu.len() == w && nv.len() == w);
    for x in 1..w - 1 {
        let fk = combine(r.feed, r.kill, r.feed_off[x], r.kill_off[x]);
        let (a, b) = update(u.mid[x], v.mid[x], u.lap(x), v.lap(x), fk, diff);
        nu[x] = a;
        nv[x] = b;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zeros(f: &Field) -> Vec<f32> {
        vec![0.0; f.w * f.h]
    }

    fn default_rates(zero: &[f32]) -> Rates<'_> {
        Rates { feed: 0.037, kill: 0.06, feed_off: zero, kill_off: zero }
    }

    #[test]
    fn the_blank_substrate_is_a_fixed_point() {
        let mut f = Field::blank(17, 11);
        let zero = zeros(&f);
        for _ in 0..100 {
            f.react(&default_rates(&zero), 0.2097, 0.105);
        }
        assert!(f.u.iter().all(|&u| u == 1.0));
        assert!(f.v.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn the_interior_kernel_agrees_with_the_clamped_one() {
        let mut rng = Mulberry32::new(7);
        let mut a = Field::blank(40, 30);
        a.seed(&mut rng);
        let mut b = a.clone();
        let zero = zeros(&a);
        a.react(&default_rates(&zero), 0.2097, 0.105);
        for y in 0..b.h {
            for x in 0..b.w {
                b.edge_cell(x, y, &default_rates(&zero), 0.2097, 0.105);
            }
        }
        std::mem::swap(&mut b.u, &mut b.nu);
        std::mem::swap(&mut b.v, &mut b.nv);
        for i in 0..a.u.len() {
            assert!((a.u[i] - b.u[i]).abs() < 1e-6 && (a.v[i] - b.v[i]).abs() < 1e-6, "cell {i}");
        }
    }

    #[test]
    fn seeding_places_spots_and_is_reproducible() {
        let (mut a, mut b) = (Field::blank(64, 40), Field::blank(64, 40));
        a.seed(&mut Mulberry32::new(3));
        b.seed(&mut Mulberry32::new(3));
        assert_eq!(a.v, b.v);
        let lit = a.v.iter().filter(|&&v| v > 0.25).count();
        assert!(lit > 20, "only {lit} cells seeded");
        assert!(a.v.iter().all(|&v| (0.0..=0.5).contains(&v)));
    }

    #[test]
    fn resampling_keeps_the_pattern() {
        let mut a = Field::blank(40, 20);
        a.seed(&mut Mulberry32::new(8));
        let same = a.resampled(40, 20);
        assert!(a.v.iter().zip(&same.v).all(|(x, y)| (x - y).abs() < 1e-6), "identity");
        let big = a.resampled(80, 40);
        // Each cell of the original is the mean of the 2x2 it became.
        let mean = |x: usize, y: usize| {
            (big.v[2 * y * 80 + 2 * x]
                + big.v[2 * y * 80 + 2 * x + 1]
                + big.v[(2 * y + 1) * 80 + 2 * x]
                + big.v[(2 * y + 1) * 80 + 2 * x + 1])
                / 4.0
        };
        assert!((mean(20, 10) - a.v[10 * 40 + 20]).abs() < 0.05);
    }

    #[test]
    fn inject_draws_a_round_disc_where_it_is_asked_to() {
        let mut f = Field::blank(80, 40);
        f.inject(0.5, 0.5, 0.1, 1.0);
        let at = |x: usize, y: usize| f.v[y * f.w + x];
        assert_eq!(at(40, 20), 0.5);
        assert_eq!(at(5, 5), 0.0);
        // Round on screen: the grid is twice as wide as tall, so a row and a
        // column both span 1/40 of the height, and three rows up from the
        // centre cell lands as far out as three columns left of it.
        assert!(at(40, 17) > 0.0 && (at(40, 17) - at(37, 20)).abs() < 1e-6);
    }
}
