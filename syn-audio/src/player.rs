//! The audio thread: pulls PCM from the core's player and writes it to a sink.
//!
//! What the sound *is* — commands between control and audio, whole-block
//! rendering, fades, the feature frames — is synesthesia-core's `syn-player`,
//! the same on every platform. What is left here is the console's device side
//! (PLAN.md decision 6): one thread, a blocking sink that paces it, and a
//! stop that fades out first so it never clicks.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use syn_core::BLOCK;
use syn_player::Player;

use crate::sink::Sink;

/// How many blocks go into one write to the device.
const BLOCKS_PER_WRITE: usize = 4;
/// How long a stop waits for the fade out before giving up, in writes: the
/// fade is 80 ms, so ~10 writes at 48 kHz; this is far more than that.
const STOP_WRITES: usize = 256;

/// The thread that plays `player` into a sink, until stopped.
pub struct Output {
    player: Arc<Player>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<Result<(), String>>>,
    sink_name: String,
}

impl Output {
    /// Starts writing `player` into `sink` at once. The player stays silent
    /// until it is faded in.
    pub fn start(player: Arc<Player>, mut sink: Box<dyn Sink>) -> Self {
        let sink_name = sink.name().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let (player, stop) = (Arc::clone(&player), Arc::clone(&stop));
            std::thread::Builder::new()
                .name("syn-audio".into())
                .spawn(move || -> Result<(), String> {
                    let mut buf = Vec::with_capacity(BLOCK * BLOCKS_PER_WRITE);
                    let mut write = |buf: &mut Vec<f32>| {
                        player.render_into(BLOCK * BLOCKS_PER_WRITE, buf);
                        sink.write(buf).map_err(|e| format!("audio device: {e}"))
                    };
                    while !stop.load(Ordering::Relaxed) {
                        write(&mut buf)?;
                    }
                    player.fade_out();
                    for _ in 0..STOP_WRITES {
                        if player.is_silent() {
                            break;
                        }
                        write(&mut buf)?;
                    }
                    Ok(())
                })
                .expect("spawn audio thread")
        };
        Output { player, stop, handle: Some(handle), sink_name }
    }

    pub fn player(&self) -> &Player {
        &self.player
    }

    /// False once the thread has ended — the device went away.
    pub fn is_running(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| !h.is_finished())
    }

    pub fn sink_name(&self) -> &str {
        &self.sink_name
    }

    /// Fades out, stops the thread and reports how it ended.
    pub fn stop(mut self) -> Result<(), String> {
        self.finish()
    }

    fn finish(&mut self) -> Result<(), String> {
        self.stop.store(true, Ordering::Relaxed);
        match self.handle.take() {
            Some(h) => h.join().unwrap_or_else(|_| Err("audio thread panicked".into())),
            None => Ok(()),
        }
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::NullSink;
    use std::sync::Mutex;
    use syn_core::state::presets;

    const SR: f64 = 22050.0;

    fn output(sink: Box<dyn Sink>) -> Output {
        let player = Arc::new(Player::new(SR, &presets()[0].state));
        player.fade_in();
        Output::start(player, sink)
    }

    #[test]
    fn it_plays_and_the_player_publishes_frames() {
        let out = output(Box::new(NullSink::new(SR)));
        let mut seen = 0;
        for _ in 0..100 {
            if out.player().latest_frame().is_some_and(|f| f.time > 0.0 && f.rms > 0.0) {
                seen += 1;
                if seen > 3 {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(seen > 3, "no frames from the audio thread");
        out.player().switch_to(presets()[5].state.clone());
        out.stop().expect("clean stop");
    }

    /// Keeps what it was given, unpaced, so a test sees every sample.
    struct Tape(Arc<Mutex<Vec<f32>>>);

    impl Sink for Tape {
        fn write(&mut self, block: &[f32]) -> std::io::Result<()> {
            self.0.lock().unwrap().extend_from_slice(block);
            std::thread::sleep(std::time::Duration::from_millis(1));
            Ok(())
        }
        fn name(&self) -> &str {
            "tape"
        }
    }

    #[test]
    fn a_stop_fades_out_instead_of_cutting_the_sound() {
        let tape = Arc::new(Mutex::new(Vec::new()));
        let out = output(Box::new(Tape(Arc::clone(&tape))));
        while tape.lock().unwrap().len() < SR as usize {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        out.stop().expect("clean stop");
        let x = tape.lock().unwrap();
        let tail = &x[x.len() - BLOCK..];
        assert!(tail.iter().all(|v| *v == 0.0), "it ends in silence");
        let peak = x[x.len() / 2..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 1e-3, "and it was playing before that");
    }
}
