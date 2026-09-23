// Does the Rust engine still sound like the web app? (PLAN.md decision 3.)
//
// The FX chain is our own, so samples cannot match — the budget is stated as
// behaviour instead: the same point, rendered here and in the browser, must
// land within ±3 dB RMS and inside the presets' fractality band. The metric is
// the web app's own `analyzeSound`, run on both renders, so nothing has to be
// ported to measure it.
//
//   node scripts/parity.mjs [--secs 24] [--sr 22050]
import { execFileSync } from 'node:child_process';
import { readFileSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { ensureServer, launchBrowser, openApp } from '../../synesthesia/scripts/lib.mjs';

const args = process.argv.slice(2);
const flag = (name, dflt) => {
  const i = args.indexOf(`--${name}`);
  return i < 0 ? dflt : Number(args[i + 1]);
};
const SECS = flag('secs', 24);
const SR = flag('sr', 22050);
const BIN = process.env.SYN_BIN
  || join(process.env.CARGO_TARGET_DIR || new URL('../target', import.meta.url).pathname, 'release/synesthesia');

const dir = mkdtempSync(join(tmpdir(), 'syn-parity-'));
const server = await ensureServer(false);
const browser = await launchBrowser();
const page = await (await browser.newContext()).newPage();
const APP = `${server.BASE}/?preset=0&res=128`;
await openApp(page, APP);

// Vite re-optimizes dependencies on the first import and forces a full reload,
// which destroys the execution context mid-evaluate. Import everything once up
// front, and retry once if a reload still catches us.
const warm = async () => page.evaluate(async () => {
  await Promise.all(['/src/presets.ts', '/src/audio/engine.ts', '/src/analysis/fractal.ts'].map((m) => import(m)));
});
await warm();
const evaluate = async (fn, arg) => {
  for (let attempt = 0; ; attempt++) {
    try {
      return await page.evaluate(fn, arg);
    } catch (e) {
      // Vite full-reloads the page when it optimizes a newly imported module,
      // which destroys the execution context mid-call. The page comes back by
      // itself; just warm it again and retry.
      if (attempt >= 3 || !/Execution context was destroyed|Target closed/.test(String(e))) throw e;
      await page.waitForTimeout(2000);
      await warm().catch(() => {});
    }
  }
};

const db = (x) => 20 * Math.log10(Math.max(x, 1e-12));
const rms = (a) => Math.sqrt(a.reduce((s, v) => s + v * v, 0) / a.length);

const rows = [];
const count = await evaluate(async () => (await import('/src/presets.ts')).PRESETS.length);

for (let i = 0; i < count; i++) {
  const raw = join(dir, `p${i}.f32`);
  const out = execFileSync(BIN, ['render', '--preset', String(i), '--secs', String(SECS), '--sr', String(SR), '--raw', raw, '--score'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
  const speed = /\(([\d.]+)x realtime\)/.exec(out)?.[1];
  // The same samples, scored by our own port of the metric — so a difference
  // in the metric can be told from a difference in the sound.
  const ownScore = JSON.parse(out.slice(out.indexOf('{'))).score;
  const buf = readFileSync(raw);
  const rust = Array.from(new Float32Array(buf.buffer, buf.byteOffset, buf.length / 4));

  const r = await evaluate(async ({ i, secs, sr, rust }) => {
    const { PRESETS } = await import('/src/presets.ts');
    const { AudioEngine } = await import('/src/audio/engine.ts');
    const { analyzeSound } = await import('/src/analysis/fractal.ts');
    const s = PRESETS[i].state;
    const web = await AudioEngine.renderOffline(
      { masterGain: s.audio.masterGain, fx: s.audio.fx, formulas: s.audio.formulas, mod: s.mod },
      secs, sr,
    );
    const a = analyzeSound(web, sr);
    const b = analyzeSound(Float32Array.from(rust), sr);
    const rmsOf = (x) => Math.sqrt(x.reduce((acc, v) => acc + v * v, 0) / x.length);
    return { name: PRESETS[i].name, webScore: a.score, rustScore: b.score, webRms: rmsOf(web) };
  }, { i, secs: SECS, sr: SR, rust });

  rows.push({ ...r, rustRms: rms(rust), speed, ownScore });
}

let bad = 0;
console.log('\npreset                 rms(web)  rms(rust)   Δ dB   score(web)  score(rust)  own metric    x realtime');
for (const r of rows) {
  const d = db(r.rustRms) - db(r.webRms);
  // Budget (PLAN.md decision 3): within ±3 dB of the browser's loudness, and
  // the fractality character close to it. The browser's own score moves by
  // ~0.1 between runs (its noise generators are unseeded), so 0.3 is the line.
  const flagged = Math.abs(d) > 3 || Math.abs(r.rustScore - r.webScore) > 0.3;
  if (flagged) bad++;
  console.log(
    `${r.name.padEnd(20)} ${r.webRms.toFixed(4).padStart(8)} ${r.rustRms.toFixed(4).padStart(10)} ` +
    `${d.toFixed(1).padStart(6)}   ${r.webScore.toFixed(2).padStart(9)} ${r.rustScore.toFixed(2).padStart(12)} ` +
    `${r.ownScore.toFixed(2).padStart(11)} ${String(r.speed).padStart(13)}${flagged ? '  <-- outside the budget' : ''}`,
  );
}
const mean = (f) => rows.reduce((s, r) => s + f(r), 0) / rows.length;
console.log(`\nmean score: web ${mean((r) => r.webScore).toFixed(2)}, rust ${mean((r) => r.rustScore).toFixed(2)}`);
console.log(`mean |Δ| rms: ${mean((r) => Math.abs(db(r.rustRms) - db(r.webRms))).toFixed(1)} dB`);
console.log(`metric port: mean |score(web metric) − score(own metric)| on the same samples: ${mean((r) => Math.abs(r.rustScore - r.ownScore)).toFixed(3)}`);

await browser.close();
await server.stop?.();
process.exit(bad > 0 ? 1 : 0);
