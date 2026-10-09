//! View logic for the sidebar (list items)
use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, DragSource, GestureClick, Image, Label, ListItem, ListView, MultiSelection,
    Orientation, SignalListItemFactory, SingleSelection, TreeExpander, TreeListRow, gdk, glib,
    pango,
};

use crate::i18n::{i18n, i18n_f};
use crate::sidebar::ConnectionItem;
use crate::sidebar_ui;

/// Per-index CSS classes for the sidebar split-membership marker.
///
/// One entry per color in the split pane palette (`SPLIT_COLOR_VALUES`); the
/// matching `background-color` values live in `sidebar_types.rs` so the marker
/// square is tinted to the same color as the connection's split pane. Kept in
/// lock-step with the palette length — add a class here if the palette grows.
const SPLIT_MARKER_CLASSES: [&str; 6] = [
    "sidebar-split-0",
    "sidebar-split-1",
    "sidebar-split-2",
    "sidebar-split-3",
    "sidebar-split-4",
    "sidebar-split-5",
];

/// Applies the split-membership marker state for a `split-color` value.
///
/// `split_color` is `-1` when the connection is not in a split (marker hidden);
/// otherwise it is a palette index and the marker is shown tinted via the
/// matching per-index CSS class. Stale classes from a recycled row are cleared
/// first so the color always reflects the current split.
fn apply_split_marker(marker: &GtkBox, split_color: i32) {
    for class in SPLIT_MARKER_CLASSES {
        marker.remove_css_class(class);
    }
    if let Ok(index) = usize::try_from(split_color) {
        marker.add_css_class(SPLIT_MARKER_CLASSES[index % SPLIT_MARKER_CLASSES.len()]);
        marker.set_visible(true);
    } else {
        marker.set_visible(false);
    }
}

/// Builds the ancestor path for a `TreeListRow` by walking up `parent()`.
///
/// Returns a `›`-separated string of group names leading to the current item
/// (excluding the item itself). Empty when the item is at the root level.
fn ancestor_path(row: &TreeListRow) -> String {
    let mut segments: Vec<String> = Vec::new();
    let mut current = row.parent();
    while let Some(parent_row) = current {
        if let Some(parent_item) = parent_row.item().and_downcast::<ConnectionItem>() {
            segments.push(parent_item.name());
        }
        current = parent_row.parent();
    }
    segments.reverse();
    segments.join(" › ")
}

/// Finds a direct child widget with a given CSS class.
fn find_child_by_css_class(parent: &GtkBox, class: &str) -> Option<gtk4::Widget> {
    let mut child = parent.first_child();
    while let Some(widget) = child {
        if widget.css_classes().iter().any(|c| c == class) {
            return Some(widget);
        }
        child = widget.next_sibling();
    }
    None
}

/// Finds the position of a tree row in a selection model, by identity.
///
/// A `ListItem` normally knows its own position, but one the `ListView` has
/// recycled reports `GTK_INVALID_LIST_POSITION` — and passing that on clears the
/// selection instead of moving it (#298). Searching the model is what the
/// `ListView`-level fallback gesture already does for the same reason (#157).
#[must_use]
pub fn position_of_row(model: &gtk4::SelectionModel, row: &gtk4::TreeListRow) -> Option<u32> {
    let row_obj: &glib::Object = row.upcast_ref();
    (0..model.n_items()).find(|i| model.item(*i).as_ref() == Some(row_obj))
}

/// Selects only `position` in the list view's selection model
/// (works for both single and multi-selection modes).
pub fn select_single_position(model: &gtk4::SelectionModel, position: u32) {
    if let Some(selection) = model.downcast_ref::<SingleSelection>() {
        selection.set_selected(position);
    } else if let Some(selection) = model.downcast_ref::<MultiSelection>() {
        // In multi-selection mode, select only this item for context menu
        selection.unselect_all();
        selection.select_item(position, false);
    }
}

/// Resolves menu-relevant data from a `ConnectionItem` and shows the
/// sidebar context menu pointing at (`x`, `y`) within `widget`.
///
/// Shared by the per-row right-click gesture, the `ListView`-level
/// fallback gesture, and the Menu / Shift+F10 keyboard handler (#157).
pub fn show_context_menu_for_connection_item(
    widget: &gtk4::Widget,
    x: f64,
    y: f64,
    item: &ConnectionItem,
    recording_checker: &Rc<RefCell<Option<Box<dyn Fn(&str) -> bool>>>>,
    activation: sidebar_ui::MenuActivation,
) {
    let is_group = item.is_group();
    let protocol = item.protocol();
    let is_ssh = protocol == "ssh" || protocol == "sftp";
    let is_connected = item.status() == "connected";
    let conn_id = item.id();
    // A connection is in a split when it carries a split-pane colour (>= 0);
    // the sidebar already uses this to draw the split-membership marker
    // (issue #374).
    let in_split = item.split_color() >= 0;
    let is_recording = if is_connected && !conn_id.is_empty() {
        recording_checker
            .borrow()
            .as_ref()
            .is_some_and(|checker| checker(&conn_id))
    } else {
        false
    };
    // Whether this connection has an active external-viewer session (issue
    // #209): gates the Disconnect / Stop tracking items (R5.1).
    let has_external_session = item.external_session();
    // Drives the favorite menu label between "Add to" and "Remove from".
    let is_pinned = item.is_pinned();

    tracing::debug!(
        name = %item.name(),
        %protocol,
        is_group,
        is_connected,
        has_external_session,
        is_pinned,
        "Showing sidebar context menu"
    );

    sidebar_ui::show_context_menu_for_item(
        widget,
        x,
        y,
        &conn_id,
        is_group,
        is_ssh,
        is_connected,
        is_recording,
        has_external_session,
        is_pinned,
        &item.sync_mode(),
        item.is_root_group(),
        item.has_dynamic_folder(),
        in_split,
        activation,
    );
}

/// Sets up a list item widget
///
/// # Accessibility
/// Each list item is set up with proper accessible properties:
/// - Status icons have live region for dynamic updates
/// - Labels are associated with their icons
pub fn setup_list_item(
    _factory: &SignalListItemFactory,
    list_item: &ListItem,
    _group_ops_mode: bool,
    recording_checker: Rc<RefCell<Option<Box<dyn Fn(&str) -> bool>>>>,
) {
    let expander = TreeExpander::new();

    let content_box = GtkBox::new(Orientation::Horizontal, 10);
    // Row rhythm close to an AdwActionRow: a little more vertical breathing
    // room and leading/trailing padding than the bare list row.
    content_box.set_margin_start(6);
    content_box.set_margin_end(6);
    content_box.set_margin_top(7);
    content_box.set_margin_bottom(7);

    let icon = Image::from_icon_name("network-server-symbolic");
    icon.set_pixel_size(16);
    content_box.append(&icon);

    // Connected-state indicator (R6.1): a check mark. This is the first of the
    // three orthogonal, color-independent state shapes that can appear together
    // on one row (R6.5): check = connected, window emblem = external viewer,
    // filled square = split membership. The three silhouettes are mutually
    // distinguishable with no color at all, so the row stays legible in
    // grayscale / high-contrast modes (R6.3/6.4). Color, where present, only
    // reinforces a shape that already carries the meaning.
    let status_icon = Image::from_icon_name("object-select-symbolic");
    status_icon.set_pixel_size(10);
    status_icon.set_visible(false);
    status_icon.add_css_class("status-icon");
    content_box.append(&status_icon);

    let label = Label::new(None);
    label.set_halign(gtk4::Align::Start);
    label.set_hexpand(true);
    label.set_ellipsize(pango::EllipsizeMode::End);
    content_box.append(&label);

    let pin_icon = Image::from_icon_name("starred-symbolic");
    pin_icon.set_pixel_size(12);
    pin_icon.set_visible(false);
    pin_icon.add_css_class("pin-icon");
    // First of the trailing suffix cluster: a small leading margin sets the
    // indicators off from the (ellipsizing, hexpand) label, the way an
    // AdwActionRow separates its suffix widgets from the title (cosmetic §5).
    pin_icon.set_margin_start(4);
    pin_icon.set_tooltip_text(Some(&i18n("Favorite")));
    // A screen reader sees the accessible label, not the tooltip, so an
    // icon-only indicator needs both (house rule in rustconn/AGENTS.md).
    pin_icon.update_property(&[gtk4::accessible::Property::Label(&i18n("Favorite"))]);
    content_box.append(&pin_icon);

    // Notes badge shown when the connection has a description; the actual
    // text is exposed via the icon tooltip (RDM users asked for a visible
    // indicator that documentation exists without opening each entry).
    let note_icon = Image::from_icon_name("document-edit-symbolic");
    note_icon.set_pixel_size(12);
    note_icon.set_visible(false);
    note_icon.add_css_class("note-icon");
    note_icon.add_css_class("dim-label");
    note_icon.update_property(&[gtk4::accessible::Property::Label(&i18n("Has notes"))]);
    content_box.append(&note_icon);

    // Red dot shown while a session of this connection is being recorded —
    // recording is privacy-sensitive and must be visible at a glance.
    let recording_icon = Image::from_icon_name("media-record-symbolic");
    recording_icon.set_pixel_size(10);
    recording_icon.set_visible(false);
    recording_icon.add_css_class("recording-icon");
    recording_icon.set_tooltip_text(Some(&i18n("Recording session")));
    recording_icon.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Recording session",
    ))]);
    content_box.append(&recording_icon);

    // External-viewer emblem (issue #209): shown alongside the green connected
    // status icon while the connection has an active external-viewer session
    // (VNC/RDP/SPICE delegated to a separate viewer process, no notebook tab).
    // A distinct window-shaped icon (not color alone) keeps the state legible
    // in monochrome mode (R2.3/6.4). Appended at the row's trailing edge so it
    // does not sit between the status icon and the label (bind reads the label
    // as the status icon's next sibling).
    let external_icon = Image::from_icon_name("window-new-symbolic");
    external_icon.set_pixel_size(12);
    external_icon.set_visible(false);
    external_icon.add_css_class("external-session-icon");
    external_icon.set_tooltip_text(Some(&i18n("Running in external window")));
    external_icon.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Running in external window",
    ))]);
    content_box.append(&external_icon);

    // Split-membership marker (Phase 2, R6.2): a small filled square tinted
    // with the session's split pane color. The square shape is deliberately
    // distinct from the connected check and the external window emblem so all
    // three stay mutually distinguishable in grayscale (R6.4/6.5); the color
    // only mirrors the split pane and is never the sole cue (a tooltip and
    // accessible label carry the meaning). Sized no larger than the status
    // icon (10px). Hidden until the connection joins a split (split-color >= 0).
    let split_marker = GtkBox::new(Orientation::Horizontal, 0);
    split_marker.set_size_request(9, 9);
    split_marker.set_halign(gtk4::Align::Center);
    split_marker.set_valign(gtk4::Align::Center);
    split_marker.set_visible(false);
    split_marker.add_css_class("split-marker");
    split_marker.set_tooltip_text(Some(&i18n("Part of a split view")));
    split_marker.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Part of a split view",
    ))]);
    content_box.append(&split_marker);

    expander.set_child(Some(&content_box));
    list_item.set_child(Some(&expander));

    // Set up drag source for reorganization
    let drag_source = DragSource::new();
    drag_source.set_actions(gdk::DragAction::MOVE);

    // Store list_item reference for drag prepare
    let list_item_weak_drag = list_item.downgrade();
    drag_source.connect_prepare(move |_source, _x, _y| {
        // Get the item from the list item
        let list_item = list_item_weak_drag.upgrade()?;
        let row = list_item.item()?.downcast::<TreeListRow>().ok()?;
        let item = row.item()?.downcast::<ConnectionItem>().ok()?;

        // Delegate to drag_drop helper
        crate::sidebar::drag_drop::prepare_drag_data(&item)
    });

    // Visual feedback during drag
    // Visual feedback during drag
    let list_item_weak_begin = list_item.downgrade();
    drag_source.connect_drag_begin(move |_source, _drag| {
        if let Some(list_item) = list_item_weak_begin.upgrade()
            && let Some(expander) = list_item.child()
        {
            expander.add_css_class("dragging");
        }
    });

    // Clean up drop indicator when drag ends
    let list_item_weak_end = list_item.downgrade();
    drag_source.connect_drag_end(move |source, _drag, _delete_data| {
        // Remove dragging CSS class
        if let Some(list_item) = list_item_weak_end.upgrade()
            && let Some(expander) = list_item.child()
        {
            expander.remove_css_class("dragging");
        }

        // Find the sidebar and hide the drop indicator
        if let Some(widget) = source.widget()
            && let Some(list_view) = widget.ancestor(ListView::static_type())
        {
            // Remove all drop-related CSS classes
            list_view.remove_css_class("drop-active");
            list_view.remove_css_class("drop-into-group");
        }
    });

    expander.add_controller(drag_source);

    // Set up right-click context menu
    // Note: is_group will be determined at bind time via list_item data
    let gesture = GestureClick::new();
    gesture.set_button(gdk::BUTTON_SECONDARY);
    // Use CAPTURE phase so the gesture fires before any internal handlers.
    // The gesture is attached to the TreeExpander itself (not content_box)
    // because TreeExpander renders indent + arrow widgets to the left of the
    // content for nested items.  When the user right-clicks anywhere on a row
    // at depth >= 2, the event target may be one of those indent/arrow
    // children — which are NOT descendants of content_box.  Attaching to
    // content_box therefore missed right-clicks that land in the indent area.
    // BUTTON_SECONDARY does not conflict with TreeExpander's internal
    // expand/collapse gesture, which only handles BUTTON_PRIMARY.
    gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
    let list_item_weak = list_item.downgrade();
    gesture.connect_pressed(move |gesture, _n_press, x, y| {
        let Some(widget) = gesture.widget() else {
            return;
        };
        let list_item = list_item_weak.upgrade();

        // Resolve the row this press belongs to. `ListItem::item()` is empty
        // while the ListView holds the item recycled but not yet rebound, so
        // fall back to the TreeExpander's own list row — set at bind time,
        // never cleared on unbind, and already what the ListView-level
        // fallback gesture resolves through (#157).
        let row = list_item
            .as_ref()
            .and_then(|li| li.item())
            .and_downcast::<gtk4::TreeListRow>()
            .or_else(|| {
                widget
                    .downcast_ref::<TreeExpander>()
                    .and_then(|e| e.list_row())
            });
        let Some(item) = row
            .as_ref()
            .and_then(|r| r.item())
            .and_downcast::<ConnectionItem>()
        else {
            // Deliberately not claimed: the ListView-level fallback gesture
            // resolves the row from the pointer position instead and would very
            // likely succeed. Claiming a press this handler then did nothing
            // with is what made a right-click look like it had been swallowed —
            // no menu, no log, nothing to go on (issue #298).
            tracing::debug!(
                "Sidebar right-click resolved no connection item — leaving the press to the fallback gesture"
            );
            return;
        };

        // Select the row so the context-menu actions target it: every action
        // re-resolves through the sidebar selection rather than being handed
        // this item. A recycled list item reports GTK_INVALID_LIST_POSITION,
        // and handing that to the selection model *clears* the selection —
        // after which Edit, Duplicate, Rename and Delete all return silently
        // and the row appears to be uneditable (issue #298).
        if let Some(list_view) = widget
            .ancestor(ListView::static_type())
            .and_downcast::<ListView>()
            && let Some(model) = list_view.model()
        {
            let position = list_item
                .map(|li| li.position())
                .filter(|p| *p != gtk4::INVALID_LIST_POSITION)
                .or_else(|| row.as_ref().and_then(|r| position_of_row(&model, r)));
            if let Some(position) = position {
                select_single_position(&model, position);
            } else {
                tracing::debug!(
                    name = %item.name(),
                    "Sidebar right-click could not locate its row in the selection model — leaving the press to the fallback gesture"
                );
                return;
            }
        }

        show_context_menu_for_connection_item(
            &widget,
            x,
            y,
            &item,
            &recording_checker,
            sidebar_ui::MenuActivation::PointerRow,
        );

        // Claim the gesture so the event does not propagate further into
        // ListView / TreeExpander internals.  Without this, GTK4 may apply
        // :active / :focus-within pseudo-classes to the row widget that
        // persist after the context menu is dismissed, causing stale
        // highlight artifacts when right-clicking multiple rows in
        // succession.
        gesture.set_state(gtk4::EventSequenceState::Claimed);
    });
    expander.add_controller(gesture);
}

/// Binds data to a list item
pub fn bind_list_item(
    _factory: &SignalListItemFactory,
    list_item: &ListItem,
    handlers: &Rc<RefCell<std::collections::HashMap<ListItem, Vec<glib::SignalHandlerId>>>>,
    query: &str,
) {
    let Some(expander) = list_item.child().and_downcast::<TreeExpander>() else {
        return;
    };

    let Some(row) = list_item.item().and_downcast::<TreeListRow>() else {
        return;
    };

    expander.set_list_row(Some(&row));

    let Some(item) = row.item().and_downcast::<ConnectionItem>() else {
        return;
    };

    let Some(content_box) = expander.child().and_downcast::<GtkBox>() else {
        return;
    };

    // Find icon — skip emoji labels that may have been prepended
    let icon = {
        let mut child = content_box.first_child();
        loop {
            match child {
                Some(w)
                    if w.downcast_ref::<Image>().is_some()
                        && !w.css_classes().iter().any(|c| c == "status-icon") =>
                {
                    break w.downcast::<Image>().ok();
                }
                Some(w) => child = w.next_sibling(),
                None => break None,
            }
        }
    };
    let Some(icon) = icon else {
        return;
    };

    let Some(status_icon) =
        find_child_by_css_class(&content_box, "status-icon").and_downcast::<Image>()
    else {
        return;
    };

    let Some(label) = status_icon.next_sibling().and_downcast::<Label>() else {
        return;
    };

    // Pin icon is after the label
    let pin_icon = label.next_sibling().and_downcast::<Image>();

    // Pre-compile highlight regex once per bind (not per label)
    let highlight_re = crate::sidebar::search::compile_highlight_regex(query);

    // Helper to set text with highlighting
    let set_label_text = |label: &Label, text: &str| {
        if highlight_re.is_none() {
            label.set_text(text);
        } else {
            let markup = crate::sidebar::search::highlight_match(text, highlight_re.as_ref());
            label.set_markup(&markup);
        }
    };

    if item.is_group() {
        // Use custom icon if set, otherwise default folder icon
        let custom_icon = item.icon();
        let glyph_icon = rustconn_core::dialog_utils::is_glyph_icon(&custom_icon);
        if custom_icon.is_empty() {
            icon.set_icon_name(Some("folder-symbolic"));
            icon.set_visible(true);
        } else if glyph_icon {
            // Emoji/unicode — show as text via icon tooltip, use a generic icon
            // We repurpose the icon widget: hide it and insert a label before it
            icon.set_visible(false);
            // Check if we already have an emoji label (from previous bind)
            let emoji_label = if let Some(first) = content_box.first_child()
                && first.css_classes().iter().any(|c| c == "emoji-icon")
            {
                first.downcast::<Label>().ok()
            } else {
                None
            };
            if let Some(lbl) = emoji_label {
                lbl.set_label(&custom_icon);
                lbl.set_visible(true);
            } else {
                let emoji_lbl = Label::new(Some(&custom_icon));
                emoji_lbl.add_css_class("emoji-icon");
                emoji_lbl.set_width_chars(2);
                content_box.prepend(&emoji_lbl);
            }
        } else {
            // GTK icon name — a name the active theme lacks would draw nothing,
            // so fall back to the default folder icon.
            icon.set_icon_name(Some(crate::icon_render::theme_icon_or(
                &custom_icon,
                "folder-symbolic",
            )));
            icon.set_visible(true);
        }
        set_label_text(&label, &item.name());
        // Groups don't have connection status
        status_icon.set_visible(false);
        // Groups don't show pin icon
        if let Some(ref pin) = pin_icon {
            pin.set_visible(false);
        }
        // Groups don't show the recording indicator (hide a stale icon
        // left over from a recycled connection row)
        if let Some(recording_icon) =
            find_child_by_css_class(&content_box, "recording-icon").and_downcast::<Image>()
        {
            recording_icon.set_visible(false);
        }
        // Groups don't show the external-viewer emblem (hide a stale icon
        // left over from a recycled connection row)
        if let Some(external_icon) =
            find_child_by_css_class(&content_box, "external-session-icon").and_downcast::<Image>()
        {
            external_icon.set_visible(false);
        }
        // Groups don't show the split-membership marker (hide/clear a stale
        // marker left over from a recycled connection row)
        if let Some(split_marker) =
            find_child_by_css_class(&content_box, "split-marker").and_downcast::<GtkBox>()
        {
            apply_split_marker(&split_marker, -1);
        }
        // Groups don't show the notes badge
        if let Some(note_icon) =
            find_child_by_css_class(&content_box, "note-icon").and_downcast::<Image>()
        {
            note_icon.set_visible(false);
        }

        // Show connection count and ancestor path in tooltip
        let child_count = if let Some(children) = row.children() {
            children.n_items()
        } else {
            0
        };
        let path_prefix = ancestor_path(&row);
        let name = item.name();
        let tooltip = match (path_prefix.is_empty(), child_count > 0) {
            (true, true) => format!("{name} ({child_count})"),
            (true, false) => name.clone(),
            (false, true) => format!("{path_prefix} › {name} ({child_count})"),
            (false, false) => format!("{path_prefix} › {name}"),
        };
        expander.set_tooltip_text(Some(&tooltip));

        // Hide stale emoji label if icon is not emoji
        if let Some(first) = content_box.first_child()
            && first.css_classes().iter().any(|c| c == "emoji-icon")
            && !glyph_icon
        {
            first.set_visible(false);
        }

        // Cloud Sync indicator icons
        // Check if a sync-indicator image already exists (from previous bind)
        let sync_indicator = content_box
            .last_child()
            .filter(|w| w.css_classes().iter().any(|c| c == "sync-indicator"))
            .and_downcast::<Image>();

        let sync_mode = item.sync_mode();
        if sync_mode == "master" || sync_mode == "import" {
            let sync_icon = if let Some(existing) = sync_indicator {
                existing
            } else {
                let img = Image::new();
                img.set_pixel_size(14);
                img.add_css_class("sync-indicator");
                content_box.append(&img);
                img
            };

            // Check for sync errors via the sync_error property
            let sync_err = item.sync_error();
            if sync_err.is_empty() {
                // No error — show synced indicator
                sync_icon.set_icon_name(Some("view-refresh-symbolic"));
                sync_icon.remove_css_class("error");
                sync_icon.add_css_class("dim-label");

                let tooltip = if sync_mode == "master" {
                    i18n("Master — synced to cloud")
                } else {
                    i18n("Import — synced from cloud")
                };
                sync_icon.set_tooltip_text(Some(&tooltip));
                sync_icon.update_property(&[gtk4::accessible::Property::Label(&tooltip)]);

                // Override group tooltip to include sync info
                let base = if path_prefix.is_empty() {
                    if child_count > 0 {
                        format!("{} ({child_count})", item.name())
                    } else {
                        item.name()
                    }
                } else if child_count > 0 {
                    format!("{path_prefix} › {} ({child_count})", item.name())
                } else {
                    format!("{path_prefix} › {}", item.name())
                };
                expander.set_tooltip_text(Some(&format!("{base} — {tooltip}")));
            } else {
                // Error state — show warning indicator
                sync_icon.set_icon_name(Some("dialog-warning-symbolic"));
                sync_icon.remove_css_class("dim-label");
                sync_icon.add_css_class("error");

                let tooltip = i18n_f("Sync error: {}", &[&sync_err]);
                sync_icon.set_tooltip_text(Some(&tooltip));
                sync_icon.update_property(&[gtk4::accessible::Property::Label(&tooltip)]);

                let base = if path_prefix.is_empty() {
                    if child_count > 0 {
                        format!("{} ({child_count})", item.name())
                    } else {
                        item.name()
                    }
                } else if child_count > 0 {
                    format!("{path_prefix} › {} ({child_count})", item.name())
                } else {
                    format!("{path_prefix} › {}", item.name())
                };
                expander.set_tooltip_text(Some(&format!("{base} — {tooltip}")));
            }
            sync_icon.set_visible(true);
        } else if let Some(existing) = sync_indicator {
            existing.set_visible(false);
        }

        // Add drop controller for dropping into groups
    } else {
        // Use custom icon if set, otherwise protocol-based icon
        let custom_icon = item.icon();
        let glyph_icon = rustconn_core::dialog_utils::is_glyph_icon(&custom_icon);
        let protocol_icon = sidebar_ui::get_protocol_icon(&item.protocol());
        if custom_icon.is_empty() {
            // Set icon based on protocol
            icon.set_icon_name(Some(protocol_icon));
            icon.set_visible(true);
        } else if glyph_icon {
            // Emoji/unicode
            icon.set_visible(false);
            let emoji_label = if let Some(first) = content_box.first_child()
                && first.css_classes().iter().any(|c| c == "emoji-icon")
            {
                first.downcast::<Label>().ok()
            } else {
                None
            };
            if let Some(lbl) = emoji_label {
                lbl.set_label(&custom_icon);
                lbl.set_visible(true);
            } else {
                let emoji_lbl = Label::new(Some(&custom_icon));
                emoji_lbl.add_css_class("emoji-icon");
                emoji_lbl.set_width_chars(2);
                content_box.prepend(&emoji_lbl);
            }
        } else {
            // GTK icon name — a name the active theme lacks would draw nothing,
            // so fall back to the protocol icon.
            icon.set_icon_name(Some(crate::icon_render::theme_icon_or(
                &custom_icon,
                protocol_icon,
            )));
            icon.set_visible(true);
        }

        // Hide stale emoji label if icon is not emoji
        if let Some(first) = content_box.first_child()
            && first.css_classes().iter().any(|c| c == "emoji-icon")
            && !glyph_icon
        {
            first.set_visible(false);
        }

        set_label_text(&label, &item.name());

        // Show full connection path and host in tooltip for deeply nested items
        let name = item.name();
        let host = item.host();
        let path_prefix = ancestor_path(&row);
        let tooltip = match (path_prefix.is_empty(), host.is_empty() || host == name) {
            (true, true) => name.clone(),
            (true, false) => format!("{name}\n{host}"),
            (false, true) => format!("{path_prefix} › {name}"),
            (false, false) => format!("{path_prefix} › {name}\n{host}"),
        };
        expander.set_tooltip_text(Some(&tooltip));

        // Show pin icon for pinned connections
        if let Some(ref pin) = pin_icon {
            pin.set_visible(item.is_pinned());
        }

        // Setup status monitoring logic
        // Update status icon
        if let Some(status_icon) =
            find_child_by_css_class(&content_box, "status-icon").and_downcast::<gtk4::Image>()
        {
            // Helper to update icon state with accessibility announcements
            let update_icon = |icon: &gtk4::Image, status: &str| {
                icon.remove_css_class("status-connected");
                icon.remove_css_class("status-connecting");
                icon.remove_css_class("status-failed");

                if status == "connected" {
                    icon.set_icon_name(Some("object-select-symbolic"));
                    icon.set_visible(true);
                    icon.add_css_class("status-connected");
                    icon.update_property(&[gtk4::accessible::Property::Label(&i18n("Connected"))]);
                } else if status == "connecting" {
                    icon.set_icon_name(Some("network-transmit-receive-symbolic"));
                    icon.set_visible(true);
                    icon.add_css_class("status-connecting");
                    icon.update_property(&[gtk4::accessible::Property::Label(&i18n("Connecting"))]);
                } else if status == "failed" {
                    icon.set_icon_name(Some("dialog-error-symbolic"));
                    icon.set_visible(true);
                    icon.add_css_class("status-failed");
                    icon.update_property(&[gtk4::accessible::Property::Label(&i18n(
                        "Connection failed",
                    ))]);
                } else {
                    icon.set_visible(false);
                    icon.update_property(&[gtk4::accessible::Property::Label("")]);
                }
            };

            // Initial update
            update_icon(&status_icon, &item.status());

            // Connect to notify::status
            let status_icon_clone = status_icon.clone();
            let handler_id =
                item.connect_notify_local(Some("status"), move |item: &ConnectionItem, _| {
                    update_icon(&status_icon_clone, &item.status());
                });

            // Store handler ID on list_item for cleanup
            handlers
                .borrow_mut()
                .entry(list_item.clone())
                .or_default()
                .push(handler_id);
        }

        // Notes badge: visible when the connection has a description; the
        // tooltip carries the text so it can be read without opening the
        // editor dialog.  Reactive via connect_notify so batch-edit / sync
        // updates propagate without a full sidebar reload.
        if let Some(note_icon) =
            find_child_by_css_class(&content_box, "note-icon").and_downcast::<Image>()
        {
            let update_note_badge = |icon: &Image, desc: &str| {
                if desc.trim().is_empty() {
                    icon.set_visible(false);
                    icon.set_tooltip_text(None);
                } else {
                    icon.set_visible(true);
                    // Truncate long descriptions to keep tooltip readable
                    let tooltip = if desc.len() > 120 {
                        // Find a safe char boundary for truncation
                        let end = desc
                            .char_indices()
                            .take_while(|(i, _)| *i <= 120)
                            .last()
                            .map_or(120, |(i, _)| i);
                        format!("{}…", &desc[..end])
                    } else {
                        desc.to_string()
                    };
                    icon.set_tooltip_text(Some(&tooltip));
                }
            };

            // Initial update
            update_note_badge(&note_icon, &item.description());

            // React to description property changes (batch edit, sync, etc.)
            let note_icon_clone = note_icon.clone();
            let handler_id =
                item.connect_notify_local(Some("description"), move |item: &ConnectionItem, _| {
                    update_note_badge(&note_icon_clone, &item.description());
                });
            handlers
                .borrow_mut()
                .entry(list_item.clone())
                .or_default()
                .push(handler_id);
        }

        // Recording indicator: red dot while a session of this connection
        // is being recorded (driven by the is-recording property, see
        // ConnectionSidebar::update_connection_recording)
        if let Some(recording_icon) =
            find_child_by_css_class(&content_box, "recording-icon").and_downcast::<Image>()
        {
            recording_icon.set_visible(item.is_recording());

            let recording_icon_clone = recording_icon.clone();
            let handler_id =
                item.connect_notify_local(Some("is-recording"), move |item: &ConnectionItem, _| {
                    recording_icon_clone.set_visible(item.is_recording());
                });
            handlers
                .borrow_mut()
                .entry(list_item.clone())
                .or_default()
                .push(handler_id);
        }

        // External-viewer emblem: window icon shown alongside the connected
        // status icon while the connection has an active external-viewer
        // session (driven by the external-session property, see
        // ConnectionSidebar::set_external_session). R2.5/2.6: visible iff the
        // external session count is greater than zero.
        if let Some(external_icon) =
            find_child_by_css_class(&content_box, "external-session-icon").and_downcast::<Image>()
        {
            external_icon.set_visible(item.external_session());

            let external_icon_clone = external_icon.clone();
            let handler_id = item.connect_notify_local(
                Some("external-session"),
                move |item: &ConnectionItem, _| {
                    external_icon_clone.set_visible(item.external_session());
                },
            );
            handlers
                .borrow_mut()
                .entry(list_item.clone())
                .or_default()
                .push(handler_id);
        }

        // Split-membership marker: filled square tinted with the split pane
        // color (driven by the split-color property, see
        // ConnectionSidebar::set_split_color). R6.2: shown while the session is
        // in a split, sized no larger than the status icon; the square shape
        // keeps it distinct from the check/window emblems in grayscale (R6.4/6.5).
        if let Some(split_marker) =
            find_child_by_css_class(&content_box, "split-marker").and_downcast::<GtkBox>()
        {
            apply_split_marker(&split_marker, item.split_color());

            let split_marker_clone = split_marker.clone();
            let handler_id =
                item.connect_notify_local(Some("split-color"), move |item: &ConnectionItem, _| {
                    apply_split_marker(&split_marker_clone, item.split_color());
                });
            handlers
                .borrow_mut()
                .entry(list_item.clone())
                .or_default()
                .push(handler_id);
        }

        // Use the bound name label captured above (`status_icon.next_sibling()`),
        // not `content_box.last_child()`: the last child is the split marker.
        set_label_text(&label, &item.name());
        label.set_tooltip_text(None);
    }
}

/// Unbinds data from a list item
pub fn unbind_list_item(
    _factory: &SignalListItemFactory,
    list_item: &ListItem,
    handlers: &Rc<RefCell<std::collections::HashMap<ListItem, Vec<glib::SignalHandlerId>>>>,
) {
    // Remove signal handlers if any exist
    if let Some(handler_ids) = handlers.borrow_mut().remove(list_item)
        && let Some(row) = list_item.item().and_downcast::<TreeListRow>()
        && let Some(item) = row.item().and_downcast::<ConnectionItem>()
    {
        for handler_id in handler_ids {
            item.disconnect(handler_id);
        }
    }
}
