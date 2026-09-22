//! The render loop on its own thread.
//!
//! The control side never touches the engine: it sends commands and reads
//! frames, both over bounded channels (PLAN.md decision 6). Points that the
//! audio thread replaces are handed back so they are dropped on the control
//! thread instead of between two blocks.

use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;

use syn_core::state::AppState;
use syn_core::{Engine, BLOCK};

use crate::sink::Sink;

/// How many blocks go into one write to the device.
const BLOCKS_PER_WRITE: usize = 4;
/// How often a frame is published, in blocks (~21 ms at 48 kHz).
const FRAME_EVERY: usize = 8;

pub enum Command {
    /// Glide to a new point (the genome morph sends these continuously).
    SetState(Box<AppState>),
    /// Hard switch: drop the tails and start the new point cleanly.
    SwitchTo(Box<AppState>),
    Stop,
}

/// What the UI gets to draw, published from the audio thread.
#[derive(Clone, Copy, Debug, Default)]
pub struct Frame {
    /// Engine time in seconds — the LFO clock.
    pub time: f64,
    pub peak: f32,
    pub rms: f32,
    pub limiter_db: f32,
}

pub struct Player {
    commands: SyncSender<Command>,
    frames: Receiver<Frame>,
    retired: Receiver<Box<AppState>>,
    handle: Option<JoinHandle<Result<(), String>>>,
    sample_rate: f64,
    sink_name: String,
}

impl Player {
    /// Starts rendering `state` into `sink` at once.
    pub fn start(sample_rate: f64, state: &AppState, mut sink: Box<dyn Sink>) -> Self {
        let (tx_cmd, rx_cmd) = sync_channel::<Command>(64);
        let (tx_frame, rx_frame) = sync_channel::<Frame>(8);
        let (tx_retired, rx_retired) = sync_channel::<Box<AppState>>(64);
        let sink_name = sink.name().to_string();

        let seed = state_seed(state);
        let mut engine = Engine::new(sample_rate, state, seed);
        let handle = std::thread::Builder::new()
            .name("syn-audio".into())
            .spawn(move || -> Result<(), String> {
                let mut buf = vec![0.0f32; BLOCK * BLOCKS_PER_WRITE];
                let mut since_frame = 0usize;
                loop {
                    for chunk in buf.chunks_mut(BLOCK) {
                        engine.render(chunk);
                    }
                    sink.write(&buf).map_err(|e| format!("audio device: {e}"))?;

                    since_frame += BLOCKS_PER_WRITE;
                    if since_frame >= FRAME_EVERY {
                        since_frame = 0;
                        let frame = measure(&buf, engine.time(), engine.limiter_reduction_db());
                        // A UI that fell behind loses frames, never the sound.
                        match tx_frame.try_send(frame) {
                            Ok(()) | Err(TrySendError::Full(_)) => {}
                            Err(TrySendError::Disconnected(_)) => return Ok(()),
                        }
                    }

                    while let Ok(cmd) = rx_cmd.try_recv() {
                        match cmd {
                            Command::SetState(s) => {
                                engine.set_state(&s);
                                let _ = tx_retired.try_send(s);
                            }
                            Command::SwitchTo(s) => {
                                engine.switch_to(&s);
                                let _ = tx_retired.try_send(s);
                            }
                            Command::Stop => return Ok(()),
                        }
                    }
                }
            })
            .expect("spawn audio thread");

        Player {
            commands: tx_cmd,
            frames: rx_frame,
            retired: rx_retired,
            handle: Some(handle),
            sample_rate,
            sink_name,
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub fn sink_name(&self) -> &str {
        &self.sink_name
    }

    /// Glide to a new point.
    pub fn set_state(&self, state: AppState) {
        self.send(Command::SetState(Box::new(state)));
    }

    /// Hard switch to a new point.
    pub fn switch_to(&self, state: AppState) {
        self.send(Command::SwitchTo(Box::new(state)));
    }

    fn send(&self, cmd: Command) {
        let _ = self.commands.try_send(cmd);
        // Free what the audio thread handed back, here rather than there.
        while self.retired.try_recv().is_ok() {}
    }

    /// The latest frame, if the audio thread published one since the last call.
    pub fn latest_frame(&self) -> Option<Frame> {
        let mut last = None;
        while let Ok(f) = self.frames.try_recv() {
            last = Some(f);
        }
        last
    }

    /// Blocks until a frame arrives or the thread is gone.
    pub fn wait_frame(&self) -> Option<Frame> {
        self.frames.recv().ok()
    }

    /// Stops the thread and reports how it ended.
    pub fn stop(mut self) -> Result<(), String> {
        let _ = self.commands.try_send(Command::Stop);
        match self.handle.take() {
            Some(h) => h.join().unwrap_or_else(|_| Err("audio thread panicked".into())),
            None => Ok(()),
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.commands.try_send(Command::Stop);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn measure(buf: &[f32], time: f64, limiter_db: f64) -> Frame {
    let mut peak = 0.0f32;
    let mut sum = 0.0f64;
    for s in buf {
        peak = peak.max(s.abs());
        sum += f64::from(*s) * f64::from(*s);
    }
    Frame { time, peak, rms: (sum / buf.len() as f64).sqrt() as f32, limiter_db: limiter_db as f32 }
}

/// A point always sounds the same on the noisy generators from run to run,
/// without a global seed: the seed is the point itself.
fn state_seed(state: &AppState) -> u32 {
    let json = serde_json::to_string(state).unwrap_or_default();
    let mut h: u32 = 2_166_136_261;
    for b in json.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(16_777_619);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::NullSink;
    use syn_core::state::presets;

    #[test]
    fn it_renders_and_publishes_frames() {
        let state = presets()[0].state.clone();
        let player = Player::start(22050.0, &state, Box::<NullSink>::default());
        let mut seen = 0;
        for _ in 0..50 {
            if let Some(f) = player.latest_frame() {
                if f.time > 0.0 && f.rms > 0.0 {
                    seen += 1;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            if seen > 3 {
                break;
            }
        }
        assert!(seen > 3, "no frames from the audio thread");
        player.stop().expect("clean stop");
    }

    #[test]
    fn switching_points_keeps_it_running() {
        let a = presets()[0].state.clone();
        let b = presets()[5].state.clone();
        let player = Player::start(22050.0, &a, Box::<NullSink>::default());
        player.wait_frame();
        player.switch_to(b);
        player.set_state(presets()[5].state.clone());
        assert!(player.wait_frame().is_some());
        player.stop().expect("clean stop");
    }
}
