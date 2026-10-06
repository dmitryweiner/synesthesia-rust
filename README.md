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
cargo run --release -- render --list    # the built-in presets (15)
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

The model — the sound, the genome and the search, the picture, the presets
— is [synesthesia-core](https://github.com/dmitryweiner/synesthesia-core),
the same Rust crate the web app (as wasm) and the Android app run, pinned
here by revision in `Cargo.toml`. It carries the checks that used to live
here: the 21 generators diffed against the TypeScript's golden takes, the
genome against the web app's, every preset rendered, the picture's
invariants, and its own `syn-bench`.

This repository checks what is its own: the sink and the render thread,
the screen and its keys, the files (`store.rs`: a point file from an older
build still loads complete), `synesthesia bench` and `bench --sim` for the
cost on this machine, `scripts/term-cost.py` for what the terminal and X
pay, and `scripts/soak.py` for xruns.

## Layout

```
(syn-core)  the model, from synesthesia-core (pinned in Cargo.toml):
            generators, modulation, FX, engine, features, analysis, the
            point, the genome, the search, the scout, the picture.
syn-audio/  the sink and the render thread
syn-tui/    the screen, the key map, the half-block picture widget
syn-app/    the binary: CLI, storage, the loop, the picture thread
scripts/    check.sh; term-cost.py, soak.py,
            picture-sheet.py (see AGENTS.md)
```
