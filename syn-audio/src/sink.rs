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
    /// Opens the first player that starts. `SYN_AUDIO_CMD` overrides the choice
    /// (the words of a command line; `{rate}` and `{channels}` are substituted).
    pub fn open(sample_rate: u32, channels: u16, latency_frames: u32) -> std::io::Result<Self> {
        let mut attempts: Vec<Vec<String>> = Vec::new();
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

/// Throws samples away at the right pace — for tests and `--no-sound`.
pub struct NullSink {
    name: String,
}

impl Default for NullSink {
    fn default() -> Self {
        Self { name: "null".into() }
    }
}

impl Sink for NullSink {
    fn write(&mut self, _block: &[f32]) -> std::io::Result<()> {
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }
}
