//! Throwaway probe: what a colour field drawn in half-blocks costs the
//! terminal. Runs a real Gray-Scott field at 2x the cell grid, draws it through
//! ratatui (the same cell diff the app uses) and counts the bytes written.
//!
//!   field_probe --secs 20 --fps 8 --color true|256 --bits 8 --breathe 0|1 --out stats.json

use std::cell::Cell;
use std::f32::consts::TAU;
use std::io::{self, Write};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Color;
use ratatui::widgets::{Block, Borders, Paragraph, Widget};
use ratatui::Terminal;

struct Counting<W: Write> {
    inner: W,
    bytes: Rc<Cell<u64>>,
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(b)?;
        self.bytes.set(self.bytes.get() + n as u64);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

struct Sim {
    w: usize,
    h: usize,
    u: Vec<f32>,
    v: Vec<f32>,
    nu: Vec<f32>,
    nv: Vec<f32>,
}

impl Sim {
    fn new(w: usize, h: usize) -> Self {
        let mut s = Sim {
            w,
            h,
            u: vec![1.0; w * h],
            v: vec![0.0; w * h],
            nu: vec![0.0; w * h],
            nv: vec![0.0; w * h],
        };
        let mut seed = 12345u32;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32
        };
        for _ in 0..24 {
            let (cx, cy, r) = (rnd() * w as f32, rnd() * h as f32, 3.0 + rnd() * 3.0);
            for y in 0..h {
                for x in 0..w {
                    let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                    if d < r {
                        s.u[y * w + x] = 0.5;
                        s.v[y * w + x] = 0.5;
                    }
                }
            }
        }
        s
    }

    fn step(&mut self, feed: f32, kill: f32) {
        let (w, h) = (self.w, self.h);
        let (du, dv, dt) = (0.2097f32, 0.105f32, 1.0f32);
        for y in 0..h {
            let (ym, yp) = (y.saturating_sub(1), (y + 1).min(h - 1));
            for x in 0..w {
                let (xm, xp) = (x.saturating_sub(1), (x + 1).min(w - 1));
                let i = y * w + x;
                let lap = |f: &[f32]| {
                    -f[i]
                        + 0.05 * (f[ym * w + xm] + f[ym * w + xp] + f[yp * w + xm] + f[yp * w + xp])
                        + 0.2 * (f[ym * w + x] + f[y * w + xm] + f[y * w + xp] + f[yp * w + x])
                };
                let (u, v) = (self.u[i], self.v[i]);
                let r = u * v * v;
                self.nu[i] = (u + (du * lap(&self.u) - r + feed * (1.0 - u)) * dt).clamp(0.0, 1.0);
                self.nv[i] = (v + (dv * lap(&self.v) + r - (feed + kill) * v) * dt).clamp(0.0, 1.0);
            }
        }
        std::mem::swap(&mut self.u, &mut self.nu);
        std::mem::swap(&mut self.v, &mut self.nv);
    }

    /// v at pixel (px, py) of a grid half this size, box-averaged 2x2.
    fn sample(&self, px: usize, py: usize) -> f32 {
        let (x, y) = (px * 2, py * 2);
        let w = self.w;
        (self.v[y * w + x] + self.v[y * w + x + 1] + self.v[(y + 1) * w + x] + self.v[(y + 1) * w + x + 1])
            * 0.25
    }
}

struct Field<'a> {
    sim: &'a Sim,
    exposure: f32,
    color256: bool,
    bits: u32,
}

impl Field<'_> {
    fn color(&self, px: usize, py: usize) -> Color {
        let pw = self.sim.w / 2;
        let ph = self.sim.h / 2;
        let h = self.sim.sample(px, py);
        let raw = (h * 1.6).clamp(0.0, 1.0);
        let t = (raw * 1.5).fract();
        // relief: height gradient against a fixed light
        let hl = self.sim.sample(px.saturating_sub(1), py);
        let hr = self.sim.sample((px + 1).min(pw - 1), py);
        let hd = self.sim.sample(px, py.saturating_sub(1));
        let hu = self.sim.sample(px, (py + 1).min(ph - 1));
        let (nx, ny) = ((hl - hr) * 4.0, (hd - hu) * 4.0);
        let diffuse = ((0.5 * nx + 0.5 * ny + 0.7) / (nx * nx + ny * ny + 1.0).sqrt()).max(0.0);
        let shade = (0.55 + 0.55 * diffuse) * self.exposure;
        let ch = |d: f32| ((0.5 + 0.5 * (TAU * (t + d)).cos()) * shade).clamp(0.0, 1.0);
        let (r, g, b) = (ch(0.0), ch(0.33), ch(0.67));
        if self.color256 {
            let q = |c: f32| (c * 5.0).round() as u8;
            Color::Indexed(16 + 36 * q(r) + 6 * q(g) + q(b))
        } else {
            let drop = 8 - self.bits;
            let q = |c: f32| (((c * 255.0) as u8) >> drop) << drop;
            Color::Rgb(q(r), q(g), q(b))
        }
    }
}

impl Widget for Field<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for cy in 0..area.height {
            for cx in 0..area.width {
                let top = self.color(cx as usize, cy as usize * 2);
                let bot = self.color(cx as usize, cy as usize * 2 + 1);
                buf[(area.x + cx, area.y + cy)].set_char('▀').set_fg(top).set_bg(bot);
            }
        }
    }
}

fn main() -> io::Result<()> {
    let mut secs = 20.0;
    let mut fps = 8.0;
    let mut color256 = false;
    let mut bits = 8;
    let mut breathe = true;
    let mut out = String::from("probe.json");
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i + 1 < args.len() {
        let v = &args[i + 1];
        match args[i].as_str() {
            "--secs" => secs = v.parse().unwrap(),
            "--fps" => fps = v.parse().unwrap(),
            "--color" => color256 = v == "256",
            "--bits" => bits = v.parse().unwrap(),
            "--breathe" => breathe = v == "1",
            "--out" => out = v.clone(),
            _ => {}
        }
        i += 2;
    }

    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let counter = Rc::new(Cell::new(0u64));
    let mut term =
        Terminal::new(CrosstermBackend::new(Counting { inner: io::stdout(), bytes: counter.clone() }))?;
    let size = term.size()?;
    let field_rows = size.height.saturating_sub(10);
    let mut sim = Sim::new(size.width as usize * 2, field_rows as usize * 4);

    let start = Instant::now();
    let period = Duration::from_secs_f64(1.0 / fps);
    let (mut frames, mut sim_time, mut draw_time) = (0u64, Duration::ZERO, Duration::ZERO);
    let bytes_at_start = counter.get();
    while start.elapsed().as_secs_f64() < secs {
        let t0 = Instant::now();
        for _ in 0..10 {
            sim.step(0.037, 0.06);
        }
        let t1 = Instant::now();
        let el = start.elapsed().as_secs_f32();
        let exposure = if breathe { 1.0 + 0.12 * (TAU * 0.4 * el).sin() } else { 1.0 };
        term.draw(|f| {
            let [top, rest] =
                Layout::vertical([Constraint::Length(field_rows), Constraint::Min(0)]).areas(f.area());
            f.render_widget(Field { sim: &sim, exposure, color256, bits }, top);
            let text = format!("probe  frame {frames}  t {el:.1}s  fps {fps}  256={color256} bits={bits}");
            f.render_widget(Paragraph::new(text).block(Block::default().borders(Borders::ALL)), rest);
        })?;
        let t2 = Instant::now();
        sim_time += t1 - t0;
        draw_time += t2 - t1;
        frames += 1;
        if let Some(rest) = period.checked_sub(t2 - t0) {
            std::thread::sleep(rest);
        }
    }
    let wall = start.elapsed().as_secs_f64();
    let bytes = counter.get() - bytes_at_start;
    io::stdout().execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;
    let json = format!(
        "{{\"cols\":{},\"rows\":{},\"field_rows\":{field_rows},\"frames\":{frames},\"wall\":{wall:.2},\"bytes_per_s\":{:.0},\"sim_ms\":{:.2},\"draw_ms\":{:.2}}}\n",
        size.width,
        size.height,
        bytes as f64 / wall,
        sim_time.as_secs_f64() * 1e3 / frames as f64,
        draw_time.as_secs_f64() * 1e3 / frames as f64,
    );
    std::fs::write(&out, json)?;
    Ok(())
}
