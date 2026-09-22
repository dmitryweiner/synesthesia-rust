//! The FX chain: our own DSP, in the web app's order and with its parameter
//! meanings, but not a re-implementation of the Web Audio nodes (PLAN.md
//! decision 3).
//!
//! ```text
//! mix → [filter | formants | comb] → [chorus/flanger] → [phaser]
//!     → [delay] → [reverb] → [limiter] → master
//! ```

pub mod biquad;
pub mod chain;
pub mod delayline;
pub mod limiter;
pub mod reverb;

pub use chain::FxChain;
