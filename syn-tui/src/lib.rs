//! The console interface: one screen, drawn from a snapshot, plus key decoding.
//! The loop lives in `syn-app`.

pub mod app;

pub use app::{decode, draw, poll_action, Action, Screen, View};
