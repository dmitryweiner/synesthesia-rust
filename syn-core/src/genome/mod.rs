//! The point as the search sees it: a vector of genes, the codec that maps it
//! to and from an `AppState`, the evolutionary operators and the explorer that
//! drives 👍/👎.

pub mod codec;
pub mod evolve;
pub mod explorer;
pub mod genes;
pub mod scout;

pub use codec::{decode_genome, encode_genome};
pub use evolve::{mutate, random_genome, repair, MutateOptions};
pub use explorer::{Explorer, ExplorerAction};
pub use genes::{gene_index, gene_value, genes, Genome};
pub use scout::{ScoutKind, ScoutResult, ScoutSettings};
