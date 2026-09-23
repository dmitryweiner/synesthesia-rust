//! `synesthesia` — the console application.
//!
//! Phase 2 of PLAN.md: only the offline renderer is wired up, so the sound can
//! be measured before there is anything to listen to. Playback and the TUI
//! follow in phases 3 and 4.

use std::process::ExitCode;

mod store;

use std::time::{Duration, Instant};

use syn_audio::{Frame, NullSink, PipeSink, Player, Sink};
use syn_core::dsp::rng::{Mulberry32, Rng};
use syn_core::genome::evolve::lerp_genome;
use syn_core::genome::explorer::ExplorerOptions;
use syn_core::genome::{decode_genome, encode_genome, Explorer, Genome};
use syn_core::share::encode_token;
use syn_tui::{draw, poll_action, Action, Screen, View};

use store::NamedPoint;
use syn_core::state::{presets, AppState};
use syn_core::{render_offline, wav};

const USAGE: &str = "\
synesthesia — sound from one point in a large parameter space

usage:
  synesthesia [options]             open the console interface
  synesthesia play [options]        play a point without the interface
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
    let first = argv.first().map(String::as_str).unwrap_or("");
    if first == "--help" || first == "-h" || first == "help" {
        print!("{USAGE}");
        return Ok(());
    }
    // No command means the interface; flags may follow either way.
    let (command, rest): (&str, &[String]) = match first {
        "render" | "play" | "tui" => (first, &argv[1..]),
        _ => ("tui", argv),
    };

    let config = store::load_config();
    if store::load_config_path_missing() {
        // Leave a file behind on the first run, so the settings are visible
        // and hand-editable rather than folklore.
        let _ = store::save_config(&config);
    }
    let mut preset_given = false;
    let mut a = Args {
        preset: 0,
        point: None,
        secs: 8.0,
        sr: config.sample_rate,
        seed: 1,
        out: None,
        raw: None,
        no_sound: false,
    };
    let mut it = rest.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--list" => {
                for (i, p) in presets().iter().enumerate() {
                    println!("{i:2}  {}", p.name);
                }
                return Ok(());
            }
            "--preset" => {
                a.preset = value()?.parse().map_err(|e| format!("--preset: {e}"))?;
                preset_given = true;
            }
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
        // Without an explicit point, the interface picks up where it left off.
        None if !preset_given && command == "tui" => match store::load_last_point() {
            Some(state) => ("where you left off".to_string(), state),
            None => {
                let p = &presets()[0];
                (p.name.clone(), p.state.clone())
            }
        },
        None => {
            let p = presets().get(a.preset).ok_or(format!("no preset {}", a.preset))?;
            (p.name.clone(), p.state.clone())
        }
    };

    match command {
        "tui" => return tui(&name, state, &a),
        "play" => return play(&name, &state, &a),
        _ => {}
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

fn open_sink(a: &Args) -> Result<Box<dyn Sink>, String> {
    if a.no_sound {
        return Ok(Box::new(NullSink::new(a.sr)));
    }
    let c = store::load_config();
    let sink = PipeSink::open_with(&c.audio_command, a.sr as u32, 1, c.latency_frames)
        .map_err(|e| format!("no audio output: {e}"))?;
    Ok(Box::new(sink))
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

/// The console interface: the search, the points and the sound in one loop.
/// Everything the screen shows comes from `syn-tui`; everything it changes
/// goes through the explorer and the player.
struct Session {
    explorer: Explorer,
    rng: Mulberry32,
    player: Player,
    base_name: String,
    steps: u32,
    master: f64,
    muted: bool,
    playing: AppState,
    morph: Option<Morph>,
    points: Vec<NamedPoint>,
    points_open: bool,
    selected: usize,
    show_help: bool,
    status: String,
}

struct Morph {
    from: Genome,
    to: Genome,
    started: Instant,
}

/// How long a change takes to arrive, in seconds — the web app's feel.
const MORPH_SECS: f64 = 2.0;

impl Session {
    fn name(&self) -> String {
        if self.steps == 0 {
            self.base_name.clone()
        } else {
            format!("{} · {} steps", self.base_name, self.steps)
        }
    }

    /// Sends the current point to the audio thread, keeping what is not a
    /// gene (the master gain) and the mute.
    fn push_state(&mut self, state: AppState, hard: bool) {
        let mut next = state;
        next.audio.master_gain = if self.muted { 0.0 } else { self.master };
        if hard {
            self.player.switch_to(next.clone());
        } else {
            self.player.set_state(next.clone());
        }
        self.playing = next;
    }

    fn start_morph(&mut self, from: Genome, to: Genome) {
        self.morph = Some(Morph { from, to, started: Instant::now() });
        self.steps += 1;
        let _ = store::save_last_point(&decode_genome(&self.explorer.current));
    }

    /// Advances a running morph; returns true while one is in flight.
    fn tick_morph(&mut self) -> bool {
        let Some(m) = &self.morph else { return false };
        let t = (m.started.elapsed().as_secs_f64() / MORPH_SECS).min(1.0);
        let g = lerp_genome(&m.from, &m.to, t);
        let state = decode_genome(&g);
        self.push_state(state, false);
        if t >= 1.0 {
            self.morph = None;
        }
        true
    }

    fn load_point(&mut self, name: String, state: &AppState) {
        self.base_name = name;
        self.steps = 0;
        self.master = state.audio.master_gain;
        self.explorer.load(encode_genome(state));
        self.push_state(state.clone(), true);
        let _ = store::save_last_point(state);
    }
}

fn tui(name: &str, state: AppState, a: &Args) -> Result<(), String> {
    let sink = open_sink(a)?;
    let sink_name = sink.name().to_string();
    let player = Player::start(a.sr, &state, sink);
    let seed =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos());

    let mut s = Session {
        explorer: Explorer::new(encode_genome(&state), ExplorerOptions::default()),
        rng: Mulberry32::new(seed),
        player,
        base_name: name.to_string(),
        steps: 0,
        master: state.audio.master_gain,
        muted: false,
        playing: state.clone(),
        morph: None,
        points: store::load_points(),
        points_open: false,
        selected: 0,
        show_help: true,
        status: format!("{sink_name} · {:.0} Hz", a.sr),
    };

    let mut screen = Screen::open().map_err(|e| format!("terminal: {e}"))?;
    let mut frame = Frame::default();
    loop {
        if let Some(f) = s.player.latest_frame() {
            frame = f;
        }
        s.tick_morph();

        let title = s.name();
        let names: Vec<String> = s.points.iter().map(|p| p.name.clone()).collect();
        let view = View {
            name: &title,
            state: &s.playing,
            frame,
            muted: s.muted,
            status: &s.status,
            show_help: s.show_help,
            points: &names,
            points_open: s.points_open,
            selected: s.selected,
        };
        screen.terminal.draw(|f| draw(f, &view)).map_err(|e| format!("draw: {e}"))?;

        let Some(action) = poll_action(Duration::from_millis(33)).map_err(|e| format!("input: {e}"))? else {
            continue;
        };
        if s.show_help && action != Action::Help {
            s.show_help = false;
        }
        if s.points_open {
            match action {
                Action::Escape | Action::Points => s.points_open = false,
                Action::Up => s.selected = s.selected.saturating_sub(1),
                Action::Down => s.selected = (s.selected + 1).min(s.points.len().saturating_sub(1)),
                Action::Enter => {
                    if let Some(p) = s.points.get(s.selected).cloned() {
                        s.load_point(p.name.clone(), &p.state);
                        s.status = format!("loaded “{}”", p.name);
                    }
                    s.points_open = false;
                }
                Action::Delete => {
                    if s.selected < s.points.len() {
                        let gone = s.points.remove(s.selected);
                        s.selected = s.selected.min(s.points.len().saturating_sub(1));
                        s.status = match store::save_points(&s.points) {
                            Ok(()) => format!("forgot “{}”", gone.name),
                            Err(e) => format!("could not write the points file: {e}"),
                        };
                    }
                }
                Action::Quit => break,
                _ => {}
            }
            continue;
        }

        match action {
            Action::Quit => break,
            Action::Escape => {}
            Action::Help => s.show_help = !s.show_help,
            Action::ToggleSound => {
                s.muted = !s.muted;
                let playing = s.playing.clone();
                s.push_state(playing, false);
                s.status = if s.muted { "silent".into() } else { "playing".into() };
            }
            Action::Like => {
                let from = s.explorer.current.clone();
                let to = s.explorer.like(None, &mut s.rng).clone();
                s.start_morph(from, to);
                s.status = format!("more of this · spread {:.2}", s.explorer.sigma);
            }
            Action::Dislike => {
                let from = s.explorer.current.clone();
                let to = s.explorer.dislike(None, &mut s.rng).clone();
                s.start_morph(from, to);
                s.status = format!("not this · spread {:.2}", s.explorer.sigma);
            }
            Action::Surprise => {
                let i = (s.rng.next() * presets().len() as f64) as usize;
                let preset = &presets()[i.min(presets().len() - 1)];
                let target = encode_genome(&preset.state);
                let from = s.explorer.current.clone();
                let to = s.explorer.surprise(&target, &mut s.rng).clone();
                s.base_name = format!("near {}", preset.name);
                s.steps = 0;
                s.master = preset.state.audio.master_gain;
                s.start_morph(from, to);
                s.status = format!("surprise: {}", preset.name);
            }
            Action::Undo => {
                let from = s.explorer.current.clone();
                match s.explorer.undo() {
                    Some(to) => {
                        let to = to.clone();
                        s.start_morph(from, to);
                        s.steps = s.steps.saturating_sub(2);
                        s.status = format!("undo · {} left", s.explorer.undo_depth());
                    }
                    None => s.status = "nothing to undo".into(),
                }
            }
            Action::Save => {
                let state = decode_genome(&s.explorer.current);
                let name = format!("{} #{}", s.base_name, s.points.len() + 1);
                s.status = match store::keep_point(&name, &state) {
                    Ok(n) => {
                        s.points = store::load_points();
                        format!("kept as “{name}” ({n} points)")
                    }
                    Err(e) => format!("could not keep the point: {e}"),
                };
            }
            Action::Points => {
                s.points = store::load_points();
                s.selected = s.selected.min(s.points.len().saturating_sub(1));
                s.points_open = true;
            }
            Action::Details => {
                let g = &s.explorer.current;
                s.status = format!(
                    "{} formulas · {} routes · spread {:.2} · {} undo steps · genome {}",
                    decode_genome(g).enabled_formulas().len(),
                    s.playing.modulation.routes.len(),
                    s.explorer.sigma,
                    s.explorer.undo_depth(),
                    g.len()
                );
            }
            Action::Export => {
                let token = encode_token(&decode_genome(&s.explorer.current));
                let path = store::data_dir().join("token.txt");
                s.status = match std::fs::create_dir_all(store::data_dir())
                    .and_then(|()| std::fs::write(&path, format!("#s={token}\n")))
                {
                    Ok(()) => format!("token written to {}", path.display()),
                    Err(e) => format!("could not write the token: {e}"),
                };
            }
            Action::Up | Action::Down | Action::Enter | Action::Delete => {}
        }
    }

    let _ = store::save_last_point(&decode_genome(&s.explorer.current));
    drop(screen);
    s.player.stop()
}
