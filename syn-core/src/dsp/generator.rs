//! The 21 formula generators, ported sample-for-sample from
//! `../synesthesia/src/dsp/generator.ts` (PLAN.md decision 4).
//!
//! Two things are load-bearing and easy to lose in a port:
//!
//! * **Every oscillator accumulates phase.** Writing `sin(2π·f·t)` with an
//!   absolute `t` makes any frequency change jump the phase by `2π·Δf·t`,
//!   which the web app heard as harsh beating after a few minutes.
//! * **Modulated parameters are constant across a block.** The TypeScript
//!   overwrites them, runs the block and restores; here each arm reads its
//!   parameters into locals once per block, which is the same thing and lets
//!   the inner loop stay tight.
//!
//! Storage widths matter too: `shepard_phases` and the Karplus–Strong buffer
//! are `f32` in the browser (`Float32Array`), `riss_phases` is `f64`. Keeping
//! those widths is what makes the golden takes match.

use std::collections::BTreeMap;

use crate::modmatrix::{effective_param, lfo_value, LfoDef, ModRoute, ParamRanges};

use super::rng::Rng;

const TWO_PI: f64 = std::f64::consts::TAU;
/// Restart a glissando after 4 octaves.
const GLISS_SPAN: f64 = 2.772_588_722_239_781; // ln(16)

/// Parameter pool of one generator, keyed exactly as the sliders are.
pub type Params = BTreeMap<String, f64>;

macro_rules! formula_ids {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        /// The 21 formulas, in the order the genome stores them.
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum FormulaId { $($variant),+ }

        impl FormulaId {
            pub const ALL: &'static [FormulaId] = &[$(FormulaId::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $(FormulaId::$variant => $name),+ }
            }

            pub fn parse(s: &str) -> Option<Self> {
                match s { $($name => Some(FormulaId::$variant),)+ _ => None }
            }
        }
    };
}

formula_ids! {
    Fm => "fm", Logistic => "logistic", Gliss => "gliss", Additive => "additive",
    Pm => "pm", Beats => "beats", Dist => "dist", Quasi => "quasi",
    Lorenz => "lorenz", Karplus => "karplus", Noiselp => "noiselp",
    Pinknoise => "pinknoise", Brownnoise => "brownnoise", Velvetnoise => "velvetnoise",
    Rossler => "rossler", Shepard => "shepard", Bytebeat => "bytebeat",
    Bell => "bell", Ocean => "ocean", Risset => "risset", Rain => "rain",
}

impl std::fmt::Display for FormulaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// Classic bytebeat recipes: integer `t`, result taken mod 256. JavaScript's
// bitwise operators work on int32 and its `*` on f64, so the port converts at
// exactly the same points (`i32_of`).
fn i32_of(x: f64) -> i32 {
    (x as i64 as u32) as i32
}

fn bytebeat(recipe: usize, t: f64) -> i32 {
    let ti = i32_of(t);
    let v: f64 = match recipe {
        0 => f64::from((ti >> 10) & 42) * t,
        1 => t * f64::from(((ti >> 12) | (ti >> 8)) & 63 & (ti >> 4)),
        2 => {
            let prod = t * f64::from((ti >> 5) | (ti >> 8));
            f64::from(i32_of(prod) >> ((ti >> 16) & 31))
        }
        3 => {
            let a = i32_of(t * 5.0) & (ti >> 7);
            let b = i32_of(t * 3.0) & (i32_of(t * 4.0) >> 10);
            f64::from(a | b)
        }
        _ => {
            let a = f64::from((ti >> 7) | ti | (ti >> 6)) * 10.0;
            let b = 4.0 * f64::from((ti & (ti >> 13)) | (ti >> 6));
            a + b
        }
    };
    i32_of(v) & 255
}

const BYTEBEAT_RECIPES: usize = 5;

// Jean-Claude Risset's bell (1969): 11 inharmonic components with individual
// amplitudes and decay times; two detuned pairs give the characteristic beats.
const RISSET_RATIO: [f64; 11] = [0.56, 0.56, 0.92, 0.92, 1.19, 1.70, 2.00, 2.74, 3.00, 3.76, 4.07];
const RISSET_AMP: [f64; 11] = [1.0, 0.67, 1.0, 1.8, 2.67, 1.67, 1.46, 1.33, 1.33, 1.0, 1.33];
const RISSET_DET: [f64; 11] = [0.0, 1.0, 0.0, 1.7, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
const RISSET_DUR: [f64; 11] = [1.0, 0.9, 0.65, 0.55, 0.325, 0.35, 0.25, 0.2, 0.15, 0.1, 0.075];

fn risset_amp_sum() -> f64 {
    RISSET_AMP.iter().sum()
}

/// One formula instance: its parameters, its state and its slice of the
/// modulation matrix.
pub struct FormulaGenerator {
    pub formula: FormulaId,
    pub sr: f64,
    pub p: Params,
    rng: Box<dyn Rng + Send>,

    t: f64,
    phase: f64,

    logi: f64,
    lx: f64,
    ly: f64,
    lz: f64,

    ks_buf: Vec<f32>,
    ks_idx: usize,
    ks_n: usize,
    nlp: f64,

    // Pink noise (Paul Kellet's approximation)
    pink_b: [f64; 7],

    brown_val: f64,

    velvet_counter: f64,
    velvet_next: f64,

    rx: f64,
    ry: f64,
    rz: f64,

    // Accumulated oscillator phases (radians, wrapped once per block).
    ph1: f64,
    ph2: f64,
    ph3: f64,
    ph4: f64,
    gliss_log: f64,
    riss_phases: [f64; 11],

    shepard_phases: [f32; 10],
    shepard_t: f64,

    bb_t: f64,
    bell_t: f64,
    ocean_lp: f64,

    riss_t: f64,
    rain_lp: f64,
    drop_t: f64,
    drop_phase: f64,
    drop_freq: f64,
    drop_amp: f64,
    rain_counter: f64,
    rain_next: f64,

    // Block-rate modulation. `mod_t` is absolute LFO time and is NOT reset by
    // `reset()` — a reset restarts the sound, not the modulation.
    mod_t: f64,
    mod_lfos: Vec<LfoDef>,
    mod_routes: Vec<ModRoute>,
    mod_ranges: ParamRanges,
    mod_saved: Vec<f64>,
}

impl FormulaGenerator {
    /// `params` is laid on top of the shared default pool, exactly as the
    /// TypeScript constructor does (`{ ...DEFAULT_PARAMS, ...params }`), so a
    /// point that only carries its own sliders still has every key a formula
    /// might read.
    pub fn new(formula: FormulaId, sample_rate: f64, params: Params, rng: Box<dyn Rng + Send>) -> Self {
        let mut pool = crate::schema::default_params();
        pool.extend(params);
        let mut g = Self {
            formula,
            sr: sample_rate,
            p: pool,
            rng,
            t: 0.0,
            phase: 0.0,
            logi: 0.33,
            lx: 0.1,
            ly: 0.0,
            lz: 0.0,
            ks_buf: Vec::new(),
            ks_idx: 0,
            ks_n: 0,
            nlp: 0.0,
            pink_b: [0.0; 7],
            brown_val: 0.0,
            velvet_counter: 0.0,
            velvet_next: 0.0,
            rx: 0.1,
            ry: 0.0,
            rz: 0.0,
            ph1: 0.0,
            ph2: 0.0,
            ph3: 0.0,
            ph4: 0.0,
            gliss_log: 0.0,
            riss_phases: [0.0; 11],
            shepard_phases: [0.0; 10],
            shepard_t: 0.0,
            bb_t: 0.0,
            bell_t: 0.0,
            ocean_lp: 0.0,
            riss_t: 0.0,
            rain_lp: 0.0,
            drop_t: 1e9,
            drop_phase: 0.0,
            drop_freq: 0.0,
            drop_amp: 0.0,
            rain_counter: 0.0,
            rain_next: 0.0,
            mod_t: 0.0,
            mod_lfos: Vec::new(),
            mod_routes: Vec::new(),
            mod_ranges: ParamRanges::new(),
            mod_saved: Vec::new(),
        };
        g.init_ks(true);
        g
    }

    /// Reads a parameter, falling back to 0 for keys this formula never uses.
    #[inline]
    fn get(&self, k: &str) -> f64 {
        self.p.get(k).copied().unwrap_or(0.0)
    }

    pub fn set(&mut self, params: &Params) {
        for (k, v) in params {
            self.p.insert(k.clone(), *v);
        }
    }

    /// The routes aimed at this generator, plus the LFO pool and the ranges.
    pub fn set_mod(&mut self, lfos: &[LfoDef], routes: &[ModRoute], ranges: &ParamRanges) {
        self.mod_lfos = lfos.to_vec();
        self.mod_routes = routes.to_vec();
        self.mod_ranges = ranges.clone();
    }

    /// Absolute modulation time, in seconds.
    pub fn mod_time(&self) -> f64 {
        self.mod_t
    }

    pub fn set_mod_time(&mut self, t: f64) {
        self.mod_t = t;
    }

    /// Restarts the sound (not the modulation clock).
    pub fn reset(&mut self) {
        self.t = 0.0;
        self.phase = 0.0;
        self.ph1 = 0.0;
        self.ph2 = 0.0;
        self.ph3 = 0.0;
        self.ph4 = 0.0;
        self.gliss_log = 0.0;
        self.riss_phases = [0.0; 11];
        self.logi = 0.33;
        self.lx = 0.1;
        self.ly = 0.0;
        self.lz = 0.0;
        self.nlp = 0.0;
        self.init_ks(true);
        self.pink_b = [0.0; 7];
        self.brown_val = 0.0;
        self.velvet_counter = 0.0;
        self.velvet_next = 0.0;
        self.rx = 0.1;
        self.ry = 0.0;
        self.rz = 0.0;
        self.shepard_phases = [0.0; 10];
        self.shepard_t = 0.0;
        self.bb_t = 0.0;
        self.bell_t = 0.0;
        self.ocean_lp = 0.0;
        self.riss_t = 0.0;
        self.rain_lp = 0.0;
        self.drop_t = 1e9;
        self.drop_phase = 0.0;
        self.drop_freq = 0.0;
        self.drop_amp = 0.0;
        self.rain_counter = 0.0;
        self.rain_next = 0.0;
    }

    fn init_ks(&mut self, force: bool) {
        // `Math.max(20, p.ksFreq || 110)` — a missing or zero ksFreq means 110,
        // and the floor is applied after that substitution.
        let raw = self.get("ksFreq");
        let freq = if raw == 0.0 { 110.0 } else { raw }.max(20.0);
        let n = ((self.sr / freq).floor() as usize).max(2);
        if !force && !self.ks_buf.is_empty() && self.ks_n == n {
            return;
        }
        self.ks_n = n;
        self.ks_buf = vec![0.0; n];
        self.ks_idx = 0;
        for i in 0..n {
            self.ks_buf[i] = (self.rng.next() * 2.0 - 1.0) as f32;
        }
    }

    /// Overwrites the modulated parameters with their effective values for
    /// this block; `restore_mod` puts the base values back.
    fn apply_mod(&mut self) {
        self.mod_saved.clear();
        for i in 0..self.mod_routes.len() {
            let r = &self.mod_routes[i];
            let base = self.p.get(&r.param).copied().unwrap_or(f64::NAN);
            self.mod_saved.push(base);
            let (Some(lfo), Some(range)) = (self.mod_lfos.get(r.src), self.mod_ranges.get(&r.param)) else {
                continue;
            };
            if !base.is_finite() {
                continue;
            }
            let v = effective_param(base, lfo_value(lfo, self.mod_t), r.depth, *range, r.exp);
            self.p.insert(r.param.clone(), v);
        }
    }

    fn restore_mod(&mut self) {
        for i in 0..self.mod_routes.len() {
            let key = self.mod_routes[i].param.clone();
            let saved = self.mod_saved[i];
            if saved.is_finite() {
                self.p.insert(key, saved);
            }
        }
    }

    /// Fills one block. `out.len()` is normally [`crate::BLOCK`].
    pub fn fill(&mut self, out: &mut [f32]) {
        let n = out.len();
        let sr = self.sr;
        let w = TWO_PI / sr;

        self.apply_mod();
        let gain = self.p.get("gain").copied().unwrap_or(0.2);

        match self.formula {
            FormulaId::Fm => {
                let (fc, fm, index) = (self.get("fc"), self.get("fm"), self.get("I"));
                for s in out.iter_mut() {
                    *s = ((self.ph1 + index * self.ph2.sin()).sin() * gain) as f32;
                    self.ph1 += w * fc;
                    self.ph2 += w * fm;
                }
            }
            FormulaId::Logistic => {
                let (r, base, depth) = (self.get("r"), self.get("base"), self.get("depth"));
                let step = (sr / self.get("lfoHz").max(1e-3)).floor().max(1.0) as usize;
                for (i, s) in out.iter_mut().enumerate() {
                    if i % step == 0 {
                        self.logi = (r * self.logi * (1.0 - self.logi)).clamp(0.0, 1.0);
                    }
                    let f = base + depth * (self.logi - 0.5);
                    self.phase += TWO_PI * (f.max(0.0) / sr);
                    *s = (self.phase.sin() * gain) as f32;
                    self.wrap_phase();
                }
            }
            FormulaId::Gliss => {
                let (f0, k) = (self.get("f0"), self.get("k"));
                for s in out.iter_mut() {
                    self.gliss_log += k / sr;
                    if self.gliss_log.abs() > GLISS_SPAN {
                        self.gliss_log = 0.0;
                    }
                    self.phase += w * (f0 * self.gliss_log.exp());
                    *s = (self.phase.sin() * gain) as f32;
                    self.wrap_phase();
                }
            }
            FormulaId::Additive => {
                let fund = self.get("fund");
                let n_harm = (self.get("N").floor() as i64).max(1);
                let move_hz = self.get("move");
                let norm = 1.0 / ((n_harm as f64) + 1.0).log2();
                for s in out.iter_mut() {
                    let mut sum = 0.0;
                    for k in 1..=n_harm {
                        let kf = k as f64;
                        let ak = (1.0 / kf) * (self.ph3 + kf).sin();
                        sum += ak * (kf * self.ph1).sin();
                    }
                    *s = (sum * norm * gain) as f32;
                    self.ph1 += w * fund;
                    self.ph3 += w * move_hz;
                }
            }
            FormulaId::Pm => {
                let (f, f2) = (self.get("f"), self.get("f2pm"));
                for s in out.iter_mut() {
                    let phi = self.ph2.sin().sin();
                    *s = ((self.ph1 + phi * 5.0).sin() * gain) as f32;
                    self.ph1 += w * f;
                    self.ph2 += w * f2;
                }
            }
            FormulaId::Beats => {
                let (fbeat, df) = (self.get("fbeat"), self.get("df"));
                for s in out.iter_mut() {
                    *s = (0.5 * (self.ph1.sin() + self.ph2.sin()) * gain) as f32;
                    self.ph1 += w * fbeat;
                    self.ph2 += w * (fbeat + df);
                }
            }
            FormulaId::Dist => {
                let (alpha, fd) = (self.get("alpha"), self.get("fd"));
                for s in out.iter_mut() {
                    *s = ((alpha * self.ph1.sin()).tanh() * gain) as f32;
                    self.ph1 += w * fd;
                }
            }
            FormulaId::Quasi => {
                let (fq, aq, wq) = (self.get("fq"), self.get("Aq"), self.get("wq"));
                for s in out.iter_mut() {
                    let m = self.ph3.sin().sin().sin();
                    self.ph3 += wq / sr;
                    let f = (fq + aq * m).max(0.0);
                    self.phase += TWO_PI * (f / sr);
                    *s = (self.phase.sin() * gain) as f32;
                    self.wrap_phase();
                }
            }
            FormulaId::Lorenz => {
                let (sigma, rho, beta) = (self.get("sigma"), self.get("rho"), self.get("beta"));
                let (l_base, l_scale, l_amp) = (self.get("lBase"), self.get("lFreqScale"), self.get("lAmp"));
                let dt = 1.0 / sr;
                for s in out.iter_mut() {
                    let dx = sigma * (self.ly - self.lx);
                    let dy = self.lx * (rho - self.lz) - self.ly;
                    let dz = self.lx * self.ly - beta * self.lz;
                    self.lx += dx * dt;
                    self.ly += dy * dt;
                    self.lz += dz * dt;

                    let freq = (l_base + l_scale * self.lx.abs()).max(0.0);
                    let amp = (l_amp * (0.3 + 0.7 * (0.5 + 0.5 * self.ly.tanh()))).clamp(0.0, 1.0);
                    self.phase += TWO_PI * (freq / sr);
                    *s = (amp * self.phase.sin() * gain) as f32;
                    self.wrap_phase();
                }
            }
            FormulaId::Karplus => {
                self.init_ks(false);
                let damp = self.get("ksDamp").clamp(0.8, 0.999_99);
                let bright = self.get("ksBright").clamp(0.0, 1.0);
                let n_ks = self.ks_n;
                for s in out.iter_mut() {
                    let idx = self.ks_idx;
                    let y0 = f64::from(self.ks_buf[idx]);
                    let y1 = f64::from(self.ks_buf[(idx + 1) % n_ks]);
                    let avg = 0.5 * (y0 + y1);
                    self.ks_buf[idx] = (damp * (bright * y0 + (1.0 - bright) * avg)) as f32;
                    self.ks_idx = (idx + 1) % n_ks;
                    *s = (y0 * gain) as f32;
                }
            }
            FormulaId::Noiselp => {
                let cut = self.get("nCut").clamp(20.0, 18000.0);
                let a = 1.0 - (-2.0 * std::f64::consts::PI * cut / sr).exp();
                for s in out.iter_mut() {
                    let white = self.rng.next() * 2.0 - 1.0;
                    self.nlp += a * (white - self.nlp);
                    *s = (self.nlp * gain) as f32;
                }
            }
            FormulaId::Pinknoise => {
                let bright = self.get("pinkBright").clamp(0.0, 1.0);
                for s in out.iter_mut() {
                    let white = self.rng.next() * 2.0 - 1.0;
                    let b = &mut self.pink_b;
                    b[0] = 0.998_86 * b[0] + white * 0.055_517_9;
                    b[1] = 0.993_32 * b[1] + white * 0.075_075_9;
                    b[2] = 0.969 * b[2] + white * 0.153_852;
                    b[3] = 0.866_5 * b[3] + white * 0.310_485_6;
                    b[4] = 0.55 * b[4] + white * 0.532_952_2;
                    b[5] = -0.761_6 * b[5] - white * 0.016_898;
                    let pink = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + white * 0.536_2;
                    b[6] = white * 0.115_926;
                    *s = ((pink * 0.11 * (1.0 - bright) + white * bright * 0.5) * gain) as f32;
                }
            }
            FormulaId::Brownnoise => {
                let step = self.get("brownStep").clamp(0.001, 0.1);
                for s in out.iter_mut() {
                    let white = self.rng.next() * 2.0 - 1.0;
                    self.brown_val = (self.brown_val + white * step).clamp(-1.0, 1.0);
                    *s = (self.brown_val * gain) as f32;
                }
            }
            FormulaId::Velvetnoise => {
                let density = self.get("velvetDensity").max(100.0);
                let avg_samples = sr / density;
                for s in out.iter_mut() {
                    let x = if self.velvet_counter >= self.velvet_next {
                        let v = if self.rng.next() > 0.5 { 1.0 } else { -1.0 };
                        self.velvet_next = self.velvet_counter + avg_samples * (0.5 + self.rng.next());
                        v
                    } else {
                        0.0
                    };
                    self.velvet_counter += 1.0;
                    *s = (x * gain) as f32;
                }
            }
            FormulaId::Rossler => {
                let (a, b, c) = (self.get("rossA"), self.get("rossB"), self.get("rossC"));
                let (base, scale, amp_k) =
                    (self.get("rossBase"), self.get("rossFreqScale"), self.get("rossAmp"));
                let dt = 1.0 / sr;
                for s in out.iter_mut() {
                    let dx = -self.ry - self.rz;
                    let dy = self.rx + a * self.ry;
                    let dz = b + self.rz * (self.rx - c);
                    self.rx = (self.rx + dx * dt * 100.0).clamp(-50.0, 50.0);
                    self.ry = (self.ry + dy * dt * 100.0).clamp(-50.0, 50.0);
                    self.rz = (self.rz + dz * dt * 100.0).clamp(-50.0, 50.0);

                    let freq = (base + scale * self.rx).max(0.0);
                    let amp = (amp_k * (0.3 + 0.7 * (0.5 + 0.02 * self.ry))).clamp(0.0, 1.0);
                    self.phase += TWO_PI * (freq / sr);
                    *s = (amp * self.phase.sin() * gain) as f32;
                    self.wrap_phase();
                }
            }
            FormulaId::Shepard => {
                let base_f = self.get("shepBase").max(20.0);
                let speed = self.get("shepSpeed");
                let octaves = (self.get("shepOctaves").floor() as i64).clamp(1, 10) as usize;
                let center_log = 440.0_f64.log2();
                let sigma = 1.5;
                let norm = 1.0 / (octaves as f64).sqrt();
                for s in out.iter_mut() {
                    self.shepard_t += speed / sr;
                    if self.shepard_t > 1.0 {
                        self.shepard_t -= 1.0;
                    }
                    if self.shepard_t < 0.0 {
                        self.shepard_t += 1.0;
                    }
                    let mut sum = 0.0;
                    for k in 0..octaves {
                        let freq = base_f * (k as f64 + self.shepard_t).exp2();
                        if freq > 18000.0 {
                            continue;
                        }
                        let env = (-0.5 * ((freq.log2() - center_log) / sigma).powi(2)).exp();
                        let mut ph = (f64::from(self.shepard_phases[k]) + TWO_PI * (freq / sr)) as f32;
                        if f64::from(ph) > TWO_PI {
                            ph = (f64::from(ph) - TWO_PI) as f32;
                        }
                        self.shepard_phases[k] = ph;
                        sum += env * f64::from(ph).sin();
                    }
                    *s = (sum * norm * gain) as f32;
                }
            }
            FormulaId::Bytebeat => {
                let rate = self.get("bbRate").max(500.0);
                let recipe =
                    (self.get("bbRecipe").floor() as i64).clamp(1, BYTEBEAT_RECIPES as i64) as usize - 1;
                for s in out.iter_mut() {
                    self.bb_t += rate / sr;
                    let byte = bytebeat(recipe, self.bb_t.floor());
                    *s = ((f64::from(byte) / 128.0 - 1.0) * gain) as f32;
                }
            }
            FormulaId::Bell => {
                let decay = self.get("bellDecay").max(0.1);
                let f0 = self.get("bellF0");
                let ratio = self.get("bellRatio");
                let index = self.get("bellIndex");
                let period = self.get("bellPeriod").max(0.5);
                for s in out.iter_mut() {
                    let env = (-3.0 * self.bell_t / decay).exp();
                    *s = (env * (self.ph1 + index * env * self.ph2.sin()).sin() * gain) as f32;
                    self.ph1 += w * f0;
                    self.ph2 += w * f0 * ratio;
                    self.bell_t += 1.0 / sr;
                    if self.bell_t >= period {
                        self.bell_t = 0.0;
                        self.ph1 = 0.0;
                        self.ph2 = 0.0;
                    }
                }
            }
            FormulaId::Ocean => {
                let rate = self.get("oceanRate");
                let cut_base = self.get("oceanCut");
                let depth = self.get("oceanDepth").clamp(0.0, 1.0);
                for s in out.iter_mut() {
                    let white = self.rng.next() * 2.0 - 1.0;
                    let swell1 = 0.5 + 0.5 * (self.ph3 - std::f64::consts::FRAC_PI_2).sin();
                    let swell2 = 0.5 + 0.5 * (self.ph4 + 1.7).sin();
                    self.ph3 += w * rate;
                    self.ph4 += w * rate * 0.37;
                    let mix = (0.6 * swell1 + 0.4 * swell2).powi(2);
                    let cut = (cut_base * (1.0 - depth + depth * 2.0 * mix)).clamp(40.0, 8000.0);
                    let a = 1.0 - (-2.0 * std::f64::consts::PI * cut / sr).exp();
                    self.ocean_lp += a * (white - self.ocean_lp);
                    *s = (self.ocean_lp * (1.0 - depth + depth * mix) * 1.8 * gain) as f32;
                }
            }
            FormulaId::Risset => {
                let decay = self.get("rissDecay").max(0.2);
                let f0 = self.get("rissF0").max(20.0);
                let period = self.get("rissPeriod").max(0.5);
                let amp_sum = risset_amp_sum();
                for s in out.iter_mut() {
                    let mut sum = 0.0;
                    for c in 0..RISSET_RATIO.len() {
                        let env = (-self.riss_t / (RISSET_DUR[c] * decay)).exp();
                        sum += RISSET_AMP[c] * env * self.riss_phases[c].sin();
                        self.riss_phases[c] += w * (f0 * RISSET_RATIO[c] + RISSET_DET[c]);
                        if self.riss_phases[c] > TWO_PI {
                            self.riss_phases[c] -= TWO_PI;
                        }
                    }
                    *s = (sum / amp_sum * gain) as f32;
                    self.riss_t += 1.0 / sr;
                    if self.riss_t >= period {
                        self.riss_t = 0.0;
                        self.riss_phases = [0.0; 11];
                    }
                }
            }
            FormulaId::Rain => {
                let density = self.get("rainDensity").max(0.2);
                let pitch = self.get("rainPitch");
                let bed = self.get("rainBed").clamp(0.0, 1.0);
                let avg_samples = sr / density;
                let bed_a = 1.0 - (-2.0 * std::f64::consts::PI * 2000.0 / sr).exp();
                const DROP_DECAY: f64 = 0.12;
                for s in out.iter_mut() {
                    if self.rain_counter >= self.rain_next {
                        self.drop_t = 0.0;
                        self.drop_phase = 0.0;
                        self.drop_amp = 0.5 + 0.5 * self.rng.next();
                        self.drop_freq = (pitch * (0.6 + 0.9 * self.rng.next())).clamp(100.0, 6000.0);
                        self.rain_next = self.rain_counter + avg_samples * (0.5 + self.rng.next());
                    }
                    self.rain_counter += 1.0;

                    let env = (-self.drop_t / DROP_DECAY).exp();
                    let glide = 1.0 + 0.5 * (1.0 - (-self.drop_t * 60.0).exp());
                    self.drop_phase += TWO_PI * self.drop_freq * glide / sr;
                    let drop = self.drop_amp * env * self.drop_phase.sin();
                    self.drop_t += 1.0 / sr;

                    let white = self.rng.next() * 2.0 - 1.0;
                    self.rain_lp += bed_a * (white - self.rain_lp);

                    *s = ((drop * 0.9 + self.rain_lp * bed * 0.6) * gain) as f32;
                }
            }
        }

        self.t += n as f64 / sr;
        // Wrap once per block: keeps precision, and k·ph1 (additive) stays
        // exact mod 2π.
        self.ph1 %= TWO_PI;
        self.ph2 %= TWO_PI;
        self.ph3 %= TWO_PI;
        self.ph4 %= TWO_PI;

        self.restore_mod();
        self.mod_t += n as f64 / sr;
    }

    #[inline]
    fn wrap_phase(&mut self) {
        if self.phase > 1e9 {
            self.phase %= TWO_PI;
        }
    }
}
