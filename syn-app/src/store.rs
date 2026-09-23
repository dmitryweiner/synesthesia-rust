//! Where things are kept (PLAN.md decision 9): local files, no network.
//!
//! * `$XDG_CONFIG_HOME/synesthesia/config.toml` — settings, hand-editable.
//! * `$XDG_DATA_HOME/synesthesia/last-point.json` — the current point,
//!   restored on the next start.
//! * `$XDG_DATA_HOME/synesthesia/points.json` — the points the user kept.
//!
//! The points are JSON rather than TOML because a point *is* the web app's
//! JSON (decision 2): the same file opens there, and a TOML rendering of a
//! 500-gene nested state would be neither readable nor compatible.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use syn_core::state::AppState;

const APP: &str = "synesthesia";

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config")).join(APP)
}

pub fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"))
        .join(APP)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Sample rate of the output, in Hz.
    pub sample_rate: f64,
    /// Command to pipe samples into; empty means "pick one" (pw-cat, aplay).
    pub audio_command: String,
    /// Frames of latency asked of the player.
    pub latency_frames: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self { sample_rate: 48000.0, audio_command: String::new(), latency_frames: 1024 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NamedPoint {
    pub name: String,
    pub state: AppState,
}

fn read_to_string(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Writes through a temporary file, so an interrupted save cannot leave a
/// half-written point behind.
fn write_atomic(path: &std::path::Path, contents: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// True when no settings file exists yet.
pub fn load_config_path_missing() -> bool {
    !config_dir().join("config.toml").exists()
}

pub fn load_config() -> Config {
    read_to_string(&config_dir().join("config.toml"))
        .and_then(|raw| toml::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save_config(config: &Config) -> Result<(), String> {
    let text = toml::to_string_pretty(config).map_err(|e| e.to_string())?;
    write_atomic(&config_dir().join("config.toml"), &text)
}

pub fn load_last_point() -> Option<AppState> {
    serde_json::from_str(&read_to_string(&data_dir().join("last-point.json"))?).ok()
}

pub fn save_last_point(state: &AppState) -> Result<(), String> {
    let text = serde_json::to_string(state).map_err(|e| e.to_string())?;
    write_atomic(&data_dir().join("last-point.json"), &text)
}

pub fn load_points() -> Vec<NamedPoint> {
    read_to_string(&data_dir().join("points.json"))
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save_points(points: &[NamedPoint]) -> Result<(), String> {
    let text = serde_json::to_string_pretty(points).map_err(|e| e.to_string())?;
    write_atomic(&data_dir().join("points.json"), &text)
}

/// Keeps a point under `name`, replacing one of the same name.
pub fn keep_point(name: &str, state: &AppState) -> Result<usize, String> {
    let mut points = load_points();
    let entry = NamedPoint { name: name.to_string(), state: state.clone() };
    match points.iter().position(|p| p.name == name) {
        Some(i) => points[i] = entry,
        None => points.push(entry),
    }
    save_points(&points)?;
    Ok(points.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn_core::state::presets;

    /// Points must round-trip through the file exactly — the web app reads the
    /// same JSON.
    #[test]
    fn a_saved_point_comes_back_unchanged() {
        let state = &presets()[4].state;
        let text = serde_json::to_string(state).unwrap();
        let back: AppState = serde_json::from_str(&text).unwrap();
        assert_eq!(&back, state);
    }

    #[test]
    fn the_config_round_trips_and_has_sane_defaults() {
        let c = Config::default();
        assert_eq!(c.sample_rate, 48000.0);
        let text = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.sample_rate, c.sample_rate);
        assert_eq!(back.latency_frames, c.latency_frames);
        // A config written by an older build, missing fields, still loads.
        let partial: Config = toml::from_str("sample_rate = 22050.0").unwrap();
        assert_eq!(partial.sample_rate, 22050.0);
        assert_eq!(partial.latency_frames, c.latency_frames);
    }

    #[test]
    fn paths_follow_the_xdg_variables() {
        // Read whatever the environment says; the point is the app suffix.
        assert!(config_dir().ends_with("synesthesia"));
        assert!(data_dir().ends_with("synesthesia"));
    }
}
