//! Deterministic fishing primitives without Tauri, Windows or model dependencies.
pub mod controller;
pub mod metrics;
pub mod types;
pub mod vision;
// Existing vision paths remain valid while the desktop re-exports this crate.
pub mod core { pub use crate::types; }
