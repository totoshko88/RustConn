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
//! use crate::split_view::{DropSource, ConnectionId, SplitViewAdapter};
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
    SplitDirection, SplitTitleSource, SplitViewBridge, create_colored_circle_icon,
    get_split_color_class, get_split_indicator_class, get_tab_color_class,
};
use gtk4::prelude::*;
use rustconn_core::models::WorkspaceSplitLayout;
use rustconn_core::split::balanced_split_plan;
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
/// After the splits are created it reapplies the saved root `split_ratio` via
/// [`SplitViewBridge::set_root_split_position`]; a freshly created split opens
/// at 0.5, so the persisted fraction has to be written back once the tree
/// exists. Only the root split's ratio is restored — the nested splits of a
/// balanced multi-panel grid still open evenly, since `WorkspaceSplitLayout`
/// records a single fraction (a per-split `Vec<f64>` would be the upgrade path).
pub fn apply_layout(
    window: &gtk4::Window,
    layout: &WorkspaceSplitLayout,
    notebook: &SharedNotebook,
    session_bridges: &SessionSplitBridges,
) {
    if !layout.is_split {
        return;
    }
    let total_splits = layout.extra_splits + 1;

    // Get window dimensions for initial aspect ratio.
    // default_size() returns the configured size (before mapping); if unavailable
    // fall back to a 4:3 aspect which still produces a good balanced grid.
    let (win_w, win_h) = window.default_size();

    let window_weak = window.downgrade();
    let notebook = notebook.clone();
    let session_bridges = session_bridges.clone();
    let root_ratio = layout.split_ratio;

    gtk4::glib::idle_add_local_once(move || {
        let Some(win) = window_weak.upgrade() else {
            return;
        };

        run_balanced_splits(
            &win,
            &notebook,
            &session_bridges,
            f64::from(win_w),
            f64::from(win_h),
            total_splits,
            |_, _| {},
        );

        // Reapply the saved root split ratio. Every split above opened at the
        // model default of 0.5; the persisted fraction is written back into the
        // active session's bridge, which rebuilds the widget tree and lets the
        // idle-poll in build_split_widget position the divider. Skipped for a
        // near-even ratio so an ordinary 50/50 layout is not rebuilt for nothing.
        if (root_ratio - 0.5).abs() > 0.01
            && let Some(active_session) = notebook.get_active_session_id()
            && let Some(bridge) = session_bridges.borrow().get(&active_session)
        {
            bridge.set_root_split_position(root_ratio);
        }
    });
}

/// Splits the active tab `splits` times into a balanced grid.
///
/// The steps come from [`balanced_split_plan`]: each one targets the largest
/// panel and halves it along its longest side. Every step fires the regular
/// `win.split-*` action, so the panel is wired exactly like one the user made.
/// All steps run synchronously, so the active tab cannot change between them
/// unless `on_new_pane` changes it.
///
/// `on_new_pane(step, pane_uuid)` runs right after step `step` created the
/// empty panel `pane_uuid`; "Split with Other Tabs" places a session there.
/// A callback that moves the active tab must restore it before returning,
/// because the next step resolves the layout through the active tab.
pub fn run_balanced_splits(
    win: &gtk4::Window,
    notebook: &SharedNotebook,
    session_bridges: &SessionSplitBridges,
    width: f64,
    height: f64,
    splits: usize,
    mut on_new_pane: impl FnMut(usize, Uuid),
) {
    let plan = balanced_split_plan(width, height, splits);
    // Panel UUIDs by plan index. The original panel's UUID is only known once
    // the first split has created its sibling.
    let mut panels: Vec<Option<Uuid>> = vec![None];

    for (idx, step) in plan.iter().enumerate() {
        // Before every split but the first, point the bridge's focused pane at
        // the target so the action splits the correct one.
        if idx > 0
            && let Some(target_uuid) = panels[step.target]
            && let Some(active_session) = notebook.get_active_session_id()
            && let Some(bridge) = session_bridges.borrow().get(&active_session)
        {
            bridge.set_focused_pane(Some(target_uuid));
        }

        let action = if step.horizontal {
            "win.split-horizontal"
        } else {
            "win.split-vertical"
        };
        let _ = WidgetExt::activate_action(win, action, None);

        // The bridge now has one more pane: the new one is the UUID we do not
        // track yet.
        let (all_uuids, first_empty): (Vec<Uuid>, Option<Uuid>) = notebook
            .get_active_session_id()
            .and_then(|active| {
                session_bridges
                    .borrow()
                    .get(&active)
                    .map(|b| (b.pane_ids(), b.first_empty_pane_uuid()))
            })
            .unwrap_or_default();
        // At step 0 neither pane is tracked yet and `pane_ids()` has no stable
        // order, so "the first untracked one" could be the original pane that
        // holds the session. The new pane is the empty one.
        let new_uuid = if idx == 0 {
            first_empty
        } else {
            all_uuids
                .iter()
                .find(|u| !panels.contains(&Some(**u)))
                .copied()
        };
        if idx == 0 {
            // The original pane is the one that is NOT new.
            panels[0] = all_uuids.iter().find(|u| Some(**u) != new_uuid).copied();
        }
        panels.push(new_uuid);

        if let Some(new_uuid) = new_uuid {
            on_new_pane(idx, new_uuid);
        }
    }
}
