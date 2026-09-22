//! The whole model of Synesthesia, with no I/O and no threads: formula
//! generators, the modulation matrix, the FX chain, the point (`AppState`)
//! and the genome. Everything here is deterministic and testable; devices,
//! terminals and files live in the other crates (PLAN.md).

pub mod dsp;
pub mod modmatrix;
pub mod schema;

pub use dsp::generator::{FormulaGenerator, FormulaId, Params};
pub use modmatrix::{effective_param, lfo_value, LfoDef, LfoShape, ModRoute, ModState};

/// Samples per processing block — the size the browser's AudioWorklet uses,
/// and the size every golden take was rendered in (`scripts/dump-golden.mjs`).
pub const BLOCK: usize = 128;
