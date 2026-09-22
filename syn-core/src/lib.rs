//! The whole model of Synesthesia, with no I/O and no threads: formula
//! generators, the modulation matrix, the FX chain, the point (`AppState`)
//! and the genome. Everything here is deterministic and testable; devices,
//! terminals and files live in the other crates (PLAN.md).

pub mod dsp;
pub mod engine;
pub mod fx;
pub mod modmatrix;
pub mod schema;
pub mod state;
pub mod wav;

pub use dsp::generator::{FormulaGenerator, FormulaId, Params};
pub use engine::{render_offline, Engine};
pub use fx::FxChain;
pub use modmatrix::{effective_param, lfo_value, LfoDef, LfoShape, ModRoute, ModState};
pub use state::{AppState, AudioState, FormulaSnapshot, FxState, Preset};

/// Samples per processing block — the size the browser's AudioWorklet uses,
/// and the size every golden take was rendered in (`scripts/dump-golden.mjs`).
pub const BLOCK: usize = 128;
