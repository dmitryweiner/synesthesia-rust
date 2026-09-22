// Dumps reference generator output from the web app (PLAN.md decision 4):
// every formula, rendered by the TypeScript with a seeded RNG, block by
// block exactly as the worklet drives it. The Rust port is diffed against
// these files in syn-core/tests/golden.rs.
//
//   node scripts/dump-golden.mjs
//
// Takes per formula:
//   a  48 kHz, 8192 samples          — the exact-arithmetic check
//   b   8 kHz, 16000 samples (2 s)   — long horizon, events forced to fire
//   c  48 kHz, 4096 samples, one LFO route — the block-rate modulation path
import { writeFileSync } from 'node:fs';
import { ensureServer, launchBrowser, openApp } from '../../synesthesia/scripts/lib.mjs';

const OUT = new URL('../golden/', import.meta.url);
const BLOCK = 128;
const SEED = 12345;

// Make the slow events (strikes, drops, sweeps) happen inside a 2 s take.
const FAST = {
  bellPeriod: 0.7, rissPeriod: 0.7, rainDensity: 20, velvetDensity: 500,
  shepSpeed: 0.5, lfoHz: 60, bbRate: 4000,
};

const server = await ensureServer(false);
const browser = await launchBrowser();
const page = await (await browser.newContext()).newPage();
await openApp(page, `${server.BASE}/?preset=0&res=128`);

const takes = await page.evaluate(async ({ BLOCK, SEED, FAST }) => {
  const { FormulaGenerator, FORMULA_IDS } = await import('/src/dsp/generator.ts');
  const { mulberry32 } = await import('/src/dsp/rng.ts');
  const { formulaDefaults, formulaDef } = await import('/src/schema/audio.ts');

  const render = (id, sr, n, params, mod) => {
    const g = new FormulaGenerator(id, sr, params, mulberry32(SEED));
    if (mod) g.setMod(mod.lfos, mod.routes, mod.ranges);
    const out = new Float32Array(n);
    const block = new Float32Array(BLOCK);
    for (let off = 0; off < n; off += BLOCK) { g.fill(block); out.set(block, off); }
    return Array.from(out);
  };

  const out = [];
  for (const id of FORMULA_IDS) {
    const base = formulaDefaults(id);
    out.push({ id, take: 'a', sr: 48000, n: 8192, params: base, samples: render(id, 48000, 8192, base) });

    const fast = { ...base };
    for (const [k, v] of Object.entries(FAST)) if (k in fast) fast[k] = v;
    out.push({ id, take: 'b', sr: 8000, n: 16000, params: fast, samples: render(id, 8000, 16000, fast) });

    // One LFO route onto the formula's first non-gain slider.
    const def = formulaDef(id);
    const slider = def.sliders.find((s) => s.k !== 'gain') ?? def.sliders[0];
    const mod = {
      lfos: [{ shape: 'triangle', rate: 3, phase: 0.25 }],
      routes: [{ src: 0, target: id, param: slider.k, depth: 0.6, exp: slider.exp === true }],
      ranges: Object.fromEntries(def.sliders.map((s) => [s.k, [s.min, s.max]])),
    };
    out.push({
      id, take: 'c', sr: 48000, n: 4096, params: base, mod,
      samples: render(id, 48000, 4096, base, mod),
    });
  }
  return out;
}, { BLOCK, SEED, FAST });

const manifest = { blockSize: BLOCK, seed: SEED, rng: 'mulberry32', takes: [] };
for (const t of takes) {
  const file = `${t.id}.${t.take}.f32`;
  writeFileSync(new URL(file, OUT), Buffer.from(new Float32Array(t.samples).buffer));
  manifest.takes.push({ id: t.id, take: t.take, file, sr: t.sr, n: t.n, params: t.params, mod: t.mod ?? null });
}
writeFileSync(new URL('manifest.json', OUT), JSON.stringify(manifest, null, 1) + '\n');
console.log(`${manifest.takes.length} takes for ${new Set(takes.map((t) => t.id)).size} formulas`);

await browser.close();
await server.stop?.();
process.exit(0);
