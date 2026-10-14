//! `synesthesia` — the console application.
//!
//! A shell around synesthesia-core (PLAN.md decisions 11 and 12): the model is
//! `syn-core`, the sound `syn-player`, what a press does `syn-session`. This
//! crate is the console's part — the command line, the files, the terminal
//! loop, and carrying out the session's effects.

use std::process::ExitCode;

mod picture;
mod store;

use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use syn_audio::{Frame, NullSink, Output, PipeSink, Player, Sink};
use syn_core::analysis::fractal::analyze_sound;
use syn_core::genome::scout::ScoutResult;
use syn_core::share::encode_token;
use syn_core::sim::Image;
use syn_core::state::{presets, AppState};
use syn_core::{render_offline, wav};
use syn_session::points::Points;
use syn_session::{Effect, ScoutConfig, ScoutPool, ScoutRequest, Session, SessionOptions};
use syn_tui::{draw, poll_action, Action, ColorMode, Screen, View, VizMode};

const USAGE: &str = "\
synesthesia — sound and image from one point in a large parameter space

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

    // `restored`: the point was left mid-search when the app last closed.
    let (name, state, restored) = match &a.point {
        Some(path) => {
            let raw = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            let state: AppState = serde_json::from_str(&raw).map_err(|e| format!("{path}: {e}"))?;
            (path.clone(), state, false)
        }
        // Without an explicit point, the interface picks up where it left off,
        // under the name of where that point came from.
        None if !preset_given && command == "tui" => match store::load_last_point() {
            Some(state) => {
                let name = state.preset_name.clone().unwrap_or_else(|| "where you left off".to_string());
                (name, state, true)
            }
            None => {
                let p = &presets()[0];
                (p.name.clone(), p.state.clone(), false)
            }
        },
        None => {
            let p = presets().get(a.preset).ok_or(format!("no preset {}", a.preset))?;
            (p.name.clone(), p.state.clone(), false)
        }
    };

    if command == "bench" {
        return if a.sim { bench_sim(&a) } else { bench(&a) };
    }
    match command {
        "tui" => return tui(&name, state, restored, &a),
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
    use syn_core::visualizer::{Visualizer, VizInput};
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
        picture.step(&VizInput {
            state,
            features: engine.features(),
            hits: engine.hits(),
            time: engine.time(),
        });
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

/// Starts the sound of `state` on `sink`, faded in.
fn start_output(sr: f64, state: &AppState, sink: Box<dyn Sink>) -> Output {
    let player = Arc::new(Player::new(sr, state));
    player.fade_in();
    Output::start(player, sink)
}

/// Plays a point until Ctrl-C (or for `--secs`), printing a level line every
/// second — the sound path of the interface, without the interface.
fn play(name: &str, state: &AppState, a: &Args) -> Result<(), String> {
    let sink: Box<dyn Sink> = if a.no_sound {
        Box::new(NullSink::new(a.sr))
    } else {
        Box::new(PipeSink::open(a.sr as u32, 1, 1024).map_err(|e| format!("no audio output: {e}"))?)
    };
    eprintln!("{name} → {} at {:.0} Hz", sink.name(), a.sr);

    let limit = if a.secs > 0.0 && std::env::args().any(|x| x == "--secs") { Some(a.secs) } else { None };
    let output = start_output(a.sr, state, sink);
    let mut next_line = 0.0;
    while output.is_running() {
        std::thread::sleep(Duration::from_millis(20));
        let Some(f) = output.player().latest_frame() else { continue };
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
    output.stop()
}

/// The console around synesthesia-core's `Session`: the session decides what
/// a press does to the sound (the search, the morph, the scout's timing, the
/// status line); this carries out its effects — on the player, the picture,
/// the files and the scout's threads — and keeps what is only the screen's:
/// the points list, the help, the mute.
struct App {
    session: Session,
    /// The session's clock: monotonic seconds since the interface opened.
    clock: Instant,
    output: Output,
    picture: picture::PictureThread,
    /// What the player was last told to play — what the screen and the
    /// picture show.
    audible: AppState,
    muted: bool,
    points: Points,
    points_open: bool,
    selected: usize,
    show_help: bool,
    status: String,
    /// The scout's own pool, smaller than the machine, so the sound always
    /// has a core to run on.
    scout_pool: Arc<ScoutPool>,
    scout_tx: Sender<ScoutResult>,
    scout_rx: Receiver<ScoutResult>,
}

impl App {
    fn now(&self) -> f64 {
        self.clock.elapsed().as_secs_f64()
    }

    /// Says something to the status line, and to `$SYN_LOG` when it is set —
    /// the only way to see what a full-screen interface did, from a script.
    fn say(&mut self, message: String) {
        if let Ok(path) = std::env::var("SYN_LOG") {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                use std::io::Write;
                let _ = writeln!(f, "{message}");
            }
        }
        // The status line is one line; the session's are "what\nwhat changed".
        self.status = message.replace('\n', " — ");
    }

    /// Does what the session asked for.
    fn apply(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::SetPoint(p) => {
                    self.output.player().set_state((*p).clone());
                    self.show(*p);
                }
                Effect::SwitchTo(p) => {
                    self.output.player().switch_to((*p).clone());
                    self.show(*p);
                }
                Effect::Reseed => self.picture.reseed(),
                Effect::SaveLastPoint(p) => {
                    let _ = store::save_last_point(&p);
                }
                Effect::StartScout(req) => self.scout(*req),
                Effect::Status(message) => self.say(message),
            }
        }
    }

    fn show(&mut self, point: AppState) {
        self.picture.set_state(&point);
        self.audible = point;
    }

    /// Seconds of rendering: on a thread of its own that waits on the pool,
    /// never on the one that draws and reads keys.
    fn scout(&self, req: ScoutRequest) {
        let (pool, tx) = (Arc::clone(&self.scout_pool), self.scout_tx.clone());
        let job = move || {
            let _ = tx.send(pool.run(&req));
        };
        let _ = std::thread::Builder::new().name("syn-scout".into()).spawn(job);
    }

    /// Moves the session on; true when something changed on screen.
    fn tick(&mut self) -> bool {
        let mut changed = false;
        while let Ok(result) = self.scout_rx.try_recv() {
            let fx = self.session.scout_finished(result);
            changed |= !fx.is_empty();
            self.apply(fx);
        }
        if self.session.wants_tick() {
            let now = self.now();
            let fx = self.session.tick(now);
            changed |= !fx.is_empty();
            self.apply(fx);
        }
        changed
    }

    /// Mute is the session's ▶/⏸: the sound fades out and the scout stops,
    /// since nobody is listening; back on, it starts where the search is.
    fn toggle_sound(&mut self) {
        let now = self.now();
        self.muted = !self.muted;
        if self.muted {
            self.output.player().fade_out();
        } else {
            let live = self.session.live_point();
            self.output.player().set_state(live.clone());
            self.show(live);
            self.output.player().fade_in();
        }
        let fx = self.session.set_playing(now, !self.muted);
        self.apply(fx);
        self.say(if self.muted { "silent".into() } else { "playing".into() });
    }

    /// 💾: under the name the core offers — the same one for a point of your
    /// own, a fresh "Point N" for anything else.
    fn keep(&mut self) {
        let point = self.session.point();
        let kept = store::load_points().and_then(|points| {
            let name = points.suggest_name(point.preset_name.as_deref());
            store::keep_point(&name, &point).map(|points| (name, points))
        });
        match kept {
            Ok((name, points)) => {
                self.points = points;
                let fx = self.session.kept_as(&name);
                self.apply(fx);
            }
            Err(e) => self.say(format!("could not keep the point: {e}")),
        }
    }

    fn open_points(&mut self) {
        match store::load_points() {
            Ok(points) => self.points = points,
            Err(e) => {
                self.points = Points::new();
                self.say(e);
            }
        }
        self.selected = self.selected.min(self.points.len().saturating_sub(1));
        self.points_open = true;
    }

    fn forget(&mut self) {
        let Some(name) = self.points.get(self.selected).map(|p| p.name.clone()) else { return };
        let msg = match store::forget_point(&name) {
            Ok(points) => {
                self.points = points;
                format!("forgot “{name}”")
            }
            Err(e) => format!("could not write the points file: {e}"),
        };
        self.selected = self.selected.min(self.points.len().saturating_sub(1));
        self.say(msg);
    }

    fn details(&mut self) {
        let point = self.session.point();
        let v = self.session.view();
        let measured = match self.session.scout_parent() {
            Some(m) => format!(" · fractality {:.2}", m.score),
            None => String::new(),
        };
        let msg = format!(
            "{} formulas · {} routes · spread {:.2} · {} undo steps{measured}",
            point.enabled_formulas().len(),
            point.modulation.routes.len(),
            v.sigma,
            v.undo_depth,
        );
        self.say(msg);
    }

    fn export(&mut self) {
        let token = encode_token(&self.session.point());
        let path = store::data_dir().join("token.txt");
        let msg = match std::fs::create_dir_all(store::data_dir())
            .and_then(|()| std::fs::write(&path, format!("#s={token}\n")))
        {
            Ok(()) => format!("token written to {}", path.display()),
            Err(e) => format!("could not write the token: {e}"),
        };
        self.say(msg);
    }
}

/// The console interface: the session, the points and the sound in one loop.
/// Everything the screen shows comes from `syn-tui`; everything a press
/// changes goes through the session.
fn tui(name: &str, state: AppState, restored: bool, a: &Args) -> Result<(), String> {
    let config = store::load_config();
    let sink = open_sink(a)?;
    let sink_name = sink.name().to_string();
    let seed =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos());
    let opts = SessionOptions {
        scout: ScoutConfig {
            enabled: config.scout && !a.no_scout,
            candidates: config.scout_candidates,
            seconds: config.scout_seconds,
            sample_rate: config.scout_sample_rate,
            ..ScoutConfig::default()
        },
        seed,
        ..SessionOptions::default()
    };
    let session =
        if restored { Session::restored(name, &state, opts) } else { Session::new(name, &state, opts) };
    let audible = session.live_point();
    let output = start_output(a.sr, &audible, sink);
    let picture = picture::PictureThread::start(
        &audible,
        picture::Settings { sim_hz: config.sim_hz, draw_fps: config.viz_fps, seed },
    );
    let (scout_tx, scout_rx) = std::sync::mpsc::channel();
    let points = store::load_points();
    let mut s = App {
        session,
        clock: Instant::now(),
        output,
        picture,
        audible,
        muted: false,
        points: points.clone().unwrap_or_default(),
        points_open: false,
        selected: 0,
        show_help: true,
        status: String::new(),
        scout_pool: Arc::new(ScoutPool::new(config.scout_threads)),
        scout_tx,
        scout_rx,
    };
    s.say(format!("{sink_name} · {:.0} Hz", a.sr));
    if let Err(e) = points {
        s.say(e);
    }
    let now = s.now();
    let fx = s.session.set_playing(now, true);
    s.apply(fx);

    let mut viz = VizMode::parse(&config.viz);
    let color = ColorMode::pick(&config.viz_color, std::env::var("COLORTERM").ok().as_deref());
    s.picture.set_running(viz != VizMode::Spectrum);
    let mut image = Image::default();

    let mut screen = Screen::open().map_err(|e| format!("terminal: {e}"))?;
    let mut frame = Frame::default();
    // Drawing is what the interface actually costs: the terminal emulator
    // repaints the whole window for every redraw, and on a machine with no GPU
    // driver that repaint is software. So redraws are rate-limited, while keys
    // are polled far more often than that and always redraw at once.
    // The picture's frames come at `viz_fps`, and only while it is on.
    let redraw_every = Duration::from_secs_f64(1.0 / config.ui_fps.max(config.viz_fps).clamp(1.0, 60.0));
    let mut last_draw = Instant::now() - redraw_every;
    let mut dirty = true;
    // A key press redraws at once: the rate limit is there to slow the
    // animation down, not to make typing feel sticky.
    let mut pressed = false;
    loop {
        if !s.output.is_running() {
            break;
        }
        if let Some(f) = s.output.player().latest_frame().filter(|f| f.time != frame.time) {
            frame = f;
            s.picture.set_frame(f);
            dirty = true;
        }
        if s.tick() {
            dirty = true;
        }
        if s.picture.take_new(&mut image) {
            dirty = true;
        }

        let now = Instant::now();
        let since_draw = now.duration_since(last_draw);
        if dirty && (since_draw >= redraw_every || pressed) {
            let title = s.session.name();
            // The names are only read when the panel is open; cloning them
            // every redraw would allocate for nothing.
            let names: Vec<String> = if s.points_open {
                s.points.names().into_iter().map(str::to_string).collect()
            } else {
                Vec::new()
            };
            let view = View {
                name: &title,
                state: &s.audible,
                frame,
                muted: s.muted,
                status: &s.status,
                show_help: s.show_help,
                points: &names,
                points_open: s.points_open,
                selected: s.selected,
                picture: (image.w > 0).then_some(&image),
                viz,
                color,
            };
            let mut area = None;
            screen.terminal.draw(|f| area = draw(f, &view)).map_err(|e| format!("draw: {e}"))?;
            if let Some(r) = area {
                s.picture.set_size(usize::from(r.width), usize::from(r.height) * 2);
            }
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
        let now = s.now();
        if s.points_open {
            match action {
                Action::Escape | Action::Points => s.points_open = false,
                Action::Up => s.selected = s.selected.saturating_sub(1),
                Action::Down => s.selected = (s.selected + 1).min(s.points.len().saturating_sub(1)),
                Action::Enter => {
                    if let Some(p) = s.points.get(s.selected).cloned() {
                        let fx = s.session.load(now, &p.name, &p.state);
                        s.apply(fx);
                    }
                    s.points_open = false;
                }
                Action::Delete => s.forget(),
                Action::Quit => break,
                _ => {}
            }
            continue;
        }

        let fx = match action {
            Action::Quit => break,
            Action::Like => s.session.like(now),
            Action::Dislike => s.session.dislike(now),
            Action::Surprise => s.session.surprise(now),
            Action::Undo => s.session.undo(now),
            Action::Help => {
                s.show_help = !s.show_help;
                continue;
            }
            Action::ToggleSound => {
                s.toggle_sound();
                continue;
            }
            Action::Save => {
                s.keep();
                continue;
            }
            Action::Points => {
                s.open_points();
                continue;
            }
            Action::Details => {
                s.details();
                continue;
            }
            Action::Export => {
                s.export();
                continue;
            }
            Action::Picture => {
                viz = viz.next();
                s.picture.set_running(viz != VizMode::Spectrum);
                // Remembered for the next start; the rest of the file is kept.
                let mut c = store::load_config();
                c.viz = viz.as_str().to_string();
                let msg = match store::save_config(&c) {
                    Ok(()) => viz.label().to_string(),
                    Err(e) => format!("{} (not saved: {e})", viz.label()),
                };
                s.say(msg);
                continue;
            }
            Action::Escape | Action::Up | Action::Down | Action::Enter | Action::Delete => continue,
        };
        s.apply(fx);
    }

    let _ = store::save_last_point(&s.session.point());
    let App { output, picture, .. } = s;
    drop(picture);
    drop(screen);
    output.stop()
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
    use syn_core::visualizer::{Visualizer, VizInput};
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
            pic.step(&VizInput { state: &p.state, features: sound, hits: 0, time: t });
        }
        let before = pic.sim().stats();
        let started = Instant::now();
        for _ in 0..steps {
            t += 1.0 / SIM_HZ;
            pic.step(&VizInput { state: &p.state, features: sound, hits: 0, time: t });
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
