//! Where the samples go.
//!
//! This machine has PipeWire running and no ALSA development headers, so the
//! default sink pipes raw `f32` into `pw-cat` (or `aplay`) instead of linking a
//! sound library. Writing blocks when the device buffer is full, which is what
//! paces the render loop. A `cpal` backend can slot in behind the same trait
//! once `libasound2-dev` is installed — see PLAN.md.

use std::io::{ErrorKind, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

/// A place to write mono blocks of `f32` samples.
pub trait Sink: Send {
    fn write(&mut self, block: &[f32]) -> std::io::Result<()>;
    /// How the sink describes itself in the UI.
    fn name(&self) -> &str;
}

/// A child process reading raw samples from its standard input.
pub struct PipeSink {
    child: Child,
    stdin: ChildStdin,
    name: String,
    bytes: Vec<u8>,
}

impl PipeSink {
    /// Opens the first player that starts, trying `command` (if given), then
    /// `SYN_AUDIO_CMD`, then pw-cat, then aplay. In a command, `{rate}` and
    /// `{channels}` are substituted.
    pub fn open(sample_rate: u32, channels: u16, latency_frames: u32) -> std::io::Result<Self> {
        Self::open_with("", sample_rate, channels, latency_frames)
    }

    pub fn open_with(
        command: &str,
        sample_rate: u32,
        channels: u16,
        latency_frames: u32,
    ) -> std::io::Result<Self> {
        let mut attempts: Vec<Vec<String>> = Vec::new();
        if !command.trim().is_empty() {
            attempts.push(command.split_whitespace().map(str::to_string).collect());
        }
        if let Ok(custom) = std::env::var("SYN_AUDIO_CMD") {
            attempts.push(custom.split_whitespace().map(str::to_string).collect());
        }
        attempts.push(
            format!(
                "pw-cat -p --format f32 --rate {sample_rate} --channels {channels} --latency {latency_frames} --raw -"
            )
            .split_whitespace()
            .map(str::to_string)
            .collect(),
        );
        attempts.push(
            format!(
                "aplay -q -t raw -f FLOAT_LE -c {channels} -r {sample_rate} --period-size={latency_frames} -"
            )
            .split_whitespace()
            .map(str::to_string)
            .collect(),
        );

        let mut last_err = None;
        for words in attempts {
            let words: Vec<String> = words
                .into_iter()
                .map(|w| {
                    w.replace("{rate}", &sample_rate.to_string()).replace("{channels}", &channels.to_string())
                })
                .collect();
            let Some((program, args)) = words.split_first() else { continue };
            match Command::new(program).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).spawn() {
                Ok(mut child) => {
                    let stdin = child.stdin.take().expect("piped stdin");
                    return Ok(Self { child, stdin, name: words.join(" "), bytes: Vec::new() });
                }
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| std::io::Error::new(ErrorKind::NotFound, "no audio player found")))
    }
}

impl Sink for PipeSink {
    fn write(&mut self, block: &[f32]) -> std::io::Result<()> {
        self.bytes.clear();
        self.bytes.reserve(block.len() * 4);
        for s in block {
            self.bytes.extend_from_slice(&s.to_le_bytes());
        }
        self.stdin.write_all(&self.bytes)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for PipeSink {
    fn drop(&mut self) {
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Throws the samples away, but at the right pace — for `--no-sound`, for
/// tests, and for the soak on a machine without PipeWire (`soak.py --null`).
///
/// It is a model of a device: a buffer of [`NULL_BUFFER_FRAMES`] (the
/// config's default latency) drained in real time. A write blocks while the
/// buffer is fuller than that, which paces the render loop as a device would;
/// a write that finds it already empty is an **underrun** — the render thread
/// fell behind real time by more than the buffer, which is what an xrun is.
/// The count is printed when the sink goes away.
pub struct NullSink {
    name: String,
    sample_rate: f64,
    buffer: std::time::Duration,
    /// When the audio written so far runs out; `None` before the first write.
    drained_at: Option<std::time::Instant>,
    started: Option<std::time::Instant>,
    underruns: u64,
    longest_gap: std::time::Duration,
}

/// The buffer a [`NullSink`] models, in frames — `latency_frames`' default.
pub const NULL_BUFFER_FRAMES: u32 = 1024;

impl NullSink {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            name: format!("null ({sample_rate:.0} Hz)"),
            sample_rate,
            buffer: std::time::Duration::from_secs_f64(f64::from(NULL_BUFFER_FRAMES) / sample_rate),
            drained_at: None,
            started: None,
            underruns: 0,
            longest_gap: std::time::Duration::ZERO,
        }
    }

    /// Times the buffer ran dry so far.
    pub fn underruns(&self) -> u64 {
        self.underruns
    }
}

impl Default for NullSink {
    fn default() -> Self {
        Self::new(48000.0)
    }
}

impl Sink for NullSink {
    fn write(&mut self, block: &[f32]) -> std::io::Result<()> {
        let now = std::time::Instant::now();
        self.started.get_or_insert(now);
        let span = std::time::Duration::from_secs_f64(block.len() as f64 / self.sample_rate);
        let from = match self.drained_at {
            Some(t) if t < now => {
                self.underruns += 1;
                self.longest_gap = self.longest_gap.max(now - t);
                now
            }
            Some(t) => t,
            None => now,
        };
        let drained_at = from + span;
        self.drained_at = Some(drained_at);
        // Block while more than the buffer is queued, as a full device does.
        let wake = drained_at.checked_sub(self.buffer).unwrap_or(now);
        if wake > now {
            std::thread::sleep(wake - now);
        }
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for NullSink {
    fn drop(&mut self) {
        if let Some(t0) = self.started {
            eprintln!(
                "null sink: {} underruns (longest gap {:.1} ms) in {:.1} s",
                self.underruns,
                self.longest_gap.as_secs_f64() * 1e3,
                t0.elapsed().as_secs_f64()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn a_writer_that_keeps_up_has_no_underruns_and_is_paced() {
        let mut sink = NullSink::new(48000.0);
        let block = vec![0.0f32; 512];
        let t0 = Instant::now();
        for _ in 0..50 {
            sink.write(&block).unwrap();
        }
        // 50 × 512 frames = 533 ms of audio, minus the buffer still queued.
        let took = t0.elapsed().as_secs_f64();
        assert!(took > 0.45 && took < 0.6, "{took}");
        assert_eq!(sink.underruns(), 0);
    }

    #[test]
    fn a_writer_late_by_more_than_the_buffer_is_an_underrun() {
        let mut sink = NullSink::new(48000.0);
        let block = vec![0.0f32; 512];
        for _ in 0..10 {
            sink.write(&block).unwrap();
        }
        // The buffer holds ~21–32 ms; stalling 80 ms empties it.
        std::thread::sleep(Duration::from_millis(80));
        sink.write(&block).unwrap();
        sink.write(&block).unwrap();
        assert_eq!(sink.underruns(), 1);
    }
}
