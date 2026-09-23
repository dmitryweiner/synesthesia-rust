// Dumps the web app's built-in presets and parameter schema into assets/.
// PLAN.md decision 2: the presets are taken ready-made, never re-typed.
// Run it again only when the web app's presets or ranges change.
//
//   node scripts/dump-presets.mjs
//
// Needs ../synesthesia (its dev server and its Playwright are reused).
import { writeFileSync } from 'node:fs';
import { ensureServer, launchBrowser, openApp } from '../../synesthesia/scripts/lib.mjs';

const OUT = new URL('../assets/', import.meta.url);

const server = await ensureServer(false);
const browser = await launchBrowser();
const page = await (await browser.newContext()).newPage();
await openApp(page, `${server.BASE}/?preset=0&res=128`);

const dump = await page.evaluate(async () => {
  const { PRESETS } = await import('/src/presets.ts');
  const audio = await import('/src/schema/audio.ts');
  const visual = await import('/src/schema/visual.ts');
  const state = await import('/src/state/schema.ts');
  const gen = await import('/src/dsp/generator.ts');
  const filters = await import('/src/audio/filters.ts');
  const genes = await import('/src/genome/genes.ts');
  const codec = await import('/src/genome/codec.ts');
  return {
    presets: PRESETS.map((p) => ({ name: p.name, state: p.state })),
    // The same points as genome vectors — the fixture the Rust codec is
    // checked against (tests/genome.rs).
    genomes: PRESETS.map((p) => codec.encodeGenome(p.state)),
    schema: {
      formulaIds: gen.FORMULA_IDS,
      defaultParams: gen.DEFAULT_PARAMS,
      formulas: audio.FORMULAS,
      maxEnabledFormulas: audio.MAX_ENABLED_FORMULAS,
      filterTypes: audio.FILTER_TYPES,
      chorusModes: audio.CHORUS_MODES,
      phaserStages: audio.PHASER_STAGES,
      defaultFx: audio.DEFAULT_FX,
      vowels: filters.VOWELS,
      fxOnKeys: audio.FX_ON_KEYS,
      fxModParams: audio.FX_MOD_PARAMS,
      fxParamRanges: audio.FX_PARAM_RANGES,
      fxParamModule: audio.FX_PARAM_MODULE,
      fxExpParams: [...audio.FX_EXP_PARAMS],
      reverbDecayRange: audio.REVERB_DECAY_RANGE,
      cards: visual.CARDS,
      alwaysOnCardIds: visual.ALWAYS_ON_CARD_IDS,
      genes: genes.GENES,
      modTargets: genes.MOD_TARGETS,
      routeSlots: genes.ROUTE_SLOTS,
      lfoShapes: genes.LFO_SHAPES,
      lfoRateRange: genes.LFO_RATE_RANGE,
      lfoCount: state.LFO_COUNT,
      defaultLfo: state.DEFAULT_LFO,
      defaultMasterGain: state.DEFAULT_MASTER_GAIN,
      couplingKeys: state.COUPLING_KEYS,
      couplingRanges: state.COUPLING_RANGES,
      couplingFloor: state.COUPLING_FLOOR,
      defaultCoupling: state.defaultCoupling(),
      defaultState: state.defaultAppState(),
    },
  };
});

writeFileSync(new URL('presets.json', OUT), JSON.stringify(dump.presets, null, 1) + '\n');
writeFileSync(new URL('schema.json', OUT), JSON.stringify(dump.schema, null, 1) + '\n');
writeFileSync(new URL('genomes.json', OUT), JSON.stringify(dump.genomes) + '\n');
console.log(`presets: ${dump.presets.length}, formulas: ${dump.schema.formulas.length}, genes: ${dump.schema.genes.length}`);

await browser.close();
await server.stop?.();
process.exit(0);
