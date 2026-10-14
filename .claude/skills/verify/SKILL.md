---
name: verify
description: Run synesthesia-rs's verification ladder before committing — fmt/clippy/tests, the audio bench, the picture's bench and a look at every preset's picture, the terminal cost, and the xrun soak — choosing the rungs the change can break, and reading the failures the way this project's past ones read. Use after changing anything in syn-core, syn-audio, syn-tui, syn-app or scripts/, and before any commit.
---

# Verify a change

Run only the rungs a change can break, cheapest first. Binaries live in
`~/.cache/cargo-target/release/` (not `./target`).

## 1. Always

```bash
./scripts/check.sh      # cargo fmt --check, clippy --all-targets -D warnings, cargo test (~1 min)
```

- `fmt` failing is not an error to argue with: run `cargo fmt`, then
  re-read any file you are about to edit again (it reflows long lines, and
  edits anchored on the old text stop matching).
- The model (`syn-core`) is synesthesia-core's, pinned by `rev` in
  Cargo.toml: its own checks (`../synesthesia-core/scripts/check.sh`,
  golden takes, presets, the picture's invariants) run there. A change to
  the sound or the picture is made there first; here, bump `rev` and run
  the rungs below on this machine.

## 2. The sound changed (dsp/, fx/, engine, modmatrix, features)

```bash
cargo run --release -- bench                     # × realtime per preset; slowest must stay ≥ 8×
cargo run --release -- render --preset 0 --secs 30 --score
```

## 3. The picture changed (syn-core/src/sim/, visualizer.rs)

```bash
taskset -c 6 ~/.cache/cargo-target/release/synesthesia bench --sim --secs 8
scripts/picture-sheet.py /tmp/…/sheet.png --secs 20      # use the scratchpad dir
```

- Compare `bench --sim` with the table in GRAPHICS.md (V1/V2): preset 0
  ~3.5 ms a step and ~1.4 ms a draw at 240×120 on an A76. Always `taskset`
  it: unpinned, it may land on an A55 and read 4.5× worse.
- **Read the sheet PNG.** Each preset should show its pattern family (spots,
  mazes, coral, worms) in its palette. A uniform colour means the field
  died or the display lost its input; stripes along one edge on drifting
  presets are expected (clamped advection, as in the browser).
- There are no golden samples for the picture (the web seeds it with
  `Math.random()`); the invariant tests in `sim/` are the contract.

## 4. The screen or the redraw changed (syn-tui, the loop, picture thread)

```bash
cargo build --release && cargo build --release --example field_probe -p syn-tui
SHOT=/tmp/…/shots scripts/term-cost.py idle app:spectrum app:panel app:full
```

Opens windows on the user's screen — say so first. Then Read the
screenshots. Expected (GRAPHICS.md V3): terminal ~14% with the spectrum,
~35% with the picture at 8 fps; the app minus its picture thread unchanged.

## 5. Threads, the scout, or anything the audio thread shares a core with

```bash
scripts/soak.py --keys ldldrldldldl            # 130 s, real sound, a press every 10 s
```

Plays sound through the speakers and opens a window — ask first. Expect
`pw-cat ERR` to stay where it was at 6 s. If it climbs, rerun with one
thing changed at a time (`--viz spectrum`, `--keys ''`, `--scout false`,
`--scout-threads N`) before blaming anything: that is how the scout, not
the picture, was found to be the cause.

No device at hand (macOS, no PipeWire): `scripts/soak.py --null --keys
ldldrldldldl` — no sound, no window; the null sink models a 1024-frame
buffer and counts underruns. Single ones happen (1–3 a run, before and
after decision 12 alike, PLAN.md), so compare against a control build
(`--bin`, a worktree of the previous commit) over three runs each. It says
whether the audio thread keeps up on this machine, not the device's xruns.

## 6. Commit

Milestone granularity, a message that carries the numbers, the attribution
line. Update PLAN.md / GRAPHICS.md / README.md when behaviour, a decision or
a measured number changed.
