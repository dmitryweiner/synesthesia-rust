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

/// Throws the samples away, but at the right pace — for `--no-sound` and for
/// tests. Without the pacing the render loop would free-run and burn a core.
pub struct NullSink {
    name: String,
    sample_rate: f64,
    next: Option<std::time::Instant>,
}

impl NullSink {
    pub fn new(sample_rate: f64) -> Self {
        Self { name: format!("null ({sample_rate:.0} Hz)"), sample_rate, next: None }
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
        let deadline = self.next.unwrap_or(now);
        if deadline > now {
            std::thread::sleep(deadline - now);
        }
        let span = std::time::Duration::from_secs_f64(block.len() as f64 / self.sample_rate);
        // Never let the deadline fall behind by more than one block.
        self.next = Some(deadline.max(now - span) + span);
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }
}
