# Synesthesia-rs — plan & decisions

A **console application in Rust** that generates the sound of
[../synesthesia](../synesthesia/) natively: the same 21 formula generators,
the same FX chain, the same LFO matrix, the same ~500-gene point, the same
👍/👎 directed search — with the whole UI in the terminal.

The picture is **not** in scope now and is **designed for** anyway: this
machine has no GPU device at all (`/dev/dri` holds only `card0` from
`sunxi-drm`, there is no `renderD128` and no PowerVR driver in Mesa), so
every WebGL2 pass of the web app is rasterized by llvmpipe on the same CPU
cores. See ../synesthesia/TODO.md for the measurements. When a working
driver appears, the renderer plugs into an interface that exists from day
one (decision 5).

Docs, UI strings and code comments are in English — same rule as the sibling
project, so the two read alike.

## Why native at all (measured on this box, 2026-09-22)

- The live audio graph in the browser renders **8 s of sound in 7.1 s** —
  1.1× realtime, i.e. ~0.95 of a core, permanently on the edge of dropouts.
- The split is **FX 5.7 s / generators 2.0 s**: most of the cost is the
  WebAudio node graph (convolution reverb, compressor, phaser), not the
  formulas.
- The scout spends **~15 s of CPU per 👍/👎** (7 candidates × 2.1 s).
- Reference point: 8 oscillators + 2 biquads in plain C on this CPU cost
  **0.4% of realtime**. The headroom is ~20–30×.

## Decisions (agreed with the user, 2026-09-22)

1. **Scope: sound only, console only.** No windowing, no GUI toolkit, no
   audio plugin, no MIDI. One binary, `synesthesia`, that plays a point and
   lets the user steer the search from the keyboard.
2. **The point is *the same* point.** `AppState` v1 (audio + visual + mod +
   coupling) is kept byte-compatible with the web app: same JSON shape, same
   gene order, same value ranges. A point file written here opens in the
   browser and the other way round, and a `#s=` token pasted from a browser
   link is accepted as an import — no server, and no content-addressed ids
   (decision 9).
   **The 12 built-in presets are taken ready-made from the web app:** dumped
   once into `assets/presets.json` (`PRESETS` → JSON, a throwaway node
   one-liner against the dev server), committed, and re-dumped only when the
   web app's presets change. They are never re-typed by hand and never edited
   in place.
   - Visual genes stay in the genome and keep mutating even though nothing
     renders them: otherwise points evolved here would be half-blind when
     opened in the browser, and the search space would differ between the
     two apps. The console shows them as text.
3. **The FX chain is our own DSP, not a re-implementation of WebAudio.**
   Biquads (RBJ cookbook) for the filter and the formant bank, a comb with a
   fractional delay, chorus/flanger and phaser as delay/allpass stages driven
   by their own LFO, a feedback delay, an **FDN reverb instead of the
   convolver**, and a simple peak limiter with look-ahead instead of
   `DynamicsCompressor`.
   - Consequence, accepted up front: **the sound will not be identical** to
     the browser, most audibly in reverb tails and in how the limiter holds
     loud points. The presets were tuned against the browser chain and will
     need a listening pass.
   - Parity is stated as behaviour, not as samples: the same point, rendered
     here and in the browser, must land within the presets' fractality band
     (0.79 ± 0.14, see ../synesthesia/AGENTS.md) and within ±3 dB RMS.
   - **Measured** (`scripts/parity.mjs`, 12 presets, 8 s at 48 kHz): mean
     |Δ| 1.7 dB, every preset within ±2.1 dB, mean fractality 0.61 in the
     browser against 0.55 here — and the browser's own score for one preset
     moves by ±0.3 between runs, since its noise generators are unseeded.
     Three things had to be *copied* rather than invented to get there, each
     found by measuring, none of them obvious:
     - the limiter's **make-up gain** — Blink boosts by ~6.8 dB at the
       default threshold, and without it every point sat that much lower;
     - the convolver's **normalization** — its wet signal is far *below* the
       dry one (0.16 of it at decay 0.5 s, 0.39 at 8 s), and an FDN
       normalized the textbook way came out 16 dB hot;
     - the **render quantum inside every feedback loop** — Web Audio breaks a
       cycle once per 128 samples, so each loop carries that much latency on
       top of its delay time. The comb was +3.4 dB and resonating on the
       wrong partials until the port did the same; with it, filter-only lands
       within 0.1 dB.
   - The metric itself is a faithful port: scored on identical samples, our
     `analyze_sound` and the web app's `analyzeSound` agree to 0.000.
4. **The generators are a literal port and are verified bit-exact.** They are
   pure, per-sample, and `mulberry32` ports exactly, so each of the 21
   formulas is diffed sample-by-sample against a reference WAV rendered by
   the TypeScript (`npm run analyze -- --wav`). This is the cheap way to
   catch the expensive mistakes — a wrong parameter scale, a dropped phase
   accumulator.
5. **Architecture is built around a feature bus and a `Visualizer` trait, so
   the future renderer is a consumer and not a rewrite.** The engine
   publishes, at frame rate, exactly the features `src/audio/features.ts`
   publishes today — loudness, swell, brightness, low/mid/high, onset
   envelope and discrete hits — with the same names and units. The TUI is the
   first `Visualizer`; a GPU renderer will be the second. The bus is
   therefore exercised from day one rather than designed on paper.
6. **Realtime discipline.** The audio thread never allocates, never locks,
   never logs. Control → audio goes through a triple buffer of `EngineState`
   plus an SPSC command queue; audio → UI goes through an SPSC ring of
   feature frames. The 2 s genome morph is evaluated *on the audio thread*
   from `(from, to, t)`, so it cannot glitch when the UI is busy.
7. **One clock.** LFOs stay pure functions of absolute time (the sibling
   project's design), and that time is the audio sample clock. Nothing is
   messaged to keep the UI and the sound in sync; both read the same clock.
8. **The scout gets full-quality renders.** In the browser it had to use a
   24 s @ 8 kHz surrogate (ρ=0.73) because a real render was unaffordable.
   Here an offline render is ~20× realtime, so candidates are scored at
   30 s @ 22 kHz on the little cores via rayon, while the two A76s keep
   playing. The surrogate stays available behind a flag for comparison.
9. **Storage is local, and there is no network at all.**
   `$XDG_CONFIG_HOME/synesthesia/config.toml` holds the settings (sample rate,
   audio command, latency) and is written on the first run so it is visible
   rather than folklore. The current point is restored on the next start from
   `$XDG_DATA_HOME/synesthesia/last-point.json`, and the points the user keeps
   live next to it in `points.json`, each under the name they typed. No cloud
   sharing, no Worker, no HTTP client, no point ids and no hashing — a point
   is identified by its name here and by its file elsewhere.
   - Points are JSON, not TOML: a point *is* the web app's JSON (decision 2),
     and a TOML rendering of a 500-gene nested state would be neither readable
     nor compatible. TOML holds only what a person would hand-edit.
10. **Measure first** — carried over verbatim from the sibling project. Every
    performance or audio claim in this repo comes with a number produced by a
    checked-in bench, not by an impression. Thresholds are stated as
    behaviour, so a change that breaks the feel fails a test.

## Architecture

```
syn-core/     no I/O, no threads, deterministic — the whole model
  dsp/        21 formulas (per-sample), gate, mulberry32
  modmatrix/  LfoDef/ModRoute/effective params (pure fn of absolute time)
  fx/         filter+formants+comb, chorus/flanger, phaser, delay, FDN
              reverb, limiter — block-rate params, per-sample audio
  state/      AppState v1, sanitize/clamp, config + points files
  genome/     genes, codec, evolve (mutate/repair/lerp), explorer
  analysis/   FFT, spectral slope, Higuchi FD, box counting, fractal score
  features/   analyser emulation → AudioFeatures + OnsetDetector
  sim/        visual params only, for now (the renderer joins it later)
syn-audio/    device (cpal → PipeWire/ALSA), realtime thread, block
              scheduler, offline render, WAV dump
syn-tui/      ratatui/crossterm screens; implements Visualizer
syn-app/      bin `synesthesia`: CLI, storage, wiring, clock
syn-viz-*/    BACKLOG: gpu (wgpu) and cpu renderers; also Visualizer
```

```
 control thread            audio thread (RT)             render threads
 ┌──────────┐  cmds/SPSC   ┌───────────────┐  features   ┌────────────┐
 │ TUI +    │ ───────────► │ generators →  │ ──ring────► │ Visualizer │
 │ explorer │ ◄─────────── │ FX → master   │             │ (TUI now,  │
 └──────────┘  point/state └───────────────┘             │  GPU later)│
        │ triple buffer                                  └────────────┘
        └── scout (rayon, little cores) ── offline renders → scores
```

Dependencies are kept to: `cpal`, `ratatui` + `crossterm`, `serde`/
`serde_json`, `toml`, `base64` (the `#s=` import token only), `rayon`,
`rtrb` (or equivalent SPSC). Adding anything else is a decision, not a
convenience.

## Console UI

```
┌ synesthesia ── Fractal garden ─────────────────────── ♪ playing ─┐
│ loud ████████████▏      swell ▲ 1.3   bright ▍▍▍▍     hits 14/20s│
│ ▂▃▅▇█▇▅▃▂▁▂▃▄▅▆▇█▇▆▅▄▃▂▁▂▃▄▅▆▇█  (64-band log spectrum, 30 fps)  │
│ formulas  risset ●  lorenz ●  noiselp ●  bell ○  shepard ●       │
│ lfo 1 ▁▂▃▄▅▆▇█▇▆▅▄▃▂▁ 77 s tri → logistic.r, reaction.feed       │
│ fx  filter·chorus·reverb·delay·phaser·limiter    fractality 0.81 │
├──────────────────────────────────────────────────────────────────┤
│ [space] sound  [l] 👍  [d] 👎  [r] 🎲  [u] undo  [s] save        │
│ [p] points  [i] details  [e] export token  [?] help  [q] quit    │
└──────────────────────────────────────────────────────────────────┘
```

- The meters and the spectrum are drawn from the **same feature frames** a
  GPU renderer will consume (decision 5) — the console is the first picture,
  not a placeholder with its own data path.
- Redraw is capped at 30 fps and happens on the control thread; a slow
  terminal must never be able to stall the sound.
- Everything is keyboard-driven, no mouse. `--no-tui` plays a point and
  prints nothing, for scripting and for the bench.

## Phases

0. **Scaffold** ✔ — workspace, `scripts/check.sh` (fmt + clippy + tests) as
   the equivalent of `npm run check`, and the dumps from the running web app,
   all committed: `assets/presets.json`, `assets/schema.json` (ranges,
   defaults, the gene list), `assets/genomes.json` and the 63 golden takes in
   `golden/`.
1. **Generators** ✔ — all 21, diffed sample by sample against the golden
   takes at 48 kHz, at 8 kHz with their slow events forced to fire, and with
   an LFO route on a slider. Worst difference 1e-6; the chaotic ones needed
   no exemption.
2. **Mod matrix + FX + master** ✔ — offline render, WAV and `--score`,
   loudness and fractality checked against the browser (decision 3).
3. **Realtime output** ✔ — a render thread writing into `pw-cat`, commands
   and frames on bounded channels, replaced points dropped off the audio
   thread. No ALSA headers on this machine, so `cpal` waits behind the same
   `Sink` trait; the 30-minute xrun soak is still to run.
4. **TUI** ✔ — the screen, the keys, the feature bus, meters and spectrum.
5. **Genome + explorer + storage** ✔ — 👍/👎/🎲/undo with a 2 s morph, the
   config, the kept points, `#s=` tokens.
6. **Scout** ✔ — rayon, full-quality renders (30 s at 22 kHz), a version
   check that drops what the last press made stale. Measured: 3 + 3
   candidates scored in 1.3–1.6 s.

## Performance budget (thresholds, checked by the bench)

| metric | target | in the browser | measured here |
|---|---|---|---|
| live point, one A76 core | ≤ 15% | ~95% | 12.2% worst (*Fractal garden*), 36% measured live with the TUI and the pipe sink |
| xruns at 48 kHz / 1024 frames | 0 in 30 min | dropouts under load | 0 in a 2-minute soak, with the picture on too; **6–10 in 2 minutes when every press starts the scout** (GRAPHICS.md, V3); the long one is still to run |
| offline render | ≥ 8× realtime | 1.1–1.6× | 8.2–49× (`synesthesia bench`) |
| scout, 3 + 3 candidates | < 3 s wall, full quality | ~15 s CPU, surrogate quality | 1.3–1.6 s at 30 s / 22 kHz |
| startup → first sound | < 300 ms | seconds | not measured yet |
| TUI redraw | < 2 ms | — | not measured yet |

The first and third rows started as ≤ 5% and ≥ 20× — a guess, corrected by the
first measurement, which is what this project does with guesses. The generators
are the floor: a bit-exact port (decision 4) has to call `sin` as often as the
browser does, and `sin` costs 24.6 ns on this CPU. *Fractal garden* sums up to
40 harmonics with two sines each, so ~98 sines a sample, and 2.4 µs a sample is
what that costs. The win over the browser is real but it is ~8× on such a
point, not ~25×; the quiet points reach 60×.

Not done, deliberately: a recurrence for the additive formula's harmonic sum
(~10× on that generator) would end bit-exactness with the browser. If the live
cost ever matters, that is the lever — behind a flag, with the exact path kept
for the golden test.

## Backlog — graphics

**Trigger:** a real GPU render node exists (`/dev/dri/renderD128`) with a
GLES/Vulkan driver. Until then a CPU renderer tops out around 14 fps at a
512×288 grid and 720p output (measured C model in ../synesthesia/TODO.md), so
it is worth building only as a deliberate fallback, not as the main path.

- `syn-viz-gpu` on `wgpu`: port the seven passes (seed, paramfield, velocity,
  react, advect, inject, display) from `../synesthesia/src/sim/shaders/`.
- `syn-viz-cpu` as the fallback, carrying the optimizations already written
  down in ../synesthesia/TODO.md: cached/quarter-resolution noise fields,
  output decoupled from screen size, NEON + rayon kernels.
- Both implement `Visualizer` and read the feature bus, so nothing in the
  audio path changes when they land.
- Windowing decision deferred with them (DRM/KMS on a free VT vs a plain
  window under X) — deliberately *not* fixed now, because `/dev/fb0` under a
  running X server is a conflict, and the present path was measured to be
  ~27 ms/frame in the browser, i.e. never the bottleneck.

## Don'ts

- Never allocate, lock, or log on the audio thread.
- Never write an oscillator as `sin(2π·f·t)` with absolute `t` — accumulate
  phase. This was the sibling project's most expensive bug (harsh beating
  after minutes), and its `tests/continuity.test.ts` is the port target.
- Never change the `AppState` shape or the gene order: point files and the
  web app depend on both.
- Never hand-edit `assets/presets.json` — re-dump it from the web app.
- Don't tune by ear alone: the numbers come first, the listening pass second.
