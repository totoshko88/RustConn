//! Split view module for tab-scoped split layouts
//!
//! This module provides the GUI layer implementation for the split view redesign.
//! It bridges the core data models from `rustconn-core::split` with GTK4/libadwaita
//! widgets.
//!
//! # Architecture
//!
//! The split view system is divided between two crates:
//!
//! - **`rustconn-core::split`**: Core data models (`SplitLayoutModel`, `PanelNode`, etc.)
//! - **`rustconn::split_view`**: GUI adapters and GTK widget management
//!
//! This separation ensures that business logic can be tested without GTK dependencies.
//!
//! # Module Structure
//!
//! - `adapter` - `SplitViewAdapter` bridging core models to GTK widgets
//! - `types` - GUI-specific types (`DropSource`, `ConnectionId`)
//! - `bridge` - `SplitViewBridge` providing legacy-compatible API over new system
//!
//! # Example
//!
//! ```ignore
//! use rustconn::split_view::{DropSource, ConnectionId, SplitViewAdapter};
//! use rustconn_core::split::{SessionId, SplitDirection};
//!
//! // Create a new split view adapter
//! let mut adapter = SplitViewAdapter::new();
//!
//! // Split the focused panel vertically
//! let new_panel_id = adapter.split(SplitDirection::Vertical).unwrap();
//!
//! // Create a drop source for a sidebar item
//! let connection_id = ConnectionId::new();
//! let source = DropSource::sidebar_item(connection_id);
//!
//! // Create a drop source for a root tab
//! let session_id = SessionId::new();
//! let source = DropSource::root_tab(session_id);
//! ```

mod adapter;
mod bridge;
pub mod types;

// Re-export the new adapter
pub use adapter::SplitViewAdapter;
// Re-export the bridge for legacy-compatible API (replaces SplitTerminalView)
pub use bridge::{
    SPLIT_COLOR_VALUES, SPLIT_PANE_COLORS, SessionColorMap, SharedSessions, SharedTerminals,
    SplitDirection, SplitViewBridge, create_colored_circle_icon, get_split_color_class,
    get_split_indicator_class, get_tab_color_class,
};
use gtk4::prelude::*;
use rustconn_core::models::WorkspaceSplitLayout;
// Re-export GUI-specific types
pub use types::{ConnectionId, DropOutcome, DropSource, EvictionAction, SourceCleanup};
use uuid::Uuid;

use crate::window::types::{SessionSplitBridges, SharedNotebook};

/// Restores a saved workspace split layout onto the active window using
/// balanced splitting: each split targets the largest panel and divides it
/// along its longest side, producing a near-uniform grid.
///
/// The algorithm ignores the saved `split_directions` — they recorded the
/// *original user actions* which may have been sequential (all splitting the
/// focused pane). Instead we compute directions dynamically so that the
/// restored layout is always balanced regardless of the original order.
///
/// All splits fire in a single idle iteration so the active tab does not
/// change between them (SSH tabs connecting in the background could steal
/// focus between timeouts).
///
/// ponytail: restores split direction only, not `split_ratio` (panes open 50/50);
/// upgrade path: expose a ratio setter on `SplitViewBridge` and apply it post-split.
pub fn apply_layout(
    window: &gtk4::Window,
    layout: &WorkspaceSplitLayout,
    notebook: &SharedNotebook,
    session_bridges: &SessionSplitBridges,
) {
    if !layout.is_split {
        return;
    }
    let extra = layout.extra_splits;
    let total_splits = extra + 1;

    // Get window dimensions for initial aspect ratio.
    // default_size() returns the configured size (before mapping); if unavailable
    // fall back to a 4:3 aspect which still produces a good balanced grid.
    let (win_w, win_h) = window.default_size();
    let initial_w: f64 = if win_w > 0 { f64::from(win_w) } else { 1280.0 };
    let initial_h: f64 = if win_h > 0 { f64::from(win_h) } else { 960.0 };

    let window_weak = window.downgrade();
    let notebook = notebook.clone();
    let session_bridges = session_bridges.clone();

    gtk4::glib::idle_add_local_once(move || {
        let Some(win) = window_weak.upgrade() else {
            return;
        };

        // Track logical panel sizes for the balanced algorithm.
        // Each entry: (width, height, pane_uuid). The first panel is the entire
        // window area; subsequent splits halve one dimension.
        // We don't know the UUID of the initial panel until after the first split
        // creates the bridge, so we use a sentinel (nil) and resolve it later.
        let mut panels: Vec<(f64, f64, Option<Uuid>)> = vec![(initial_w, initial_h, None)];

        for split_idx in 0..total_splits {
            // Find the largest panel (by area; tie-break by longest side).
            let largest_idx = panels
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| {
                    let area_a = a.0 * a.1;
                    let area_b = b.0 * b.1;
                    area_a
                        .partial_cmp(&area_b)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| {
                            let longest_a = a.0.max(a.1);
                            let longest_b = b.0.max(b.1);
                            longest_a
                                .partial_cmp(&longest_b)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        })
                })
                .map(|(i, _)| i)
                .unwrap_or(0);

            let (pw, ph, pane_uuid) = panels[largest_idx];

            // Determine direction: split along the longest side.
            // width >= height → split-vertical (creates left/right, halves width)
            // height > width → split-horizontal (creates top/bottom, halves height)
            let split_horizontal = ph > pw;
            let action = if split_horizontal {
                "win.split-horizontal"
            } else {
                "win.split-vertical"
            };

            // Before splitting (except the very first split), switch the
            // bridge's focused pane to the target panel so the action splits
            // the correct one.
            if split_idx > 0
                && let Some(target_uuid) = pane_uuid
                && let Some(active_session) = notebook.get_active_session_id()
            {
                let bridges = session_bridges.borrow();
                if let Some(bridge) = bridges.get(&active_session) {
                    bridge.set_focused_pane(Some(target_uuid));
                }
            }

            // Fire the split action synchronously.
            let _ = WidgetExt::activate_action(&win, action, None);

            // After the split, resolve the new pane UUID from the bridge.
            // The bridge now has one more pane than before. The new pane is
            // the one not present in our tracking list.
            let new_uuid: Option<Uuid> =
                notebook.get_active_session_id().and_then(|active_session| {
                    let bridges = session_bridges.borrow();
                    let bridge = bridges.get(&active_session)?;
                    let all_uuids = bridge.pane_ids();
                    // Find the UUID that isn't already tracked.
                    let tracked: Vec<Uuid> = panels.iter().filter_map(|(_, _, u)| *u).collect();
                    all_uuids.into_iter().find(|u| !tracked.contains(u))
                });

            // Also resolve the original pane's UUID if this was the first split.
            if split_idx == 0 {
                // After first split the bridge has 2 panes. The focused one
                // (original) is the one that's NOT new_uuid.
                if let Some(active_session) = notebook.get_active_session_id() {
                    let bridges = session_bridges.borrow();
                    if let Some(bridge) = bridges.get(&active_session) {
                        let all_uuids = bridge.pane_ids();
                        let original_uuid =
                            all_uuids.iter().find(|u| Some(**u) != new_uuid).copied();
                        panels[largest_idx].2 = original_uuid;
                    }
                }
            }

            // Update tracking: split the largest panel into two halves.
            let (new_w, new_h) = if split_horizontal {
                (pw, ph / 2.0)
            } else {
                (pw / 2.0, ph)
            };
            // The original panel shrinks to half.
            panels[largest_idx] = (new_w, new_h, panels[largest_idx].2);
            // The new panel is the other half.
            panels.push((new_w, new_h, new_uuid));
        }
    });
}
