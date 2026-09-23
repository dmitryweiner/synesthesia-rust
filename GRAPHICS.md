# Graphics in the terminal — plan

The picture of the web app, drawn in the console: the same Gray–Scott field,
the same four visual cards, the same sound → image couplings, rasterized on
the CPU at the resolution a terminal has and shown in half-block characters.
It is a stopgap for the GPU renderer in PLAN.md's backlog, not a replacement:
when a render node appears, `syn-viz-gpu` becomes the second implementation
of the same `Visualizer` and this one stays as the fallback.

Why it is affordable when the CPU renderer in the backlog was not: that one
was sized for 512×288 grids and 720p output (69 ms a frame on all eight
cores). A 120×40 terminal holds 120×80 pixels. The web app's own lowest
quality rung is a 192-cell grid (`src/sim/quality.ts`), and the grid planned
here (240×120 at 120×40) is above it — this is the web picture at its
smallest, not a different picture.

## The probe (2026-09-23)

`syn-tui/examples/field_probe.rs` runs a real Gray–Scott field at 2× the
pixel grid, draws it in `▀` cells through ratatui (the same cell diff the app
uses) and counts the bytes written. `scripts/term-cost.py` opens it in a
separate `xfce4-terminal --disable-server` window at 120×40 and reads the CPU
of that terminal, of Xorg and of xfwm4 from `/proc` over 18 s. Every row
below is one run; Xorg's own background load on this box moves by ~±10% of a
core between runs, so read its column as a trend.

| what runs | written to the tty | terminal | Xorg | xfwm4 | our process | draw, wall ms/frame |
|---|---|---|---|---|---|---|
| empty window (`sleep`) | — | 0% | 40–48% | 2% | — | — |
| the app today, `ui_fps` 8 | ~2 KB/s | 14% | 53% | 3% | (audio etc.) | — |
| field, 8 fps, truecolor, 8 bit, exposure breathing | 654 KB/s | 35% | 57% | 4% | 29% | 30.0 |
| same, no breathing | 574 KB/s | 34% | 54% | 4% | 28% | 27.7 |
| 8 fps, truecolor, 5 bit, breathing | 545 KB/s | 34% | 56% | 4% | 28% | 25.2 |
| 8 fps, truecolor, 5 bit, no breathing | 431 KB/s | 33% | 59% | 4% | 27% | 21.7 |
| 8 fps, 256 colours, breathing | 170 KB/s | 36% | 57% | 4% | 21% | 6.6 |
| 8 fps, 256 colours, no breathing | 129 KB/s | 36% | 56% | 4% | 20% | 3.4 |
| 4 fps, truecolor, 8 bit, breathing | 263 KB/s | 24% | 51% | 3% | 21% | 34.6 |
| 4 fps, 256 colours, no breathing | 58 KB/s | 26% | 51% | 3% | 13% | 3.8 |

(120×40 window, field 120×30 cells = 120×60 pixels, grid 240×120, 18 s per
row. CPU is % of one core.)

What it says:

- **The terminal pays per frame, not per byte.** Five times fewer bytes
  (654 → 129 KB/s) leave VTE at ~35%; halving the frame rate takes it to
  ~25%. A frame in which most cells change costs a full repaint however
  short its escapes are. So quantization and 256 colours buy the terminal
  nothing, and `viz_fps` is the one knob that does.
- **A growing pattern changes almost every cell every frame.** Dropping
  three bits per channel, or the exposure breathing, cuts the bytes by only
  15–35%: the ratatui diff has little to skip.
- **Bytes cost *us*, though:** at truecolor a draw spends 25–35 ms of wall
  time, most of it blocked in `write` while the terminal drains the pty (our
  CPU goes up by only ~8% of a core). At 256 colours it is 3–7 ms.
- Xorg adds ~10–15% over the empty window, within its own noise.

The probe's own cost (simulation plus colour, single-threaded, unoptimized:
bounds checks everywhere, five neighbour lookups per pixel) is not the
number to budget against — see V1's target.

## Decisions

1. **Half-blocks, not Braille, not sixel.** `▀` with a foreground and a
   background colour gives two square pixels per cell and a full colour for
   each. Braille gives 2×4 dots but one colour a cell — wrong for a colour
   field. Sixel and the kitty protocol are not relied on: VTE has sixel only
   as a build option, and a terminal that lacks it shows garbage.
2. **Truecolor at full depth, and `viz_fps` as the one knob.** The
   probe shows that fewer colours do not make the terminal cheaper, so the
   picture keeps them. `viz_color = "256"` exists only as the fallback,
   picked automatically when `COLORTERM` does not say `truecolor`.
   `viz_fps` defaults to 8 (the same as `ui_fps`); 4 takes ~10% of a core
   off the terminal. Because a truecolor frame blocks in `write` for tens of
   ms, the draw must never hold anything the keys or the scout wait on.
3. **The simulation grid is 2× the pixel grid, box-filtered down.** A
   Gray–Scott pattern has a fixed size *in cells* (set by the diffusion
   rates), so a 1:1 grid shows a handful of blobs; 2× shows the pattern.
   The long side is capped (384 cells) so a huge terminal does not buy a
   huge grid, and the down-filter adapts.
4. **The simulation clock is not the redraw clock.** The web steps the field
   once per animation frame, so how fast a pattern grows depends on the
   frame rate there. Here the field steps at a fixed `sim_hz` (30 by default,
   `speed` substeps each) and is shown at `viz_fps`. Changing the redraw
   rate to save the terminal must not slow the pattern down.
5. **The field runs on its own thread**, not on the control thread: the
   control thread keeps the keys, the morph and the scout, and the field
   would take tens of ms a second from them. It publishes an RGB image at
   the pixel grid through a double buffer (a mutex is fine — this is not the
   audio thread); the TUI only copies colours into cells.
6. **Seeded, not `Math.random()`.** Seed spots, onset positions and reseeds
   use mulberry32 from the point's seed, so a test and a bench see the same
   field every time. Parity with the browser is by eye, not by sample —
   the browser's field is unseeded and cannot be compared bit for bit.
7. **Inputs are exactly the web app's inputs.** Visual card params go
   through `modmatrix::effective_params` (the LFOs already address visual
   cards) and then through a port of `applyCoupling`; the display effects
   come from a port of `displayCoupling`; onset hits are the `hits` counter
   of `Frame` (a delta between two frames = that many hits, each one an
   `inject` and a ripple, as `seedOnHit` does). During a 2 s morph the field
   reads the morphing point, like the sound does.
8. **Reseed when the web app reseeds:** on loading a point and on 🎲.
   👍/👎 morph into the new visual genes without wiping the field.
9. **The `Visualizer` trait is introduced now.** PLAN.md decision 5 promised
   it from day one; it does not exist in the code yet. It takes one input
   per frame — clock time, features, new hits, the effective point — so a
   GPU renderer later consumes the same thing.

## Where the code goes

```
syn-core/src/sim/        pure, no threads, deterministic
  field.rs               Gray–Scott state, seed, inject, react substep
  noise.rs               hash21 / noise2 / fbm / warpedFbm / curl — the GLSL, in f32
  fields.rs              paramfield + velocity at half resolution, cached by key
  advect.rs              semi-Lagrangian, bilinear
  display.rs             palette, relief, gloss, tint, flash, exposure, ripples → RGB
  coupling.rs            applyCoupling + displayCoupling + RippleSet
  mod.rs                 Sim: one step(), one render(), the Visualizer input
syn-tui/src/field.rs     the half-block widget and the colour mode
syn-app/src/main.rs      the field thread, the keys, the config keys
```

## Phases

**V0. Probe ✔** — the table above. The probe and the measuring script are
kept: they are the bench for V3.

**V1. The field ✔** (`syn-core::sim`: field, noise, fields, advect).
- The react kernel with a fast interior (rows as slices, bounds checks
  hoisted, fused multiply-adds — it vectorizes to NEON) and a clamped border,
  the zero-flux boundary `CLAMP_TO_EDGE` gave the shader.
- Paramfield and velocity at half the grid's side, recomputed only when
  their key (params + `evolveT`) changes, skipped when their card is off or
  their amount is zero — everything `src/sim/engine.ts` learned the hard way.
- Tests: `u=1, v=0` is a fixed point; the interior kernel agrees with the
  clamped one; advect with no velocity is an exact copy and with no amount is
  skipped; an off card never draws its field; every preset stays in `[0, 1]`
  with no NaN at the corners of the reaction sliders and the fastest flow;
  every preset is still alive after 10 s; same seed → same field.
  `syn-core` is built at `opt-level = 1` in the dev profile so these run in
  about a second rather than fifteen.
- Bench: `taskset -c 6 synesthesia bench --sim`.

Measured at 240×120, 30 Hz, one A76 core (2026-09-23):

| | ms/step | % of a core |
|---|---|---|
| reaction alone | 5.2–5.5 ns a cell a substep | — |
| presets without Field variation or Flow | 1.5–1.8 | 4.5–5.4 |
| *Fractal garden* (preset 0, speed 16) | 3.5 | 10.6 |
| heaviest, *Loom & copper* (speed 24) | 4.7 | 14.2 |

Target was ≤ 10% for the default point: met for speed ≤ 12, ~11% at
preset 0, 14% at the heaviest. What it took, in order of what it bought:

- **Redrawing the fields at most every 6 steps** (`FIELD_REFRESH_STEPS`,
  5 times a second). An evolving point redrew both every step, which was
  two thirds of a step for a drift of ~1e-4 per step. 18.7% → 12.9% on
  preset 0.
- **The velocity sampled once per redraw, not per cell per step:** it now
  lives as a full-resolution map in cells per step. Advection went from 47
  to 23 ns a cell.
- FMA in the kernel: ~4%.

Two findings for V3:
- **The kernel is compute-bound, not memory-bound:** 5.5 ns a cell at
  240×120 (675 KB of state), 7.5 at 480×240, 8.6 at 60×30 where the clamped
  border is a larger share. What is left is micro-optimization.
- **The little cores are 4.5× slower** (preset 0 is 47% of an A55). The
  field thread must be free to run on an A76 — it must not be pinned to the
  scout's cores.

**V2. The colour ✔** (`palette`, `coupling`, `display`, `frame`, `picture`).
- The five palettes and `composePalette`; `applyCoupling` (hue and light
  angle wrap), `displayCoupling`, `RippleSet`; `display.frag` line by line,
  per pixel of the picture. A bilinear lookup at a pixel's centre on a grid
  twice as fine *is* the 2×2 box filter, so decision 3's down-filter costs
  nothing extra.
- The relief is computed in the web's Y-up terms on a Y-down grid, so the
  light comes from the same side of the screen as in the browser — a test
  lights a ramp from each side and checks which one is brighter.
- `frame_params` derives one frame's inputs the way the web app's `loop()`
  does: card params through the LFOs, then the couplings, off cards as zeros.
  `Picture` wraps the simulation, the ripples and the image behind `step`
  (sim clock: new onset hits become growth and ripples) and `draw` (redraw
  clock) — what V3's thread will run.
- `synesthesia render --preset N --secs S --picture out.ppm [--size WxH]`
  drives the picture with the point's own offline sound and writes the last
  frame. All twelve presets after 20 s show their pattern families: spots,
  mazes, coral, worms; the drifting presets leave dead substrate along the
  top edge, which the browser's `CLAMP_TO_EDGE` advection should do too —
  to check in V4.
- Tests: ported `coupling.test.ts`, `visualfx.test.ts` and the palette cases
  of `visual.test.ts`; a flat field comes out as the colour computed by hand;
  exposure, flash and tint act where they should; a ripple is gone at the
  end of its life; LFO routes and the loudness coupling reach the params.

Cost at 120×60 pixels, 8 fps: **1.4 ms a draw, ~1% of an A76**. Keeping
feed/kill as two scalars plus per-cell offsets (not two per-cell maps) took
*Fractal garden*, whose LFOs sweep feed, back from 4.0 to 3.5 ms a step.
With the draws, the heaviest preset is 15.9% of a core, preset 0 11.7%.

Found on the way, the same in the browser: `brightToShift` is centred on
brightness 0.5, so in silence (brightness 0) the hue sits shifted by half
the gene rather than at the card's value.

**V3. On screen ✔** (`Visualizer`, the thread, the widget, the keys).
- `syn_core::visualizer::Visualizer` — `step(VizInput)` and `reseed()`;
  the input is the point as it sounds (mid-morph included), the features,
  the hit counter and the engine clock. `Picture` is the first one.
- `syn-app/src/picture.rs`: the thread steps at `sim_hz` on its own clock,
  drops steps rather than racing when it falls behind, draws at `viz_fps`
  into a buffer the interface copies from, and idles while the picture is
  off. The control thread hands it the point (only when it changed), each
  feature frame and the size the layout gave the picture.
- `syn-tui/src/field.rs`: `▀` cells, truecolor or the nearest xterm-256
  colour (cube or grey ramp). The layout gives the picture whatever height
  the text leaves in the panel, or the whole window minus a status line.
- `v` cycles off → panel → full and writes `viz` to the config. Loading a
  point and 🎲 reseed the field, as in the browser; 👍/👎 morph it.
  Resizing resamples the field instead of reseeding it.
- Config: `viz`, `viz_fps` (8), `sim_hz` (30), `viz_color` ("auto").

Measured with `scripts/term-cost.py idle app:off app:panel app:full`
(120×40 window, `--no-sound --no-scout`, the last point — speed 16, both
optional cards on; 2026-09-23):

| | terminal | Xorg | picture thread | whole app |
|---|---|---|---|---|
| empty window | 0% | 50% | — | — |
| picture off | 14% | 52% | 0.2% | 67% |
| panel (118×48 px) | 35% | 63% | 49% | 116% |
| full screen (120×78 px) | 36% | 66% | 32% | 102% |

- The terminal lands where the probe said it would: ~35% at 8 fps, the
  same for the panel and the full screen — per frame, not per pixel.
- **The picture thread runs on the little cores.** Sampling its CPU number
  every 20 ms: 93% of the time on 0–5 (A55), 7% on 6–7 (A76). The kernel's
  energy-aware scheduler sees a periodic load it can fit on a little core and
  puts it there, where a step costs 4.5× more (V1) — ~50% of an A55, which
  is the bench's ~11% of an A76. The panel/full difference is where it
  happened to be scheduled, not the pixel count. A step takes ~16 ms of its
  33 ms at 30 Hz there, so the pace holds. Pinning it to the A76 cores would
  need an affinity call (a dependency or `unsafe`), and is not done.
- The control thread does not notice the picture: the whole app minus the
  picture thread is 67% with it and without it.
- Not done: the 2-minute xrun soak with the sound and the scout running
  (the player keeps no xrun count; it needs PipeWire's own).

**V4. Side by side.** Every preset in the browser (lowest rung) and here,
by eye: the same pattern family, the same palette, the same response to
onsets and swells. Findings go into this file, as PLAN.md decision 3 did for
the sound. Then PLAN.md: decision 1 gains "…and a picture in the terminal",
the backlog points here.

## Not now

- Painting with the mouse (the web app's touch strokes) — the console is
  keyboard-only by decision.
- Sixel / kitty graphics as an optional sharper mode — only if a terminal on
  this box turns out to support it, and only behind a config key.
- The web's quality ladder and boot probe: a terminal's size is the ladder.
