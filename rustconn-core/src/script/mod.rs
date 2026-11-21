//! Script library management.
//!
//! Scripts are stored as individual `.rsc` files under `<config_dir>/scripts/`.
//! The `ScriptManager` provides CRUD over those files and validates a script's
//! source through the `.rsc` parser before it is saved.

mod manager;

pub use manager::{ScriptError, ScriptManager, scripts_dir};
