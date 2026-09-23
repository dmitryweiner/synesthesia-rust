//! `synesthesia` — the console application.
//!
//! Phase 2 of PLAN.md: only the offline renderer is wired up, so the sound can
//! be measured before there is anything to listen to. Playback and the TUI
//! follow in phases 3 and 4.

use std::process::ExitCode;

mod store;

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use syn_audio::{Frame, NullSink, PipeSink, Player, Sink};
use syn_core::dsp::rng::{Mulberry32, Rng};
use syn_core::genome::evolve::lerp_genome;
use syn_core::genome::explorer::ExplorerOptions;
use syn_core::genome::scout::{self, ScoutKind, ScoutResult, ScoutSettings};
use syn_core::genome::{decode_genome, encode_genome, Explorer, Genome};
use syn_core::share::encode_token;
use syn_tui::{draw, poll_action, Action, Screen, View};

use store::NamedPoint;
use syn_core::analysis::fractal::analyze_sound;
use syn_core::state::{presets, AppState};
use syn_core::{render_offline, wav};

const USAGE: &str = "\
synesthesia — sound from one point in a large parameter space

usage:
  synesthesia [options]             open the console interface
  synesthesia play [options]        play a point without the interface
  synesthesia render [options]      render a point offline
  synesthesia bench [options]       render every preset and report

options:
  --preset N        built-in preset, 0..11 (default 0)
  --point FILE      a point as JSON (overrides --preset)
  --secs S          seconds to render (default 8)
  --sr RATE         sample rate (default 48000)
  --seed N          seed for the noise generators (default 1)
  --out FILE        write a 16-bit WAV here (render only)
  --raw FILE        write raw little-endian f32 samples here (render only)
  --no-sound        play into a null device (for measuring)
  --no-scout        do not score candidates in the background
  --score           print the fractality metrics of the render, as JSON
  --sim             bench the picture's simulation instead of the sound
  --picture FILE    render: also draw the picture at the end, as a PPM
  --size WxH        the picture's size in pixels (default 120x60)
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
    no_scout: bool,
    score: bool,
    sim: bool,
    picture: Option<String>,
    size: (usize, usize),
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
        "render" | "play" | "tui" | "bench" => (first, &argv[1..]),
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
        no_scout: false,
        score: false,
        sim: false,
        picture: None,
        size: (120, 60),
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
            "--no-scout" => a.no_scout = true,
            "--score" => a.score = true,
            "--sim" => a.sim = true,
            "--picture" => a.picture = Some(value()?),
            "--size" => {
                let v = value()?;
                let parsed = v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
                a.size =
                    parsed.filter(|&(w, h)| w > 0 && h > 0).ok_or(format!("--size: `{v}` is not WxH"))?;
            }
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

    if command == "bench" {
        return if a.sim { bench_sim(&a) } else { bench(&a) };
    }
    match command {
        "tui" => return tui(&name, state, &a),
        "play" => return play(&name, &state, &a),
        _ => {}
    }

    if let Some(path) = &a.picture {
        return render_picture(&state, &a, path);
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

    if a.score {
        let m = analyze_sound(&samples, a.sr);
        println!(
            "{{\"score\":{:.6},\"loudness\":{:.3},\"envBeta\":{},\"centroidBeta\":{},\"envHiguchi\":{},\"boxDim\":{:.4},\"silent\":{}}}",
            m.score,
            m.loudness,
            json_number(m.env_beta),
            json_number(m.centroid_beta),
            json_number(m.env_higuchi),
            m.box_dim,
            m.silent
        );
    }

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

/// The point's sound, rendered offline, drives the picture the way it will
/// live: its features and onset hits, stepped at 30 Hz; the last frame is
/// written as a binary PPM. For looking at a point without a terminal.
fn render_picture(state: &AppState, a: &Args, path: &str) -> Result<(), String> {
    use syn_core::sim::Picture;
    use syn_core::{Engine, BLOCK};
    const SIM_HZ: f64 = 30.0;
    let (w, h) = a.size;
    let mut engine = Engine::new(a.sr, state, a.seed);
    let mut picture = Picture::new(w, h, a.seed);
    let mut block = [0.0f32; BLOCK];
    let steps = (a.secs * SIM_HZ).round() as u64;
    let started = Instant::now();
    for i in 1..=steps {
        while engine.time() < i as f64 / SIM_HZ {
            engine.render(&mut block);
        }
        picture.step(state, &engine.features(), engine.hits(), engine.time());
    }
    let image = picture.draw(engine.time());
    let mut ppm = format!("P6\n{} {}\n255\n", image.w, image.h).into_bytes();
    ppm.extend(image.rgb.iter().flatten());
    std::fs::write(path, ppm).map_err(|e| format!("{path}: {e}"))?;
    println!(
        "{path}: {w}x{h} after {:.1} s, {} onset hits, in {:.2} s",
        a.secs,
        engine.hits(),
        started.elapsed().as_secs_f64()
    );
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
/// The background search: candidates are rendered and scored while the user
/// listens, and a press takes the best one prepared for that direction.
struct ScoutState {
    settings: ScoutSettings,
    candidates: usize,
    enabled: bool,
    start_at: Option<Instant>,
    rx: Option<Receiver<ScoutResult>>,
    result: Option<ScoutResult>,
}

impl ScoutState {
    /// Drops what was prepared: the point moved, so it is stale.
    fn invalidate(&mut self) {
        self.result = None;
        self.rx = None;
        self.start_at = None;
    }

    fn ready(&self, version: u64) -> Option<&ScoutResult> {
        self.result.as_ref().filter(|r| r.version == version)
    }
}

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
    scout: ScoutState,
}

struct Morph {
    from: Genome,
    to: Genome,
    started: Instant,
}

/// How long a change takes to arrive, in seconds — the web app's feel.
const MORPH_SECS: f64 = 2.0;

impl Session {
    /// Says something to the status line, and to `$SYN_LOG` when it is set —
    /// the only way to see what a full-screen interface did, from a script.
    fn say(&mut self, message: String) {
        if let Ok(path) = std::env::var("SYN_LOG") {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                use std::io::Write;
                let _ = writeln!(f, "{message}");
            }
        }
        self.status = message;
    }

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
        self.scout.invalidate();
        let _ = store::save_last_point(&decode_genome(&self.explorer.current));
    }

    /// Starts scouting once the sound has settled, and picks up a finished job.
    fn tick_scout(&mut self) {
        if !self.scout.enabled {
            return;
        }
        if let Some(rx) = &self.scout.rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.scout.rx = None;
                    if result.version == self.explorer.version {
                        let msg = format!(
                            "scouted {} + {} candidates in {:.1} s",
                            result.ready(ScoutKind::Like),
                            result.ready(ScoutKind::Dislike),
                            result.seconds
                        );
                        self.say(msg);
                        self.scout.result = Some(result);
                    }
                }
                Err(TryRecvError::Disconnected) => self.scout.rx = None,
                Err(TryRecvError::Empty) => {}
            }
            return;
        }
        if self.morph.is_some() || self.scout.result.is_some() {
            return;
        }
        let Some(at) = self.scout.start_at else {
            // Settle first: the user may press again straight away.
            self.scout.start_at = Some(Instant::now() + Duration::from_millis(800));
            return;
        };
        if Instant::now() < at {
            return;
        }

        let version = self.explorer.version;
        let parent = self.explorer.current.clone();
        let likes: Vec<Genome> =
            (0..self.scout.candidates).map(|_| self.explorer.propose_like(&mut self.rng)).collect();
        let dislikes: Vec<Genome> =
            (0..self.scout.candidates).map(|_| self.explorer.propose_dislike(&mut self.rng)).collect();
        let mut settings = self.scout.settings;
        settings.master_gain = self.master;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("syn-scout".into())
            .spawn(move || {
                let _ = tx.send(scout::run(version, &parent, &likes, &dislikes, settings));
            })
            .ok();
        self.scout.rx = Some(rx);
        self.scout.start_at = None;
    }

    /// The candidate the scout prepared for this direction, if it is still
    /// about the point the user is listening to.
    fn scouted(&self, kind: ScoutKind) -> Option<(Genome, f64, usize)> {
        let result = self.scout.ready(self.explorer.version)?;
        let best = result.best(kind)?;
        Some((best.genome.clone(), best.analysis.score, result.ready(kind)))
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
        self.scout.invalidate();
        self.base_name = name;
        self.steps = 0;
        self.master = state.audio.master_gain;
        self.explorer.load(encode_genome(state));
        self.push_state(state.clone(), true);
        let _ = store::save_last_point(state);
    }
}

fn tui(name: &str, state: AppState, a: &Args) -> Result<(), String> {
    let config = store::load_config();
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
        scout: ScoutState {
            settings: ScoutSettings {
                seconds: config.scout_seconds,
                sample_rate: config.scout_sample_rate,
                master_gain: state.audio.master_gain,
                seed: seed ^ 0x5f36_1a2b,
            },
            candidates: config.scout_candidates,
            enabled: config.scout && !a.no_scout,
            start_at: None,
            rx: None,
            result: None,
        },
    };

    let mut screen = Screen::open().map_err(|e| format!("terminal: {e}"))?;
    let mut frame = Frame::default();
    // Drawing is what the interface actually costs: the terminal emulator
    // repaints the whole window for every redraw, and on a machine with no GPU
    // driver that repaint is software. So redraws are rate-limited, while keys
    // are polled far more often than that and always redraw at once.
    let redraw_every = Duration::from_secs_f64(1.0 / config.ui_fps.clamp(1.0, 60.0));
    let mut last_draw = Instant::now() - redraw_every;
    let mut dirty = true;
    // A key press redraws at once: the rate limit is there to slow the
    // animation down, not to make typing feel sticky.
    let mut pressed = false;
    loop {
        if let Some(f) = s.player.latest_frame() {
            frame = f;
            dirty = true;
        }
        if s.tick_morph() {
            dirty = true;
        }
        s.tick_scout();

        let now = Instant::now();
        let since_draw = now.duration_since(last_draw);
        if dirty && (since_draw >= redraw_every || pressed) {
            let title = s.name();
            // The names are only read when the panel is open; cloning them
            // every redraw would allocate for nothing.
            let names: Vec<String> =
                if s.points_open { s.points.iter().map(|p| p.name.clone()).collect() } else { Vec::new() };
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
            last_draw = now;
            dirty = false;
            pressed = false;
        }

        // Wait for a key until the next redraw is due (never longer than a
        // moment, so Ctrl-C and q feel immediate).
        let wait = redraw_every
            .saturating_sub(since_draw)
            .clamp(Duration::from_millis(5), Duration::from_millis(40));
        let Some(action) = poll_action(wait).map_err(|e| format!("input: {e}"))? else {
            continue;
        };
        dirty = true;
        pressed = true;
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
                        s.say(format!("loaded “{}”", p.name));
                    }
                    s.points_open = false;
                }
                Action::Delete => {
                    if s.selected < s.points.len() {
                        let gone = s.points.remove(s.selected);
                        s.selected = s.selected.min(s.points.len().saturating_sub(1));
                        let msg = match store::save_points(&s.points) {
                            Ok(()) => format!("forgot “{}”", gone.name),
                            Err(e) => format!("could not write the points file: {e}"),
                        };
                        s.say(msg);
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
                s.say(if s.muted { "silent".into() } else { "playing".into() });
            }
            Action::Like => {
                let picked = s.scouted(ScoutKind::Like);
                let from = s.explorer.current.clone();
                let to = s.explorer.like(picked.as_ref().map(|p| p.0.clone()), &mut s.rng).clone();
                s.start_morph(from, to);
                let msg = match picked {
                    Some((_, score, of)) => {
                        format!("more of this · best of {of} scouted (fractality {score:.2})")
                    }
                    None => format!("more of this · spread {:.2}", s.explorer.sigma),
                };
                s.say(msg);
            }
            Action::Dislike => {
                let picked = s.scouted(ScoutKind::Dislike);
                let from = s.explorer.current.clone();
                let to = s.explorer.dislike(picked.as_ref().map(|p| p.0.clone()), &mut s.rng).clone();
                s.start_morph(from, to);
                let msg = match picked {
                    Some((_, score, of)) => {
                        format!("not this · best of {of} scouted (fractality {score:.2})")
                    }
                    None => format!("not this · spread {:.2}", s.explorer.sigma),
                };
                s.say(msg);
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
                let msg = format!("surprise: {}", preset.name);
                s.say(msg);
            }
            Action::Undo => {
                let from = s.explorer.current.clone();
                match s.explorer.undo() {
                    Some(to) => {
                        let to = to.clone();
                        s.start_morph(from, to);
                        s.steps = s.steps.saturating_sub(2);
                        let msg = format!("undo · {} left", s.explorer.undo_depth());
                        s.say(msg);
                    }
                    None => s.say("nothing to undo".into()),
                }
            }
            Action::Save => {
                let state = decode_genome(&s.explorer.current);
                let name = format!("{} #{}", s.base_name, s.points.len() + 1);
                let msg = match store::keep_point(&name, &state) {
                    Ok(n) => {
                        s.points = store::load_points();
                        format!("kept as “{name}” ({n} points)")
                    }
                    Err(e) => format!("could not keep the point: {e}"),
                };
                s.say(msg);
            }
            Action::Points => {
                s.points = store::load_points();
                s.selected = s.selected.min(s.points.len().saturating_sub(1));
                s.points_open = true;
            }
            Action::Details => {
                let g = &s.explorer.current;
                let msg = format!(
                    "{} formulas · {} routes · spread {:.2} · {} undo steps · genome {}",
                    decode_genome(g).enabled_formulas().len(),
                    s.playing.modulation.routes.len(),
                    s.explorer.sigma,
                    s.explorer.undo_depth(),
                    g.len()
                );
                s.say(msg);
            }
            Action::Export => {
                let token = encode_token(&decode_genome(&s.explorer.current));
                let path = store::data_dir().join("token.txt");
                let msg = match std::fs::create_dir_all(store::data_dir())
                    .and_then(|()| std::fs::write(&path, format!("#s={token}\n")))
                {
                    Ok(()) => format!("token written to {}", path.display()),
                    Err(e) => format!("could not write the token: {e}"),
                };
                s.say(msg);
            }
            Action::Up | Action::Down | Action::Enter | Action::Delete => {}
        }
    }

    let _ = store::save_last_point(&decode_genome(&s.explorer.current));
    drop(screen);
    s.player.stop()
}

/// Renders every preset and reports what it cost and what it scored — the
/// bench this project judges changes by (PLAN.md decision 10).
fn bench(a: &Args) -> Result<(), String> {
    println!(
        "{:<20} {:>10} {:>9} {:>8} {:>7} {:>7}",
        "preset", "x realtime", "rms", "peak", "score", "boxDim"
    );
    let (mut slowest, mut slowest_name) = (f64::INFINITY, String::new());
    for p in presets() {
        let started = std::time::Instant::now();
        let x = render_offline(&p.state, a.secs, a.sr, a.seed);
        let speed = a.secs / started.elapsed().as_secs_f64();
        let m = analyze_sound(&x, a.sr);
        let peak = x.iter().fold(0.0f32, |acc, v| acc.max(v.abs()));
        let rms = (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len() as f64).sqrt();
        println!("{:<20} {speed:>10.1} {rms:>9.4} {peak:>8.3} {:>7.2} {:>7.2}", p.name, m.score, m.box_dim);
        if speed < slowest {
            slowest = speed;
            slowest_name = p.name.clone();
        }
    }
    println!(
        "\nslowest: {slowest_name} at {slowest:.1}x realtime — {:.1}% of one core to play live",
        100.0 / slowest
    );
    Ok(())
}

/// The picture on the grid a 120x40 terminal gets (GRAPHICS.md): `--secs`
/// seconds of steps at the 30 Hz the app steps at, per preset, with the
/// point's LFOs and a steady mid-brightness sound; then the cost of drawing
/// one frame. Run it under `taskset -c 6` to read the numbers as one A76 core.
fn bench_sim(a: &Args) -> Result<(), String> {
    use syn_core::sim::{grid_for_pixels, Picture, SimParams};
    use syn_core::AudioFeatures;
    const SIM_HZ: f64 = 30.0;
    const DRAW_FPS: f64 = 8.0;
    let (pw, ph) = (120, 60);
    let (w, h) = grid_for_pixels(pw, ph);
    let steps = (a.secs * SIM_HZ).round().max(1.0) as u64;
    let sound = AudioFeatures { loudness: 0.5, brightness: 0.5, ..Default::default() };
    println!("grid {w}x{h}, picture {pw}x{ph}, {steps} steps at {SIM_HZ} Hz, drawn at {DRAW_FPS} fps\n");
    println!(
        "{:<20} {:>6} {:>9} {:>12} {:>7} {:>8} {:>7}",
        "preset", "speed", "ms/step", "ns/cell/sub", "fields", "ms/draw", "core %"
    );
    let mut worst = (0.0f64, String::new());
    for p in presets() {
        let substeps = SimParams::from_cards(&p.state.visual.cards).reaction.substeps();
        let mut pic = Picture::new(pw, ph, a.seed);
        let mut t = 0.0;
        for _ in 0..30 {
            t += 1.0 / SIM_HZ;
            pic.step(&p.state, &sound, 0, t);
        }
        let before = pic.sim().stats();
        let started = Instant::now();
        for _ in 0..steps {
            t += 1.0 / SIM_HZ;
            pic.step(&p.state, &sound, 0, t);
        }
        let per_step = started.elapsed().as_secs_f64() / steps as f64;
        let after = pic.sim().stats();
        let started = Instant::now();
        for _ in 0..20 {
            pic.draw(t);
        }
        let per_draw = started.elapsed().as_secs_f64() / 20.0;
        let ns_cell = per_step * 1e9 / (w * h * substeps) as f64;
        let draws = (after.param_field_draws - before.param_field_draws)
            + (after.velocity_draws - before.velocity_draws);
        let core = (per_step * SIM_HZ + per_draw * DRAW_FPS) * 100.0;
        println!(
            "{:<20} {substeps:>6} {:>9.2} {ns_cell:>12.2} {:>7.1} {:>8.2} {core:>7.1}",
            p.name,
            per_step * 1e3,
            draws as f64 / steps as f64,
            per_draw * 1e3
        );
        if core > worst.0 {
            worst = (core, p.name.clone());
        }
    }
    println!("\nheaviest: {} at {:.1}% of one core", worst.1, worst.0);
    Ok(())
}

/// JSON has no NaN; the web app's metrics use it for "nothing to fit here".
fn json_number(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.4}")
    } else {
        "null".to_string()
    }
}
