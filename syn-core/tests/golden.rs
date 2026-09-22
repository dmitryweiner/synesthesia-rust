//! Diffs every formula against the reference output of the web app
//! (PLAN.md decision 4). The takes are produced by `scripts/dump-golden.mjs`
//! and live in `golden/`:
//!
//!   a  48 kHz, 8192 samples        — exact arithmetic
//!   b   8 kHz, 16000 samples       — long horizon, slow events forced to fire
//!   c  48 kHz, 4096 samples + LFO  — the block-rate modulation path

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use syn_core::dsp::rng::Mulberry32;
use syn_core::modmatrix::{LfoDef, ModRoute, ParamRanges};
use syn_core::{FormulaGenerator, FormulaId};

#[derive(Deserialize)]
struct Mod {
    lfos: Vec<LfoDef>,
    routes: Vec<ModRoute>,
    ranges: ParamRanges,
}

#[derive(Deserialize)]
struct Take {
    id: String,
    take: String,
    file: String,
    sr: f64,
    n: usize,
    params: BTreeMap<String, f64>,
    #[serde(rename = "mod")]
    modulation: Option<Mod>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    block_size: usize,
    seed: u32,
    takes: Vec<Take>,
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../golden")
}

fn read_f32(path: &Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

fn render(m: &Manifest, t: &Take) -> Vec<f32> {
    let id = FormulaId::parse(&t.id).unwrap_or_else(|| panic!("unknown formula {}", t.id));
    let mut g = FormulaGenerator::new(id, t.sr, t.params.clone(), Box::new(Mulberry32::new(m.seed)));
    if let Some(md) = &t.modulation {
        g.set_mod(&md.lfos, &md.routes, &md.ranges);
    }
    let mut out = vec![0.0f32; t.n];
    for block in out.chunks_mut(m.block_size) {
        g.fill(block);
    }
    out
}

/// Where the two renders first differ by more than `tol`, and by how much.
fn compare(got: &[f32], want: &[f32], tol: f32) -> (Option<usize>, f32) {
    let mut worst = 0.0f32;
    let mut first = None;
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        let d = (g - w).abs();
        if d > worst {
            worst = d;
        }
        if first.is_none() && d > tol {
            first = Some(i);
        }
    }
    (first, worst)
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len() as f64).sqrt()
}

fn manifest() -> Manifest {
    let raw = std::fs::read_to_string(golden_dir().join("manifest.json")).expect("golden/manifest.json");
    serde_json::from_str(&raw).expect("manifest parses")
}

/// Formulas whose state is chaotic: a last-bit difference in `sin` or `exp`
/// would grow exponentially instead of staying a last-bit difference. On this
/// machine they match the browser exactly like everything else (V8 and glibc
/// agree here), so the list is empty — move an id in here if another libm ever
/// makes its long horizon drift, and `chaotic_formulas_stay_in_family` will
/// keep guarding it.
const CHAOTIC: &[&str] = &[];

/// Watched by the statistical test whether or not they are exempted above.
const CHAOS_WATCH: &[&str] = &["logistic", "lorenz", "rossler"];

const TOL: f32 = 1e-6;

#[test]
fn every_formula_matches_the_web_app() {
    let m = manifest();
    let mut failures = Vec::new();
    let mut worst_overall = 0.0f32;
    let mut worst_take = String::new();
    for t in &m.takes {
        if CHAOTIC.contains(&t.id.as_str()) {
            continue;
        }
        let want = read_f32(&golden_dir().join(&t.file));
        assert_eq!(want.len(), t.n, "{} take {}", t.id, t.take);
        let got = render(&m, t);
        let (first, worst) = compare(&got, &want, TOL);
        if worst > worst_overall {
            worst_overall = worst;
            worst_take = format!("{} take {}", t.id, t.take);
        }
        if let Some(i) = first {
            failures.push(format!(
                "{} take {}: first diff at sample {i} ({} vs {}), worst {worst:e}",
                t.id, t.take, got[i], want[i]
            ));
        }
    }
    println!("{} takes, worst sample difference {worst_overall:e} ({worst_take})", m.takes.len());
    assert!(failures.is_empty(), "{} take(s) differ:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn chaotic_formulas_stay_in_family() {
    let m = manifest();
    for t in m.takes.iter().filter(|t| CHAOS_WATCH.contains(&t.id.as_str())) {
        let want = read_f32(&golden_dir().join(&t.file));
        let got = render(&m, t);

        // The first block must still be sample-accurate: that is what proves
        // the port, rather than the long-horizon trajectory.
        let (first, _) = compare(&got[..m.block_size], &want[..m.block_size], TOL);
        assert!(first.is_none(), "{} take {}: diverges inside the first block", t.id, t.take);

        let (a, b) = (rms(&got), rms(&want));
        assert!((a - b).abs() <= 0.1 * b.max(1e-6), "{} take {}: RMS {a:.5} vs {b:.5}", t.id, t.take);
    }
}
