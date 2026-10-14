//! Sound output: the sink and the thread that plays the core's player into it.

pub mod player;
pub mod sink;

pub use player::Output;
pub use sink::{NullSink, PipeSink, Sink};
pub use syn_player::{Frame, Player};
