//! `synesthesia` — the console application.
//!
//! Phase 2 of PLAN.md: only the offline renderer is wired up, so the sound can
//! be measured before there is anything to listen to. Playback and the TUI
//! follow in phases 3 and 4.

use std::process::ExitCode;

use syn_audio::{NullSink, PipeSink, Player, Sink};
use syn_core::state::{presets, AppState};
use syn_core::{render_offline, wav};

const USAGE: &str = "\
synesthesia — sound from one point in a large parameter space

usage:
  synesthesia play [options]        play a point
  synesthesia render [options]      render a point offline

options:
  --preset N        built-in preset, 0..11 (default 0)
  --point FILE      a point as JSON (overrides --preset)
  --secs S          seconds to render (default 8)
  --sr RATE         sample rate (default 48000)
  --seed N          seed for the noise generators (default 1)
  --out FILE        write a 16-bit WAV here (render only)
  --raw FILE        write raw little-endian f32 samples here (render only)
  --no-sound        play into a null device (for measuring)
  --list            list the built-in presets

`play` runs until Ctrl-C, or for --secs seconds if that is given.
";

struct Args {
    preset: usize,
    point: Option<String>,
    secs: f64,
    sr: f64,
    seed: u32,
    out: Option<String>,
    raw: Option<String>,
    no_sound: bool,
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match run(&argv) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("synesthesia: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(argv: &[String]) -> Result<(), String> {
    let command = argv.first().map(String::as_str).unwrap_or("");
    if command.is_empty() || command == "--help" || command == "-h" || command == "help" {
        print!("{USAGE}");
        return Ok(());
    }
    if command != "render" && command != "play" {
        return Err(format!("unknown command `{command}`\n\n{USAGE}"));
    }

    let mut a = Args {
        preset: 0,
        point: None,
        secs: 8.0,
        sr: 48000.0,
        seed: 1,
        out: None,
        raw: None,
        no_sound: false,
    };
    let mut it = argv[1..].iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--list" => {
                for (i, p) in presets().iter().enumerate() {
                    println!("{i:2}  {}", p.name);
                }
                return Ok(());
            }
            "--preset" => a.preset = value()?.parse().map_err(|e| format!("--preset: {e}"))?,
            "--point" => a.point = Some(value()?),
            "--secs" => a.secs = value()?.parse().map_err(|e| format!("--secs: {e}"))?,
            "--sr" => a.sr = value()?.parse().map_err(|e| format!("--sr: {e}"))?,
            "--seed" => a.seed = value()?.parse().map_err(|e| format!("--seed: {e}"))?,
            "--no-sound" => a.no_sound = true,
            "--out" => a.out = Some(value()?),
            "--raw" => a.raw = Some(value()?),
            other => return Err(format!("unknown option `{other}`\n\n{USAGE}")),
        }
    }

    let (name, state) = match &a.point {
        Some(path) => {
            let raw = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            let state: AppState = serde_json::from_str(&raw).map_err(|e| format!("{path}: {e}"))?;
            (path.clone(), state)
        }
        None => {
            let p = presets().get(a.preset).ok_or(format!("no preset {}", a.preset))?;
            (p.name.clone(), p.state.clone())
        }
    };

    if command == "play" {
        return play(&name, &state, &a);
    }

    let started = std::time::Instant::now();
    let samples = render_offline(&state, a.secs, a.sr, a.seed);
    let wall = started.elapsed().as_secs_f64();

    let peak = samples.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let rms =
        (samples.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / samples.len() as f64).sqrt();
    println!(
        "{name}: {:.1} s at {:.0} Hz in {wall:.2} s ({:.1}x realtime), peak {peak:.3}, rms {rms:.4}",
        a.secs,
        a.sr,
        a.secs / wall
    );

    if let Some(path) = &a.out {
        std::fs::write(path, wav::encode_mono(&samples, a.sr as u32)).map_err(|e| format!("{path}: {e}"))?;
    }
    if let Some(path) = &a.raw {
        let mut bytes = Vec::with_capacity(samples.len() * 4);
        for s in &samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(path, bytes).map_err(|e| format!("{path}: {e}"))?;
    }
    Ok(())
}

/// Plays a point until Ctrl-C (or for `--secs`), printing a level line every
/// second. The TUI of phase 4 replaces this readout; the sound path is the one
/// it will use.
fn play(name: &str, state: &AppState, a: &Args) -> Result<(), String> {
    let sink: Box<dyn Sink> = if a.no_sound {
        Box::<NullSink>::default()
    } else {
        Box::new(PipeSink::open(a.sr as u32, 1, 1024).map_err(|e| format!("no audio output: {e}"))?)
    };
    eprintln!("{name} → {} at {:.0} Hz", sink.name(), a.sr);

    let limit = if a.secs > 0.0 && std::env::args().any(|x| x == "--secs") { Some(a.secs) } else { None };
    let player = Player::start(a.sr, state, sink);
    let mut next_line = 0.0;
    while let Some(f) = player.wait_frame() {
        if f.time >= next_line {
            next_line = f.time + 1.0;
            let bars = (f.rms * 120.0).min(40.0) as usize;
            eprintln!(
                "{:6.1}s  {:<40}  rms {:.4}  peak {:.3}{}",
                f.time,
                "#".repeat(bars),
                f.rms,
                f.peak,
                if f.limiter_db < -0.1 { format!("  limiter {:.1} dB", f.limiter_db) } else { String::new() },
            );
        }
        if limit.is_some_and(|l| f.time >= l) {
            break;
        }
    }
    player.stop()
}
