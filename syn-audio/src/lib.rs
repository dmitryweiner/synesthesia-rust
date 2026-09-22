//! Sound output: the sink and the thread that renders into it.

pub mod player;
pub mod sink;

pub use player::{Command, Frame, Player};
pub use sink::{NullSink, PipeSink, Sink};
