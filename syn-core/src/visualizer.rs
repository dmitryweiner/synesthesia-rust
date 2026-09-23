//! The picture's side of the feature bus (PLAN.md decision 5): whatever turns
//! the point and the sound into an image implements [`Visualizer`], and reads
//! nothing else. The terminal picture ([`crate::sim::Picture`]) is the first;
//! a GPU renderer would be the second, fed the same input.

use crate::features::AudioFeatures;
use crate::state::AppState;

/// One step's input: the point as it sounds now (mid-morph included), the
/// latest features, the engine's onset-hit counter and its clock.
#[derive(Clone, Copy, Debug)]
pub struct VizInput<'a> {
    pub state: &'a AppState,
    pub features: AudioFeatures,
    /// Onset hits since the engine started; a visualizer acts on the delta.
    pub hits: u64,
    /// Engine time in seconds — the LFO clock the sound runs on (decision 7).
    pub time: f64,
}

pub trait Visualizer {
    /// Advances by one step of the simulation clock.
    fn step(&mut self, input: &VizInput);
    /// A fresh start: a new point was loaded, or 🎲 was pressed.
    fn reseed(&mut self);
}
