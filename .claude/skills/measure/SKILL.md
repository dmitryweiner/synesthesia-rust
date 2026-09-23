---
name: measure
description: Get a trustworthy number on this machine (sunxi, 6×A55 + 2×A76, no GPU driver, xfce4-terminal under X) for CPU cost, terminal/X cost, which core a thread runs on, or audio xruns — and design the experiment so the number means what it seems to. Use before optimizing anything, before claiming a cost in docs or a commit, and whenever a live number disagrees with a bench.
---

# Measure before changing

PLAN.md decision 10: every performance claim needs a number from a
checked-in bench. These are the ways numbers have lied here, and the tools
that don't.

## Pick the right tool

| question | tool |
|---|---|
| cost of the sound, per preset | `cargo run --release -- bench` |
| cost of the picture, per preset | `taskset -c 6 …/synesthesia bench --sim --secs 8` |
| cost of a *new* part before it exists | a probe: `syn-tui/examples/field_probe.rs` is the model — the real kernel, the real draw path, a byte counter |
| what the terminal and X pay | `scripts/term-cost.py` (real `xfce4-terminal --disable-server` window, `/proc` ticks of terminal, Xorg, xfwm4, the app, its `picture` thread) |
| xruns | `scripts/soak.py` (PipeWire's ERR for the `pw-cat` node; the player keeps no count) |
| where a thread actually runs | sample `/proc/<pid>/task/<tid>/stat` field 39 (the CPU) every ~20 ms; match the thread by `comm` (`picture`, `syn-scout-N`, `syn-audio`) |
| a split nobody benches yet | a temporary `examples/*.rs` timing variants of the params, deleted after; keep the finding, not the file |

## Rules that came from mistakes

1. **Name the core.** Cores 0–5 are A55, 6–7 are A76; an A55 is ~4.5×
   slower on the picture kernel. Bench with `taskset -c 6`. A live thread is
   placed by the energy-aware scheduler — the picture thread sat on the A55s
   93% of the time, so "49% of a core" live was the bench's 11% of an A76.
2. **Measure outside the process.** A terminal repaints in software; its CPU
   and Xorg's are the real cost of a redraw. Bytes written are a proxy that
   can mislead: cutting them 5× (256 colours) left the terminal at 35%.
3. **Give a pty a size.** Without `TIOCSWINSZ` ratatui draws nothing and the
   measurement says "free".
4. **One variable per run, with a control.** Keep a baseline run in the same
   session (Xorg's idle moves 40–50% between runs). The xrun table in
   GRAPHICS.md is the pattern: vary picture, presses, scout, pool size.
5. **Split before optimizing.** Time the step with each feature off in turn
   (no flow / no field variation / neither / frozen fields) — the cost was
   never where the intuition put it (noise redraws, then per-cell velocity
   sampling, then map rebuilds under an LFO; the kernel itself was fine).
6. **Check memory- vs compute-bound** by timing the kernel over grid sizes
   around the cache sizes before reaching for layout changes.
7. **Verify the thing under test actually ran.** A soak with 0 xruns means
   nothing if the scout never started: confirm it from thread CPU bursts.
8. **Windows and sound are visible to the user.** `term-cost.py` and
   `soak.py` open windows; `soak.py` plays audio. Ask before running them.

## Record it

Put the table in the commit message and in PLAN.md / GRAPHICS.md, with the
date, the size, the preset and the core. State targets as behaviour ("the
default point at 120×40 costs ≤ 10% of an A76") so a later change that
breaks it can be caught by rerunning the same command.
