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
    /// Score candidates in the background and pick the best on a press.
    pub scout: bool,
    /// Seconds rendered per candidate, and at what rate.
    pub scout_seconds: f64,
    pub scout_sample_rate: f64,
    /// Candidates per direction.
    pub scout_candidates: usize,
    /// Threads the scout renders on; 0 means every core but two. With every
    /// core busy, the audio thread and `pw-cat` lose to it and the sound
    /// drops out on each 👍/👎 (GRAPHICS.md, the soak).
    pub scout_threads: usize,
    /// Screen redraws per second. Every one of them is paid for outside this
    /// process — the terminal emulator repaints its window and X composites it,
    /// both in software on this machine, which is where the cycles go. Eight is
    /// enough for meters that breathe; raise it if the spectrum looks choppy.
    /// Key presses always redraw at once, whatever this is.
    pub ui_fps: f64,
    /// What fills the screen (GRAPHICS.md): "spectrum", "panel" (the
    /// picture in the spectrum's place) or "full" — what `v` last left it at.
    pub viz: String,
    /// Picture frames drawn a second. Like `ui_fps`, the terminal pays for
    /// each one: 8 costs it about a third of a core here, 4 about a quarter.
    pub viz_fps: f64,
    /// Simulation steps a second — how fast the pattern grows, whatever the
    /// frame rate. The web app steps once per animation frame.
    pub sim_hz: f64,
    /// "truecolor", "256", or "auto" to go by `COLORTERM`.
    pub viz_color: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sample_rate: 48000.0,
            audio_command: String::new(),
            latency_frames: 1024,
            scout: true,
            scout_seconds: 30.0,
            scout_sample_rate: 22050.0,
            scout_candidates: 3,
            scout_threads: 0,
            ui_fps: 8.0,
            viz: "panel".into(),
            viz_fps: 8.0,
            sim_hz: 30.0,
            viz_color: "auto".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NamedPoint {
    pub name: String,
    #[serde(deserialize_with = "sanitized")]
    pub state: AppState,
}

/// A point from a file, read the way every app reads one (synesthesia-core's
/// `point::sanitize`): fields a newer model added get their defaults, values
/// are clamped, either spelling of the name is taken. Strict serde would
/// leave a file from an older build incomplete (no tanpura, no bowl).
fn sanitized<'de, D: serde::Deserializer<'de>>(d: D) -> Result<AppState, D::Error> {
    let raw = serde_json::Value::deserialize(d)?;
    syn_core::point::sanitize(&raw).ok_or_else(|| serde::de::Error::custom("not a point"))
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
    let raw: serde_json::Value =
        serde_json::from_str(&read_to_string(&data_dir().join("last-point.json"))?).ok()?;
    syn_core::point::sanitize(&raw)
}

pub fn save_last_point(state: &AppState) -> Result<(), String> {
    let text = serde_json::to_string(state).map_err(|e| e.to_string())?;
    write_atomic(&data_dir().join("last-point.json"), &text)
}

pub fn load_points() -> Vec<NamedPoint> {
    read_points().unwrap_or_default()
}

/// The kept points, an entry that is not a point skipped; an error when the
/// file is there but is not a list at all — then it must not be overwritten.
fn read_points() -> Result<Vec<NamedPoint>, String> {
    let Some(raw) = read_to_string(&data_dir().join("points.json")) else {
        return Ok(Vec::new());
    };
    parse_points(&raw)
}

fn parse_points(raw: &str) -> Result<Vec<NamedPoint>, String> {
    let entries: Vec<serde_json::Value> = serde_json::from_str(raw)
        .map_err(|e| format!("points.json is not a list of points ({e}); left as it is"))?;
    Ok(entries.into_iter().filter_map(|e| serde_json::from_value(e).ok()).collect())
}

pub fn save_points(points: &[NamedPoint]) -> Result<(), String> {
    let text = serde_json::to_string_pretty(points).map_err(|e| e.to_string())?;
    write_atomic(&data_dir().join("points.json"), &text)
}

/// Keeps a point under `name`, replacing one of the same name.
pub fn keep_point(name: &str, state: &AppState) -> Result<usize, String> {
    let mut points = read_points()?;
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

    /// Files written before the model came from synesthesia-core: no
    /// `delayShimmer`, no tanpura or bowl, the name spelled either way. They
    /// must load with every value they had — `load_points` falls back to an
    /// empty list on a parse error, and the next keep would overwrite the file.
    #[test]
    fn a_point_file_from_the_consoles_own_core_still_loads() {
        let mut old = serde_json::to_value(&presets()[0].state).unwrap();
        old["audio"]["fx"].as_object_mut().unwrap().remove("delayShimmer");
        let formulas = old["audio"]["formulas"].as_object_mut().unwrap();
        formulas.remove("tanpura");
        formulas.remove("bowl");
        for key in ["preset_name", "presetName"] {
            let mut file = old.clone();
            let name = file.as_object_mut().unwrap().remove("preset_name").unwrap();
            file[key] = name;
            let list = serde_json::json!([{ "name": "kept", "state": file }, { "name": "junk", "state": 5 }]);
            let back = parse_points(&list.to_string()).unwrap();
            assert_eq!(back.len(), 1, "an entry that is not a point is skipped, not the whole file");
            assert_eq!(back[0].state.preset_name.as_deref(), Some("Fractal garden"), "{key}");
            assert_eq!(back[0].state, presets()[0].state, "{key}");
        }
        assert!(
            parse_points("{not json").is_err(),
            "an unreadable file is an error, never an empty list to save over"
        );
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
        assert_eq!(partial.ui_fps, c.ui_fps);
    }

    #[test]
    fn paths_follow_the_xdg_variables() {
        // Read whatever the environment says; the point is the app suffix.
        assert!(config_dir().ends_with("synesthesia"));
        assert!(data_dir().ends_with("synesthesia"));
    }
}
