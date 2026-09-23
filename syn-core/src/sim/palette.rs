//! Cosine palettes (Inigo Quilez's `a + b·cos(2π(c·t + d))`) — `src/palette.ts`.
//! The Palette card's shift and contrast are folded into the coefficients
//! once per frame, so the per-pixel code only evaluates the formula.

use crate::state::Params;

pub type Vec3 = [f32; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CosinePalette {
    pub name: &'static str,
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    pub d: Vec3,
}

/// Order matches the Palette card's `paletteId` select.
pub const BUILTIN_PALETTES: [CosinePalette; 5] = [
    CosinePalette {
        name: "Marble",
        a: [0.55, 0.5, 0.45],
        b: [0.45, 0.4, 0.35],
        c: [1.0, 1.0, 1.0],
        d: [0.0, 0.15, 0.3],
    },
    CosinePalette {
        name: "Glaze",
        a: [0.5, 0.35, 0.4],
        b: [0.5, 0.4, 0.4],
        c: [1.2, 1.0, 0.9],
        d: [0.3, 0.2, 0.15],
    },
    CosinePalette {
        name: "Verdigris",
        a: [0.35, 0.45, 0.42],
        b: [0.3, 0.35, 0.3],
        c: [1.0, 1.2, 1.0],
        d: [0.4, 0.5, 0.4],
    },
    CosinePalette {
        name: "Ink",
        a: [0.2, 0.2, 0.22],
        b: [0.6, 0.6, 0.62],
        c: [1.0, 1.0, 1.0],
        d: [0.0, 0.05, 0.1],
    },
    CosinePalette {
        name: "Basalt",
        a: [0.22, 0.22, 0.24],
        b: [0.18, 0.18, 0.2],
        c: [1.0, 1.0, 1.1],
        d: [0.05, 0.1, 0.15],
    },
];

/// `BUILTIN_PALETTES[index] ?? BUILTIN_PALETTES[0]`: anything that is not a
/// valid integer index — negative, too large, fractional — is the first one.
pub fn palette_by_index(index: f64) -> &'static CosinePalette {
    let valid = index >= 0.0 && index.fract() == 0.0 && (index as usize) < BUILTIN_PALETTES.len();
    &BUILTIN_PALETTES[if valid { index as usize } else { 0 }]
}

/// What the display pass reads: the composed gradient plus the relief knobs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    pub d: Vec3,
    pub bands: f32,
    pub relief: f32,
    pub light_angle: f32,
    pub gloss: f32,
}

impl Palette {
    /// Folds shift into `d` and contrast into `b` (`composePalette`).
    pub fn compose(
        base: &CosinePalette,
        shift: f64,
        contrast: f64,
        bands: f64,
        relief: f64,
        light_angle: f64,
        gloss: f64,
    ) -> Self {
        let (shift, contrast) = (shift as f32, contrast as f32);
        Self {
            a: base.a,
            b: base.b.map(|x| x * contrast),
            c: base.c,
            d: base.d.map(|x| x + shift),
            bands: bands as f32,
            relief: relief as f32,
            light_angle: light_angle as f32,
            gloss: gloss as f32,
        }
    }

    /// From the Palette card's (effective) params, the card's defaults for
    /// anything missing.
    pub fn from_card(p: &Params) -> Self {
        let get = |k: &str, d: f64| p.get(k).copied().unwrap_or(d);
        Self::compose(
            palette_by_index(get("paletteId", 0.0)),
            get("shift", 0.0),
            get("contrast", 1.0),
            get("bands", 1.0),
            get("relief", 0.8),
            get("lightAngle", 2.0),
            get("gloss", 0.3),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::card_def;

    #[test]
    fn palette_names_line_up_with_the_select() {
        let select = &card_def("palette").unwrap().selects[0];
        let names: Vec<&str> = select.options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(names, BUILTIN_PALETTES.map(|p| p.name));
    }

    #[test]
    fn a_bad_index_falls_back_to_the_first_palette() {
        assert_eq!(palette_by_index(1.0).name, "Glaze");
        assert_eq!(palette_by_index(99.0).name, "Marble");
        assert_eq!(palette_by_index(-1.0).name, "Marble");
        assert_eq!(palette_by_index(1.5).name, "Marble");
    }

    #[test]
    fn compose_folds_shift_into_d_and_contrast_into_b() {
        let base = &BUILTIN_PALETTES[2];
        let u = Palette::compose(base, 0.25, 2.0, 3.0, 1.5, 1.2, 0.4);
        assert_eq!(u.a, base.a);
        assert_eq!(u.c, base.c);
        assert_eq!(u.b, base.b.map(|x| x * 2.0));
        assert_eq!(u.d, base.d.map(|x| x + 0.25));
        assert_eq!((u.bands, u.relief, u.light_angle, u.gloss), (3.0, 1.5, 1.2, 0.4));
    }
}
