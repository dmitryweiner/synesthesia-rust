//! Field → colour, `display.frag` per pixel of the picture: ripples displace
//! the lookup, a cosine gradient colours the height, bass/mid/treble tint the
//! dark/mid/light tones, a bump-mapped relief lights it, the onset flash and
//! the swell's exposure act on the result.
//!
//! The picture is coarser than the grid (GRAPHICS.md decision 3), and a
//! bilinear lookup at a pixel's centre on a grid twice as fine is exactly the
//! 2×2 box average — the down-filter comes for free, as it does in the web
//! app when its canvas is smaller than its grid.
//!
//! Y grows downwards here; the relief is computed in the web's Y-up terms
//! (`hD` is the row *below* on screen), so the light falls from the same
//! side of the screen as in the browser.

use super::coupling::{DisplayFx, Ripple, RIPPLE_LIFE};
use super::field::Field;
use super::fields::taps;
use super::palette::Palette;

const RIPPLE_SPEED: f32 = 0.35; // UV (height units) per second
const RIPPLE_WIDTH: f32 = 0.035;
const RIPPLE_PUSH: f32 = 0.012; // largest displacement, UV
const TINT_LOW: [f32; 3] = [0.85, 0.18, 0.30];
const TINT_MID: [f32; 3] = [1.0, 0.72, 0.25];
const TINT_HIGH: [f32; 3] = [0.55, 0.90, 1.0];
const LIGHT_Z: f32 = 0.6;

/// An RGB picture, row 0 at the top.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<[u8; 3]>,
}

impl Image {
    pub fn new(w: usize, h: usize) -> Self {
        Self { w, h, rgb: vec![[0; 3]; w * h] }
    }

    pub fn at(&self, x: usize, y: usize) -> [u8; 3] {
        self.rgb[y * self.w + x]
    }
}

#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// `v` at UV `(x, y)`, bilinear, clamped at the border.
#[inline]
fn height(field: &Field, x: f32, y: f32) -> f32 {
    let (i0, i1, fx) = taps(x, field.w);
    let (j0, j1, fy) = taps(y, field.h);
    let v = &field.v;
    let w = field.w;
    let top = v[j0 * w + i0] + (v[j0 * w + i1] - v[j0 * w + i0]) * fx;
    let bot = v[j1 * w + i0] + (v[j1 * w + i1] - v[j1 * w + i0]) * fx;
    top + (bot - top) * fy
}

/// The light vector for `angle` (`lightDir` in `engine.ts`), Y up.
pub fn light_dir(angle: f32) -> [f32; 3] {
    let (x, y) = (angle.cos(), angle.sin());
    let len = (x * x + y * y + LIGHT_Z * LIGHT_Z).sqrt();
    [x / len, y / len, LIGHT_Z / len]
}

/// Draws `field` into `img` at the image's own size.
pub fn render(field: &Field, pal: &Palette, fx: &DisplayFx, ripples: &[Ripple], img: &mut Image) {
    let aspect = field.aspect();
    let (tx, ty) = (1.0 / field.w as f32, 1.0 / field.h as f32);
    let light = light_dir(pal.light_angle);
    let spec_gain = pal.gloss + fx.flash * 1.5;
    let glow = fx.flash * 0.18;
    let live: Vec<&Ripple> = ripples.iter().filter(|r| r.amp > 0.0).collect();
    for py in 0..img.h {
        let v0 = (py as f32 + 0.5) / img.h as f32;
        for px in 0..img.w {
            let u0 = (px as f32 + 0.5) / img.w as f32;

            // ripples: radial displacement of the lookup + a bright crest
            let (mut u, mut v) = (u0, v0);
            let mut crest = 0.0;
            for r in &live {
                let (dx, dy) = ((u0 - r.x) * aspect, v0 - r.y);
                let dist = dx.hypot(dy);
                let g = (dist - r.age * RIPPLE_SPEED) / RIPPLE_WIDTH;
                let wave = (-g * g).exp() * r.amp * (1.0 - r.age / RIPPLE_LIFE as f32);
                if dist > 1e-5 {
                    u -= dx / dist / aspect * wave * RIPPLE_PUSH;
                    v -= dy / dist * wave * RIPPLE_PUSH;
                }
                crest += wave;
            }

            let raw = (height(field, u, v) * 1.6).clamp(0.0, 1.0);
            let t = (raw * pal.bands).fract();
            let mut col = std::array::from_fn(|k| {
                pal.a[k] + pal.b[k] * (std::f32::consts::TAU * (pal.c[k] * t + pal.d[k])).cos()
            });

            // spectrum tint by tone: bass → dark, mids → middle, treble → light
            let w_low = 1.0 - smoothstep(0.0, 0.45, raw);
            let w_high = smoothstep(0.55, 1.0, raw);
            let w_mid = (1.0 - w_low - w_high).clamp(0.0, 1.0);
            col = mix3(col, TINT_LOW, fx.tint[0] * w_low * 0.6);
            col = mix3(col, TINT_MID, fx.tint[1] * w_mid * 0.6);
            col = mix3(col, TINT_HIGH, fx.tint[2] * w_high * 0.6);

            // relief: the height's gradient as a normal, Y up
            let h_l = height(field, u - tx, v);
            let h_r = height(field, u + tx, v);
            let h_d = height(field, u, v + ty);
            let h_u = height(field, u, v - ty);
            let (nx, ny) = ((h_l - h_r) * pal.relief, (h_d - h_u) * pal.relief);
            let inv = 1.0 / (nx * nx + ny * ny + 1.0).sqrt();
            let n = [nx * inv, ny * inv, inv];
            let n_dot_l = n[0] * light[0] + n[1] * light[1] + n[2] * light[2];
            let diffuse = n_dot_l.max(0.0);
            // reflect(-L, N).z
            let z = (2.0 * n_dot_l * n[2] - light[2]).max(0.0);
            let z2 = z * z;
            let z4 = z2 * z2;
            let z8 = z4 * z4;
            let specular = z8 * z8 * z8 * spec_gain;

            let shade = 0.55 + 0.55 * diffuse;
            let add = specular + glow + crest * 0.22;
            let out = col.map(|c| ((c * shade + add) * fx.exposure).clamp(0.0, 1.0));
            img.rgb[py * img.w + px] = out.map(|c| (c * 255.0).round() as u8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::palette::BUILTIN_PALETTES;

    fn flat(v: f32) -> Field {
        let mut f = Field::blank(40, 20);
        f.v.fill(v);
        f
    }

    fn marble() -> Palette {
        Palette::compose(&BUILTIN_PALETTES[0], 0.0, 1.0, 1.0, 0.8, 2.0, 0.3)
    }

    fn draw(field: &Field, fx: &DisplayFx, ripples: &[Ripple]) -> Image {
        let mut img = Image::new(20, 10);
        render(field, &marble(), fx, ripples, &mut img);
        img
    }

    #[test]
    fn a_flat_field_is_one_colour_computed_as_the_shader_would() {
        let img = draw(&flat(0.3), &DisplayFx::NEUTRAL, &[]);
        assert!(img.rgb.iter().all(|&c| c == img.rgb[0]));
        // By hand: t = 0.48, a flat normal (0, 0, 1), so diffuse = L.z and the
        // reflection's z is L.z too.
        let p = marble();
        let l = light_dir(2.0);
        let t = 0.3f32 * 1.6;
        let spec = l[2].powi(24) * p.gloss;
        let expected: Vec<u8> = (0..3)
            .map(|k| {
                let c = p.a[k] + p.b[k] * (std::f32::consts::TAU * (p.c[k] * t + p.d[k])).cos();
                ((c * (0.55 + 0.55 * l[2]) + spec).clamp(0.0, 1.0) * 255.0).round() as u8
            })
            .collect();
        assert_eq!(img.rgb[0].to_vec(), expected);
    }

    #[test]
    fn exposure_and_flash_brighten_and_tint_colours_the_tones() {
        let field = flat(0.3);
        let base = draw(&field, &DisplayFx::NEUTRAL, &[]).rgb[0];
        let sum = |c: [u8; 3]| c.iter().map(|&x| u32::from(x)).sum::<u32>();
        let brighter = draw(&field, &DisplayFx { exposure: 1.3, ..DisplayFx::NEUTRAL }, &[]).rgb[0];
        let flashed = draw(&field, &DisplayFx { flash: 1.0, ..DisplayFx::NEUTRAL }, &[]).rgb[0];
        assert!(sum(brighter) > sum(base) && sum(flashed) > sum(base));
        // v = 0 is the darkest tone: only the bass tint reaches it.
        let dark = flat(0.0);
        let plain = draw(&dark, &DisplayFx::NEUTRAL, &[]).rgb[0];
        let treble = draw(&dark, &DisplayFx { tint: [0.0, 0.0, 1.0], ..DisplayFx::NEUTRAL }, &[]).rgb[0];
        let bass = draw(&dark, &DisplayFx { tint: [1.0, 0.0, 0.0], ..DisplayFx::NEUTRAL }, &[]).rgb[0];
        assert_eq!(plain, treble);
        assert_ne!(plain, bass);
    }

    #[test]
    fn a_ripple_brightens_its_ring_and_is_gone_at_the_end_of_its_life() {
        let field = flat(0.3);
        let quiet = draw(&field, &DisplayFx::NEUTRAL, &[]);
        let ring = Ripple { x: 0.5, y: 0.5, age: 0.2, amp: 1.0 };
        let img = draw(&field, &DisplayFx::NEUTRAL, &[ring]);
        // At 0.2 s the crest is 0.07 height units out — within a pixel or two
        // of the centre on a 10-row picture; the corners are far outside it.
        assert_ne!(img, quiet);
        assert_eq!(img.at(0, 0), quiet.at(0, 0), "far corners are untouched");
        let dead = Ripple { age: RIPPLE_LIFE as f32, ..ring };
        assert_eq!(draw(&field, &DisplayFx::NEUTRAL, &[dead]), quiet);
    }

    #[test]
    fn the_relief_is_lit_from_the_same_side_as_in_the_browser() {
        // A ramp rising to the right faces left: light from the left (angle
        // π) falls on it, light from the right grazes its back.
        let mut ramp = Field::blank(40, 20);
        for y in 0..20 {
            for x in 0..40 {
                ramp.v[y * 40 + x] = x as f32 / 400.0;
            }
        }
        let mut pal = marble();
        pal.relief = 3.0;
        pal.bands = 0.0; // one colour whatever the height
        let mut lit = Image::new(20, 10);
        let (from_left, from_right) = (std::f32::consts::PI, 0.0);
        pal.light_angle = from_left;
        render(&ramp, &pal, &DisplayFx::NEUTRAL, &[], &mut lit);
        let mut other = Image::new(20, 10);
        pal.light_angle = from_right;
        render(&ramp, &pal, &DisplayFx::NEUTRAL, &[], &mut other);
        let sum = |c: [u8; 3]| c.iter().map(|&x| u32::from(x)).sum::<u32>();
        assert!(sum(lit.at(10, 5)) > sum(other.at(10, 5)));

        // And "up" is up on screen: a ramp rising towards the bottom faces the
        // top, so light from the top (angle π/2, as in the browser) falls on it.
        let mut down = Field::blank(40, 20);
        for y in 0..20 {
            for x in 0..40 {
                down.v[y * 40 + x] = y as f32 / 200.0;
            }
        }
        pal.light_angle = std::f32::consts::FRAC_PI_2; // from the top
        render(&down, &pal, &DisplayFx::NEUTRAL, &[], &mut lit);
        pal.light_angle = -std::f32::consts::FRAC_PI_2; // from the bottom
        render(&down, &pal, &DisplayFx::NEUTRAL, &[], &mut other);
        assert!(sum(lit.at(10, 5)) > sum(other.at(10, 5)));
    }
}
