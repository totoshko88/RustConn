//! Split view redesign module
//!
//! This module provides the core data models for tab-scoped split layouts.
//! Each root tab maintains its own independent panel configuration using a
//! binary tree structure that supports recursive nesting.
//!
//! # Architecture
//!
//! - **Tab-scoped layouts**: Each tab owns its split configuration
//! - **Tree-based panel structure**: Panels organized in a binary tree
//! - **Eviction mechanism**: Dropping on occupied panels preserves displaced connections
//! - **Color-coded containers**: Visual identification through unique Color IDs
//!
//! # Module Structure
//!
//! - `types` - Core type definitions (`PanelId`, `TabId`, `SessionId`, `ColorId`, `SplitDirection`)
//! - `tree` - Panel tree structure (`PanelNode`, `LeafPanel`, `SplitNode`)
//! - `model` - Split layout model (`SplitLayoutModel`)
//! - `color` - Color pool management (`ColorPool`)
//! - `error` - Error types (`SplitError`, `DropResult`)
//!
//! # Example
//!
//! ```
//! use rustconn_core::split::{SplitLayoutModel, SplitDirection, SessionId, DropResult};
//!
//! let mut layout = SplitLayoutModel::new();
//!
//! // Initially, there's one panel with no splits
//! assert!(!layout.is_split());
//! assert_eq!(layout.panel_count(), 1);
//!
//! // Split the focused panel vertically
//! let new_panel_id = layout.split(SplitDirection::Vertical).unwrap();
//!
//! // Now we have two panels
//! assert!(layout.is_split());
//! assert_eq!(layout.panel_count(), 2);
//!
//! // Place a session in the new panel
//! let session = SessionId::new();
//! let result = layout.place_in_panel(new_panel_id, session).unwrap();
//! assert!(matches!(result, DropResult::Placed));
//! ```

mod color;
mod error;
mod model;
mod tree;
mod types;

pub use color::{ColorPool, SPLIT_COLORS};
pub use error::{DropResult, SplitError};
pub use model::SplitLayoutModel;
pub use tree::{
    DEFAULT_SPLIT_POSITION, LeafPanel, MAX_SPLIT_POSITION, MIN_SPLIT_POSITION, PanelNode,
    RemoveResult, SplitNode,
};
pub use types::{ColorId, PanelId, SessionId, SplitDirection, TabId};
