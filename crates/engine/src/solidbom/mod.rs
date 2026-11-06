//! SolidBOM's reviewed-source analysis: the imported STEP source, its definitions, occurrences
//! and the mapping from a tree or viewport pick to them. Exact geometry only; the manifest is
//! scoped to one source hash and extractor version.

pub mod extract;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub use extract::{EXTRACTOR_VERSION, analysis_matches, extract, resolve_pick};
