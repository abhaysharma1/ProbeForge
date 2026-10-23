//! Shared contract crate for ProbeForge services and workers.
//!
//! The Rust types mirror `packages/contracts/schemas/probeforge.v1.schema.json`.
//! Shared fixtures in `packages/contracts/fixtures/v1` are exercised by both
//! this crate and the TypeScript validator so the two languages cannot drift
//! silently.

mod model;
mod validation;

pub use model::*;
pub use validation::*;
