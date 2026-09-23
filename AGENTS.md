# AGENTS.md — project map and working rules

(Claude Code reads CLAUDE.md, which points here; keep everything in this one
file so the two never drift apart.)

Synesthesia-rs: a native console port of [../synesthesia](../synesthesia/) —
one point in a ~500-gene space makes sound (21 formula generators, our own FX
chain) and a picture (Gray–Scott reaction-diffusion, drawn in half-block
characters), and 👍/👎 steer the search. Decisions agreed with the user live
in **PLAN.md** (sound, architecture, budget) and **GRAPHICS.md** (the
picture) — read them before changing behaviour. README.md is the
human-facing spec. Docs, UI strings and code comments are in English; the
user talks to agents in Russian.

## Commands

```bash
./scripts/check.sh                   # fmt --check + clippy -D warnings + tests — after every change
cargo run --release -- bench         # every preset: × realtime, rms, fractality
taskset -c 6 ~/.cache/cargo-target/release/synesthesia bench --sim
                                     # the picture: ms/step, ns/cell, ms/draw, % of ONE A76
cargo run --release -- render --preset N --secs 20 --picture p.ppm [--size WxH]
                                     # the picture driven by the point's own offline sound
scripts/picture-sheet.py out.png     # all 12 presets' pictures on one PNG — then Read it
scripts/term-cost.py idle app:spectrum app:panel app:full [FPS:COLOR:BITS:BREATHE]
                                     # CPU of the terminal, Xorg, xfwm4, the app and its
                                     # picture thread, in a real 120x40 xfce4-terminal window;
                                     # SHOT=dir leaves screenshots (Read them)
scripts/soak.py --keys ldldrldldldl  # xruns with real sound, from PipeWire's ERR counter
node scripts/parity.mjs              # loudness + fractality against the browser (needs the web app)
```

Binaries are in **`~/.cache/cargo-target`** (`build.target-dir` in
`~/.cargo/config.toml`), not `./target` — use `cargo metadata` in scripts
rather than hard-coding either. Never point a build at `/tmp`: it is a
2.9 GB tmpfs and the machine starts swapping.

Two skills carry the routines: **`verify`** (what to run for which change,
and how to read a failure) and **`measure`** (how to get a number for cost,
the terminal, or xruns that can be trusted).

Commits at phase/milestone granularity are fine and expected; say so in the
reply. Push only when asked. Agreed decisions go into PLAN.md / GRAPHICS.md
with the numbers behind them.

## How work is done here — measure first

PLAN.md decision 10. Every choice in the picture's port came from a number,
and the numbers repeatedly said something other than the intuition:

| question | measurement | result |
|---|---|---|
| can a terminal show a colour field at all? | `field_probe` + `term-cost.py` before any port | yes: ~35% of a core for the terminal at 8 fps |
| do 256 colours / fewer bits make the terminal cheaper? | the probe matrix | **no** — 654 → 129 KB/s left VTE at ~35%; it pays per frame, not per byte; only `viz_fps` helps |
| where did a step go? | `bench --sim` + a throwaway split by feature | noise fields redrawn every step for a ~1e-4 drift: 2/3 of the step |
| why is advection 47 ns a cell? | same | a bilinear sample of the velocity per cell per step; a per-cell map made it 23 |
| why did preset 0 get slower once LFOs applied? | same, with `Picture` instead of `Sim` | an LFO on feed rebuilt two per-cell maps every step; scalars + offsets fixed it |
| is the kernel memory-bound? | ns/cell at 60×30 … 480×240 | no: fastest at 240×120 (675 KB); compute-bound |
| why is the picture thread 50% live when the bench says 11%? | CPU number of the thread every 20 ms (`/proc/…/task/…/stat` field 39) | the scheduler keeps it on the A55s (93%), where a step is 4.5× dearer |
| does the picture cause xruns? | `soak.py`, one variable at a time | no — the scout did (rayon on every core); its own pool of cores−2: 4 → 0 |

Corollaries:
- **Build the probe before the port.** A 200-line throwaway that draws a
  real Gray–Scott field priced the whole plan before a line of it existed.
- **Change one variable per run, and keep a control.** The xrun cause only
  showed up once "picture, no presses" (0) and "no picture, presses" (6)
  were run next to each other.
- **Say which core a percentage is of.** An A55 is ~4.5× slower than an A76
  here; `taskset -c 6` for bench numbers, and look at where a live thread
  actually runs before comparing it with the bench.
- **Keep the bench** (`bench --sim`, `term-cost.py`, `soak.py`), not a
  one-off script: the next session starts from numbers.

## Pitfalls that already cost time

- **The terminal's cost is outside this process.** Measure the terminal
  emulator, Xorg and xfwm4 (`term-cost.py` does), in a real window of a
  known size. Xorg idles at 40–50% here and moves ±10% between runs — the
  agent's own terminal redraws while tools run — so read it as a trend.
- **A pty with no window size draws nothing** (ratatui sees 0×0). Set
  `TIOCSWINSZ` (the scripts do); plain `script` measured "no cost at all".
- **The tty output cannot be grepped for status text**: ratatui rewrites only
  the cells that changed, so "scouted … in 1.5 s" never appears whole. Read
  thread CPU from `/proc` instead, or put the number somewhere readable.
- **`term-cost.py` and `soak.py` open windows on the user's screen, and
  `soak.py` plays sound.** Say so before running them.
- **The scout runs once after start and then only after a press.** A soak
  without key presses never exercises it; `soak.py --keys` presses them.
- **Script edits: assert the anchor.** A Python `s.replace(a, b)` whose `a`
  came out empty (slice bounds in the wrong order) inserted `b` between
  every character of `main.rs`. Check `a` is non-empty and occurs exactly
  once, or use the Edit tool; `git checkout` the file if it goes wrong.
- **`cargo fmt` reflows long signatures**, so an edit anchored on the
  pre-format text stops matching; re-read the file before editing it again.
- **The presets route LFOs onto visual params** (Fractal garden sweeps feed
  and curl): a test that expects "the card's own values" must clear
  `modulation.routes` first.
- **Silence is not neutral for colour**: `brightToShift` is centred on
  brightness 0.5, so brightness 0 shifts the hue by half the gene — the
  browser does the same. Use `brightness: 0.5` for a neutral sound in tests.
- **Y grows down here, up in the web's textures.** Positive drift is "down"
  as the slider says (the web negates it); the relief is computed in Y-up
  terms so the light falls from the same side of the screen. Tests pin both.
- **Unoptimized tests are 4× slower** — `syn-core` is built at
  `opt-level = 1` in the dev profile for its whole-simulation tests.
- **`unsafe_code = "forbid"` and "a dependency is a decision"**: no thread
  affinity or priority calls. The scout was fixed with a smaller rayon pool
  instead; pinning the picture thread was left undone for this reason.
- **`Math.random()` in the web's picture** means no golden samples for the
  field: check it by invariants, by `bench --sim`, and by eye
  (`picture-sheet.py` next to the browser).

## Module map

```
syn-core/src/dsp/         21 generators (bit-exact with the TS), gate, mulberry32
syn-core/src/modmatrix.rs LFOs as pure functions of engine time; routes onto
                          fx, formulas and visual cards
syn-core/src/fx/          our DSP chain (biquads, comb, chorus, phaser, delay,
                          FDN reverb, limiter) — parity by behaviour, PLAN #3
syn-core/src/engine.rs    Engine: render blocks, features, onset hits, time
syn-core/src/features.rs  AnalyserNode emulation → AudioFeatures, OnsetDetector
syn-core/src/genome/      codec, evolve, explorer, scout (rayon; the caller
                          picks the pool — syn-app gives it cores−2)
syn-core/src/sim/         the picture: noise, field (Gray–Scott kernel + seed +
                          inject), fields (paramfield/velocity at half size,
                          cached, refreshed ≤ every 6 steps), advect, palette,
                          coupling, display (→ RGB), frame (per-frame params
                          as the web loop derives them), picture (`Picture`)
syn-core/src/visualizer.rs  the Visualizer trait + VizInput
syn-audio/                the sink (pw-cat/aplay pipe) and the render thread
syn-tui/src/app.rs        the screen, keys, VizMode (spectrum/panel/full)
syn-tui/src/field.rs      the half-block widget, truecolor / xterm-256
syn-tui/examples/field_probe.rs  the terminal-cost probe
syn-app/src/main.rs       CLI, the loop, bench, render --picture, scout pool
syn-app/src/picture.rs    the picture thread (sim_hz clock, viz_fps draws)
syn-app/src/store.rs      config.toml, last point, kept points
```
