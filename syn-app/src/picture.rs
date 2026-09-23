//! The picture's thread (GRAPHICS.md decisions 4 and 5): it steps the field
//! on its own clock, `sim_hz` times a second, and draws a frame `viz_fps`
//! times a second into a buffer the interface copies from. The control
//! thread only hands it inputs — the point, the latest feature frame, the
//! size — so neither the keys nor the scout ever wait for a Gray–Scott step,
//! and a slow terminal cannot slow the pattern down.
//!
//! This is not the audio thread: plain mutexes are fine here, and the
//! thread is not pinned anywhere — on this machine it wants a big core.

use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use syn_audio::Frame;
use syn_core::sim::{Image, Picture};
use syn_core::state::AppState;
use syn_core::visualizer::{Visualizer, VizInput};

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Simulation steps a second — the pattern's pace.
    pub sim_hz: f64,
    /// Frames drawn a second — what the terminal is asked to show.
    pub draw_fps: f64,
    pub seed: u32,
}

/// What the control thread hands over.
struct Inputs {
    state: Arc<AppState>,
    frame: Frame,
    /// Pixels; zero until the interface has laid the picture out.
    size: (usize, usize),
    running: bool,
    reseed: bool,
    quit: bool,
}

/// The latest frame drawn, and a counter that moves when it changes.
struct Output {
    seq: u64,
    image: Image,
}

pub struct PictureThread {
    inputs: Arc<Mutex<Inputs>>,
    output: Arc<Mutex<Output>>,
    seen: u64,
    handle: Option<JoinHandle<()>>,
}

/// A poisoned lock here only means the other side panicked mid-update of
/// plain data; the data is still the best there is.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl PictureThread {
    pub fn start(state: &AppState, settings: Settings) -> Self {
        let inputs = Arc::new(Mutex::new(Inputs {
            state: Arc::new(state.clone()),
            frame: Frame::default(),
            size: (0, 0),
            running: false,
            reseed: false,
            quit: false,
        }));
        let output = Arc::new(Mutex::new(Output { seq: 0, image: Image::default() }));
        let handle = {
            let (inputs, output) = (Arc::clone(&inputs), Arc::clone(&output));
            std::thread::Builder::new()
                .name("picture".into())
                .spawn(move || run(&inputs, &output, settings))
                .expect("spawning the picture thread")
        };
        Self { inputs, output, seen: 0, handle: Some(handle) }
    }

    pub fn set_state(&self, state: &AppState) {
        lock(&self.inputs).state = Arc::new(state.clone());
    }

    pub fn set_frame(&self, frame: Frame) {
        lock(&self.inputs).frame = frame;
    }

    /// The picture's size in pixels: two a cell vertically.
    pub fn set_size(&self, w: usize, h: usize) {
        lock(&self.inputs).size = (w, h);
    }

    /// A stopped picture costs nothing: the thread idles until it runs again.
    pub fn set_running(&self, running: bool) {
        lock(&self.inputs).running = running;
    }

    pub fn reseed(&self) {
        lock(&self.inputs).reseed = true;
    }

    /// Copies the latest frame into `into` if it is newer than the last one
    /// taken; true when it did.
    pub fn take_new(&mut self, into: &mut Image) -> bool {
        let out = lock(&self.output);
        if out.seq == self.seen {
            return false;
        }
        self.seen = out.seq;
        into.clone_from(&out.image);
        true
    }
}

impl Drop for PictureThread {
    fn drop(&mut self) {
        lock(&self.inputs).quit = true;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn run(inputs: &Mutex<Inputs>, output: &Mutex<Output>, settings: Settings) {
    let step = Duration::from_secs_f64(1.0 / settings.sim_hz.clamp(1.0, 120.0));
    let draw_every = Duration::from_secs_f64(1.0 / settings.draw_fps.clamp(1.0, 60.0));
    let mut picture: Option<Picture> = None;
    let mut next = Instant::now();
    let mut last_draw: Option<Instant> = None;
    loop {
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        }
        next += step;
        // Behind by more than a few steps (a busy core, a suspended laptop):
        // drop them rather than race to catch up.
        if Instant::now() > next + step * 3 {
            next = Instant::now() + step;
        }

        let (state, frame, size, reseed) = {
            let mut i = lock(inputs);
            if i.quit {
                return;
            }
            if !i.running || i.size.0 == 0 || i.size.1 == 0 {
                continue;
            }
            (Arc::clone(&i.state), i.frame, i.size, std::mem::take(&mut i.reseed))
        };

        let pic = picture.get_or_insert_with(|| Picture::new(size.0, size.1, settings.seed));
        pic.resize(size.0, size.1);
        if reseed {
            pic.reseed();
        }
        pic.step(&VizInput { state: &state, features: frame.features, hits: frame.hits, time: frame.time });

        let now = Instant::now();
        if last_draw.is_none_or(|t| now.duration_since(t) >= draw_every) {
            let image = pic.draw(frame.time);
            let mut out = lock(output);
            out.image.clone_from(image);
            out.seq += 1;
            last_draw = Some(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn_core::state::presets;

    fn wait_for_frame(t: &mut PictureThread, img: &mut Image) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if t.take_new(img) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn it_draws_only_while_running_and_follows_the_size() {
        let settings = Settings { sim_hz: 60.0, draw_fps: 30.0, seed: 1 };
        let mut t = PictureThread::start(&presets()[0].state, settings);
        let mut img = Image::default();
        t.set_size(16, 8);
        std::thread::sleep(Duration::from_millis(100));
        assert!(!t.take_new(&mut img), "nothing is drawn before it runs");

        t.set_running(true);
        assert!(wait_for_frame(&mut t, &mut img));
        assert_eq!((img.w, img.h), (16, 8));
        assert!(img.rgb.iter().any(|&c| c != [0, 0, 0]));

        t.set_size(24, 10);
        let deadline = Instant::now() + Duration::from_secs(5);
        while (img.w, img.h) != (24, 10) && Instant::now() < deadline {
            wait_for_frame(&mut t, &mut img);
        }
        assert_eq!((img.w, img.h), (24, 10));
    }
}
