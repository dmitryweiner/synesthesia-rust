# Synesthesia — console

Sound and image from one point in a ~500-gene parameter space, steered from
the keyboard: 👍 when you like where it is going, 👎 when you don't, and the
search follows. A native port of [../synesthesia](../synesthesia/) for a
machine where the browser cannot keep up: the sound in full, the picture in
half-block characters in the terminal. [PLAN.md](PLAN.md) has why and the
decisions; [GRAPHICS.md](GRAPHICS.md) the picture's plan and measurements.

```
┌ synesthesia — Bell spots · 2 steps ─────────────────────── ♪ ─┐
│ loud ████████▏······  swell ▲0.31  bright ███▏····  hits 14   │
│ low ██▏·····  mid ███·····  high █·······  peak 0.074         │
│ ▂▃▅▇█▇▅▃▂▁▂▃▄▅▆▇█▇▆▅▄▃▂▁▂▃▄▅▆▇█                               │
│ formulas  risset ●  noiselp ●  bell ●                         │
│ lfo 1    77.0s triangle █████····· → logistic.r, reaction.feed│
│ fx  filter·chorus·reverb·limiter   master 0.75   42s          │
├───────────────────────────────────────────────────────────────┤
│ more of this · best of 3 scouted (fractality 0.72)            │
│ [space] sound  [l] like  [d] dislike  [r] surprise  [q] quit  │
└───────────────────────────────────────────────────────────────┘
```

## Running it

```bash
cargo run --release                     # the interface, on the last point
cargo run --release -- --preset 3       # …on a built-in preset
cargo run --release -- render --list    # the twelve presets
cargo run --release -- render --preset 0 --secs 30 --out point.wav
cargo run --release -- render --preset 0 --secs 30 --score   # its fractality
cargo run --release -- render --preset 0 --secs 20 --picture p.ppm   # its picture
cargo run --release -- play --preset 7  # sound without the interface
cargo run --release -- bench           # every preset: render speed, loudness, score
taskset -c 6 ~/.cache/cargo-target/release/synesthesia bench --sim   # the picture's cost, one A76
./scripts/check.sh                      # fmt + clippy + tests, after every change
```

Sound goes out through `pw-cat`, or `aplay` if PipeWire is not there; set
`audio_command` in the config to use something else. `--no-sound` renders
into nothing at the right pace, for measuring.

The screen redraws `ui_fps` times a second (8 by default). That number is
paid for outside this process — the terminal emulator repaints its window and
X composites it, both in software here — so it is the one knob that makes the
interface cheaper: 30 → 8 fps cuts what the terminal has to redraw from
6.4 KB/s to 2.1 KB/s. Key presses always redraw at once, whatever it is set to.

The picture — the web app's Gray–Scott field, in half-block characters —
takes the spectrum's place above the text, or fills the window; `v` cycles
spectrum → picture → full-screen picture, and the choice is kept. It costs
the terminal about a third of a core at the default 8 frames a second
(`viz_fps`; 4 costs a quarter) and its own thread 5–16% of a big core, which
the kernel usually runs as ~50% of a little one. GRAPHICS.md has the
measurements.

Builds land in `~/.cache/cargo-target` (set once for every project in
`~/.cargo/config.toml`), so the binaries are
`~/.cache/cargo-target/release/synesthesia` and `…/release/examples/…`.
Never point a build at `/tmp`: it is a 2.9 GB tmpfs here, and a build there
spends 2 GB of RAM and sends the machine into swap.

Keys: `space` sound · `l`/`d` 👍/👎 · `r` surprise · `u` undo · `s` keep this
point · `p` the points list · `v` spectrum/picture · `i` what this point is ·
`e` write a `#s=` token · `?` help · `q` quit.

## Where things are kept

| path | what |
|---|---|
| `~/.config/synesthesia/config.toml` | sample rate, audio command, latency, scout settings (`scout_threads`: 0 = every core but two), `ui_fps`, the picture: `viz`, `viz_fps`, `sim_hz`, `viz_color` |
| `~/.local/share/synesthesia/last-point.json` | the point you were on |
| `~/.local/share/synesthesia/points.json` | the points you kept |

Points are the web app's own JSON: a file written here opens there, and a
`#s=` token pasted from a browser link opens here. Nothing is sent anywhere.

## How it is checked

The sound is a port, so the tests compare it against the original rather
than against opinion:

- **The 21 generators are diffed sample by sample** against 63 reference
  takes rendered by the TypeScript (`golden/`, made by
  `scripts/dump-golden.mjs`). All 63 match within 1e-6.
- **The genome is the same genome**: every built-in preset, encoded here and
  in the web app, agrees on all 237 genes to 1.1e-16
  (`assets/genomes.json`, `syn-core/tests/genome.rs`).
- **The FX chain is our own DSP**, so it is judged on behaviour:
  `scripts/parity.mjs` renders the twelve presets in both and compares
  loudness and fractality, scoring both with the web app's own
  `analyzeSound`. The metric port agrees with it exactly on the same
  samples; the sound lands within ~1 dB on average.

The picture has no reference samples — the browser's field is seeded by
`Math.random()` — so it is checked by behaviour and by numbers instead: unit
tests ported from the web app's (couplings, ripples, palettes) plus the
invariants of the field (a fixed point, bounded at every slider's extreme,
every preset alive after 10 s); `synesthesia bench --sim` for its cost;
`scripts/term-cost.py` for what the terminal and X pay to show it; and a
look side by side with the browser.

Ranges, defaults, presets and the gene list are **dumped** from the web app
(`assets/`, `scripts/dump-presets.mjs`), never re-typed — so the two cannot
drift apart.

## Layout

```
syn-core/   the model: generators, modulation, FX, engine, features,
            analysis, the point, the genome, the search, the scout, and the
            picture (`sim/`, `visualizer.rs`). No I/O, no threads.
syn-audio/  the sink and the render thread
syn-tui/    the screen, the key map, the half-block picture widget
syn-app/    the binary: CLI, storage, the loop, the picture thread
scripts/    check.sh; the web-app dumps and parity; term-cost.py, soak.py,
            picture-sheet.py (see AGENTS.md)
```
