//! `core-ir` — Language-agnostic graph IR for code structure and dataflow.
//!
//! Pure data types with no I/O, no rendering, and no language-specific dependencies.
//! This crate is the stable contract that all other spaghetti crates depend on.

mod types;

pub use types::*;

#[cfg(test)]
mod tests;
