//! UI helper functions for connection sidebar
//!
//! This module contains UI-related helper functions for creating popovers,
//! context menus, and other visual elements used by the sidebar widget.

use crate::i18n::i18n;
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Label, Orientation, Separator};
use std::cell::RefCell;

thread_local! {
    /// Tracks the currently open context menu popover across the entire application.
    /// When a new context menu is requested (sidebar or split view), the previous
    /// one is closed first to prevent GTK4 popover lifecycle conflicts (issue #87).
    static ACTIVE_POPOVER: RefCell<Option<gtk4::Popover>> = const { RefCell::new(None) };
    /// True while one of our own handlers is popping a context menu down.
    /// The `closed` handler reads it (the emission is synchronous) to tell
    /// an intentional close apart from a compositor dismissal (#157).
    static INTENTIONAL_POPDOWN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Compositor-dismissal retry window (#157): KWin cancels non-grabbing
/// xdg_popups (autohide=false) on focus changes, so on KDE Plasma the menu
/// could close immediately after popup. If a context-menu popover closes
/// within this window without any user interaction, it is re-opened once
/// with autohide=true (grab taken — KWin keeps it; costs the #87
/// double-click nicety on that attempt, which beats no menu at all).
const EARLY_DISMISS_WINDOW: std::time::Duration = std::time::Duration::from_millis(300);

/// Pops a context-menu popover down, marking the close as intentional so
/// the early-dismissal retry in `show_popover` does not re-open it.
fn popdown_intentionally(popover: &gtk4::Popover) {
    INTENTIONAL_POPDOWN.with(|flag| {
        flag.set(true);
        popover.popdown();
        flag.set(false);
    });
}

/// Closes and unparents any currently active context menu popover.
///
/// Call this before creating a new popover to prevent GTK4 grab conflicts
/// where two popovers compete for the event grab (issue #87).
pub fn close_active_popover() {
    ACTIVE_POPOVER.with(|cell| {
        // Take the popover out first, releasing the borrow, so that the
        // synchronous `connect_closed` callback (which calls
        // `clear_active_popover`) does not hit a double-borrow panic.
        let popover = cell.borrow_mut().take();
        if let Some(old) = popover {
            // popdown() may synchronously emit `closed` → connect_closed
            // handler already calls unparent().  Only call unparent()
            // ourselves if the popover still has a parent after popdown
            // (which happens when the popover was not visible and closed
            // signal did not fire).
            popdown_intentionally(&old);
            if old.parent().is_some() {
                old.unparent();
            }
        }
    });
}

/// Registers a popover as the currently active context menu.
///
/// The popover's `connect_closed` handler should call [`clear_active_popover`]
/// to clean up the reference.
pub fn set_active_popover(popover: &gtk4::Popover) {
    ACTIVE_POPOVER.with(|cell| {
        *cell.borrow_mut() = Some(popover.clone());
    });
}

/// Clears the active popover reference if it matches the given popover.
///
/// Called from `connect_closed` handlers to avoid stale references.
pub fn clear_active_popover(popover: &gtk4::Popover) {
    ACTIVE_POPOVER.with(|cell| {
        let mut active = cell.borrow_mut();
        if active.as_ref().is_some_and(|a| a == popover) {
            *active = None;
        }
    });
}

/// How the context menu was invoked. Determines popover grab behaviour:
///
/// - `PointerRow`: per-row right-click gesture. Uses `autohide=false` so a
///   right-click on a *different* row reaches that row's gesture directly
///   (#87). Early compositor dismissals are retried with a grab (#157).
/// - `PointerFallback`: ListView-level right-click / touch long-press
///   fallback used when per-row dispatch fails (deep nesting, #157). Pops
///   up with `autohide=true` immediately: the grab is then tied to the
///   fresh input serial of the triggering press, which KWin honours —
///   a deferred re-popup with a stale serial is dismissed again.
/// - `Keyboard`: Menu key / Shift+F10. Takes a grab and moves focus to the
///   first menu item so the menu is keyboard-navigable.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MenuActivation {
    PointerRow,
    PointerFallback,
    Keyboard,
}

impl MenuActivation {
    fn takes_grab(self) -> bool {
        !matches!(self, Self::PointerRow)
    }
}

/// A single item in the context menu.
enum ContextMenuItem {
    /// A clickable action with a label and a window action name (without "win." prefix).
    Action { label: String, action: String },
    /// A visual separator between groups of actions.
    Separator,
}

impl ContextMenuItem {
    fn action(label: &str, action: &str) -> Self {
        Self::Action {
            label: label.to_string(),
            action: action.to_string(),
        }
    }
}

/// Shows the context menu for a connection item with group awareness
#[expect(
    clippy::fn_params_excessive_bools,
    clippy::too_many_arguments,
    reason = "function parameters mirror Clap-derived flags 1:1; bundling would only restate them"
)]
pub fn show_context_menu_for_item(
    widget: &impl IsA<gtk4::Widget>,
    x: f64,
    y: f64,
    is_group: bool,
    is_ssh: bool,
    is_connected: bool,
    is_recording: bool,
    sync_mode: &str,
    is_root_group: bool,
    has_dynamic_folder: bool,
    activation: MenuActivation,
) {
    let Some(root) = widget.root() else { return };
    let Some(window) = root.downcast_ref::<gtk4::ApplicationWindow>() else {
        return;
    };

    let mut items: Vec<ContextMenuItem> = Vec::new();

    if is_group {
        let is_import_group = sync_mode == "import";

        // § Primary actions
        items.push(ContextMenuItem::action(
            &i18n("Connect All"),
            "connect-all-in-group",
        ));
        // § Organisation
        items.push(ContextMenuItem::Separator);
        items.push(ContextMenuItem::action(&i18n("Rename"), "rename-item"));
        // § Creation / properties (GNOME HIG: properties-like items before delete)
        // Import groups: hide "New Connection in Group" (connections managed by sync)
        if !is_import_group {
            items.push(ContextMenuItem::Separator);
            items.push(ContextMenuItem::action(
                &i18n("New Connection in Group"),
                "new-connection-in-group",
            ));
        }
        items.push(ContextMenuItem::action(&i18n("Edit"), "edit-connection"));
        // § Cloud Sync (flat items, GNOME HIG)
        if sync_mode == "master" || is_import_group {
            items.push(ContextMenuItem::Separator);
            items.push(ContextMenuItem::action(&i18n("Sync Now"), "sync-now"));
        } else if is_root_group && sync_mode == "none" {
            items.push(ContextMenuItem::Separator);
            items.push(ContextMenuItem::action(
                &i18n("Enable Cloud Sync..."),
                "edit-connection",
            ));
        }
        // § Dynamic Folder
        if has_dynamic_folder {
            items.push(ContextMenuItem::Separator);
            items.push(ContextMenuItem::action(
                &i18n("Refresh Dynamic Folder"),
                "refresh-dynamic-folder",
            ));
        }
    } else {
        // § Primary actions
        items.push(ContextMenuItem::action(&i18n("Connect"), "connect"));
        items.push(ContextMenuItem::action(&i18n("Pin / Unpin"), "toggle-pin"));
        // § Organisation
        items.push(ContextMenuItem::Separator);
        items.push(ContextMenuItem::action(&i18n("Rename"), "rename-item"));
        items.push(ContextMenuItem::action(
            &i18n("Duplicate"),
            "duplicate-connection",
        ));
        items.push(ContextMenuItem::action(
            &i18n("Duplicate via Wizard…"),
            "duplicate-via-wizard",
        ));
        items.push(ContextMenuItem::action(
            &i18n("Move to Group..."),
            "move-to-group",
        ));
        // § Utilities (copy, tools, network)
        items.push(ContextMenuItem::Separator);
        items.push(ContextMenuItem::action(
            &i18n("Copy Username"),
            "copy-username",
        ));
        items.push(ContextMenuItem::action(
            &i18n("Copy Password"),
            "copy-password",
        ));
        items.push(ContextMenuItem::action(
            &i18n("Run Snippet..."),
            "run-snippet-for-connection",
        ));
        if is_ssh {
            items.push(ContextMenuItem::action(&i18n("Open SFTP"), "open-sftp"));
        }
        items.push(ContextMenuItem::action(&i18n("Wake On LAN"), "wake-on-lan"));
        items.push(ContextMenuItem::action(
            &i18n("Check if Online"),
            "check-host-online",
        ));
        if is_connected {
            items.push(ContextMenuItem::Separator);
            if is_recording {
                items.push(ContextMenuItem::action(
                    &i18n("Stop Recording"),
                    "stop-recording",
                ));
            } else {
                items.push(ContextMenuItem::action(
                    &i18n("Start Recording"),
                    "start-recording",
                ));
            }
        }
        // § Creation / properties (GNOME HIG: properties-like items before delete)
        items.push(ContextMenuItem::Separator);
        items.push(ContextMenuItem::action(
            &i18n("New Connection"),
            "new-connection-from-context",
        ));
        items.push(ContextMenuItem::action(&i18n("Edit"), "edit-connection"));
    }

    // Delete section (always last, visually separated)
    // Import groups: hide "Delete" (group lifecycle managed by sync)
    let is_import_group = is_group && sync_mode == "import";
    if !is_import_group {
        items.push(ContextMenuItem::Separator);
        items.push(ContextMenuItem::action(
            &i18n("Delete"),
            "delete-connection",
        ));
    }

    show_popover(widget, window, &items, x, y, activation);
}

/// Shows the context menu for empty space in the sidebar
pub fn show_empty_space_context_menu(widget: &impl IsA<gtk4::Widget>, x: f64, y: f64) {
    let Some(root) = widget.root() else { return };
    let Some(window) = root.downcast_ref::<gtk4::ApplicationWindow>() else {
        return;
    };

    let items = vec![
        ContextMenuItem::action(&i18n("Quick Connect"), "quick-connect"),
        ContextMenuItem::action(&i18n("New Connection"), "new-connection"),
        ContextMenuItem::action(&i18n("New Group"), "new-group"),
        ContextMenuItem::action(&i18n("New Smart Folder"), "new-smart-folder"),
        ContextMenuItem::Separator,
        ContextMenuItem::action(&i18n("Import..."), "import"),
        ContextMenuItem::action(&i18n("Export..."), "export"),
    ];

    show_popover(widget, window, &items, x, y, MenuActivation::PointerRow);
}

/// Creates and shows a `Popover` with button items that directly activate
/// window actions. This bypasses `PopoverMenu` action-resolution issues
/// inside `ListView` / `TreeExpander` widget hierarchies.
///
/// The popover uses `autohide = false` so that GTK4 does not grab the
/// pointer.  This allows a right-click on a *different* sidebar row to
/// immediately fire its `GestureClick`, which calls
/// [`close_active_popover`] before opening a new menu — giving seamless
/// "click another item → old menu closes, new menu opens" behaviour
/// without the double-click problem caused by autohide consuming the
/// first click.
///
/// Dismissal is handled by:
/// - [`close_active_popover`] (called at the start of every context-menu
///   request and by the `GestureClick` on the `ScrolledWindow` for
///   empty-space clicks).
/// - Each button's click handler closing the popover before activating
///   the action.
fn show_popover(
    widget: &impl IsA<gtk4::Widget>,
    window: &gtk4::ApplicationWindow,
    items: &[ContextMenuItem],
    x: f64,
    y: f64,
    activation: MenuActivation,
) {
    close_active_popover();

    let popover = gtk4::Popover::new();
    popover.set_parent(widget);
    // Pin the popover's own background/foreground to the libadwaita popover
    // palette. Third-party GTK themes (e.g. Breeze on KDE) otherwise colour
    // the flat-button text to clash with the popover background, rendering the
    // menu rows invisible (#181). Styled in assets/style.css.
    popover.add_css_class("context-menu-popover");

    // `accessible-role` is construct-only — use the builder so screen
    // readers announce the container as a menu.
    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .accessible_role(gtk4::AccessibleRole::Menu)
        .build();
    vbox.add_css_class("context-menu");

    for item in items {
        match item {
            ContextMenuItem::Action { label, action } => {
                let button = Button::builder()
                    .accessible_role(gtk4::AccessibleRole::MenuItem)
                    .build();
                button.add_css_class("flat");
                button.add_css_class("context-menu-item");

                let lbl = Label::new(Some(label));
                lbl.set_xalign(0.0);
                button.set_child(Some(&lbl));

                let window_weak = window.downgrade();
                let action_name = action.clone();
                let popover_weak = popover.downgrade();
                button.connect_clicked(move |_| {
                    if let Some(p) = popover_weak.upgrade() {
                        popdown_intentionally(&p);
                    }
                    if let Some(w) = window_weak.upgrade() {
                        gtk4::prelude::ActionGroupExt::activate_action(&w, &action_name, None);
                    }
                });

                vbox.append(&button);
            }
            ContextMenuItem::Separator => {
                vbox.append(&Separator::new(Orientation::Horizontal));
            }
        }
    }

    popover.set_child(Some(&vbox));

    #[expect(
        clippy::cast_possible_truncation,
        reason = "value range fits the target type by construction in this code path"
    )]
    let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
    popover.set_pointing_to(Some(&rect));
    // PointerRow uses autohide=false so GTK4 does not grab the pointer.
    // With a grab, a right-click on a *different* sidebar row is consumed by
    // the autohide mechanism and never reaches the row's GestureClick
    // handler, causing the context menu to intermittently fail to open
    // (issue #87). Dismissal is then handled manually:
    // - Left-click dismiss gesture on ScrolledWindow (CAPTURE phase)
    // - close_active_popover() called before every new context menu
    // - Each button closes the popover before activating its action
    // - Escape key handler below
    //
    // PointerFallback / Keyboard take the grab immediately (see
    // [`MenuActivation`]): on KDE Plasma a non-grabbing xdg_popup is
    // cancelled by KWin on the focus change that follows the click, and a
    // deferred re-popup cannot acquire a grab because its input serial is
    // stale (#157, deep-nesting reports).
    popover.set_autohide(activation.takes_grab());
    popover.set_has_arrow(false);

    // Escape key closes the popover (autohide=false means GTK4 won't do it)
    let key_controller = gtk4::EventControllerKey::new();
    let popover_weak_esc = popover.downgrade();
    key_controller.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape {
            if let Some(p) = popover_weak_esc.upgrade() {
                popdown_intentionally(&p);
            }
            gtk4::glib::Propagation::Stop
        } else {
            gtk4::glib::Propagation::Proceed
        }
    });
    popover.add_controller(key_controller);

    // Arrow-key navigation between menu items with wrap-around, plus
    // Home/End (standard GNOME menu behavior). The first item is focused
    // on popup, so the controller receives key events immediately.
    let vbox_for_nav = vbox.downgrade();
    let nav_controller = gtk4::EventControllerKey::new();
    nav_controller.connect_key_pressed(move |_, key, _, _| {
        let Some(menu_box) = vbox_for_nav.upgrade() else {
            return gtk4::glib::Propagation::Proceed;
        };
        let items = menu_item_buttons(&menu_box);
        if items.is_empty() {
            return gtk4::glib::Propagation::Proceed;
        }
        let focused = items.iter().position(|b| b.has_focus());
        let target = match key {
            gdk::Key::Down => focused.map_or(0, |i| (i + 1) % items.len()),
            gdk::Key::Up => {
                focused.map_or(items.len() - 1, |i| (i + items.len() - 1) % items.len())
            }
            gdk::Key::Home => 0,
            gdk::Key::End => items.len() - 1,
            _ => return gtk4::glib::Propagation::Proceed,
        };
        items[target].grab_focus();
        gtk4::glib::Propagation::Stop
    });
    popover.add_controller(nav_controller);

    // Close the popover when focus leaves it (e.g. user presses a keyboard
    // shortcut that opens a dialog, or clicks a toolbar button).  This
    // replaces the autohide behaviour for the focus-loss scenario (#93)
    // without the pointer-grab side-effect that breaks right-click
    // switching (#87).
    //
    // We watch the window's focus-widget property: when it changes to a
    // widget that is NOT a descendant of this popover, we close the menu.
    // The handler is disconnected in `connect_closed` to avoid accumulating
    // stale handlers on the window (#168).
    let popover_weak_focus = popover.downgrade();
    let window_for_handler = window.clone();
    let focus_handler_id = window.connect_notify_local(Some("focus-widget"), move |win, _| {
        let Some(pop) = popover_weak_focus.upgrade() else {
            return;
        };
        // If the popover is not visible, nothing to do
        if !pop.is_visible() {
            return;
        }
        // Check if the new focus widget is inside the popover
        let win_ref: &gtk4::Window = win.upcast_ref();
        if let Some(focus) = gtk4::prelude::GtkWindowExt::focus(win_ref) {
            if !focus.is_ancestor(&pop) {
                popdown_intentionally(&pop);
            }
        } else {
            // No focus widget — dialog or another window took focus
            popdown_intentionally(&pop);
        }
    });

    // Store the handler ID so we can disconnect it when the popover closes.
    // Use Rc<Cell> to move the handler ID into the connect_closed closure.
    let focus_handler_cell = std::rc::Rc::new(std::cell::Cell::new(Some(focus_handler_id)));
    let focus_handler_for_close = focus_handler_cell.clone();
    let popup_at = std::time::Instant::now();
    let retried = std::rc::Rc::new(std::cell::Cell::new(false));
    popover.connect_closed(move |p| {
        // Disconnect the focus-widget handler to prevent accumulation (#168)
        if let Some(handler_id) = focus_handler_for_close.take() {
            window_for_handler.disconnect(handler_id);
        }

        let intentional = INTENTIONAL_POPDOWN.with(std::cell::Cell::get);
        // Only the non-grabbing PointerRow popup retries: a grabbing popup
        // that the compositor still dismissed cannot be saved by re-popping
        // (the input serial is already stale, #157).
        if !activation.takes_grab()
            && popup_at.elapsed() < EARLY_DISMISS_WINDOW
            && !retried.get()
            && !intentional
        {
            retried.set(true);
            tracing::debug!(
                "Context menu dismissed {}ms after popup — retrying with autohide=true",
                popup_at.elapsed().as_millis()
            );
            p.set_autohide(true);
            let p_weak = p.downgrade();
            gtk4::glib::idle_add_local_once(move || {
                if let Some(p) = p_weak.upgrade()
                    && p.parent().is_some()
                {
                    p.popup();
                }
            });
            return;
        }

        // Defer unparent to idle: this handler runs synchronously inside
        // GTK's hide/popdown sequence, and unparenting here can drop the
        // popover's last reference mid-emission — GTK then touches the
        // freed object ("gtk_popover_get_autohide: assertion GTK_IS_POPOVER
        // failed", #157 follow-up report).
        let p_for_unparent = p.clone();
        gtk4::glib::idle_add_local_once(move || {
            if p_for_unparent.parent().is_some() {
                p_for_unparent.unparent();
            }
        });
        clear_active_popover(p);
    });

    set_active_popover(&popover);
    popover.popup();

    // For keyboard invocation, focus the first item so the menu is
    // immediately keyboard-navigable (Menu key / Shift+F10, #157). Deferred
    // to idle so the popover is mapped before the focus grab. Pointer paths
    // skip this: moving keyboard focus right after popup is itself a focus
    // change that makes KWin cancel a non-grabbing popup (#157).
    if activation == MenuActivation::Keyboard {
        let vbox_for_focus = vbox.downgrade();
        gtk4::glib::idle_add_local_once(move || {
            if let Some(menu_box) = vbox_for_focus.upgrade()
                && let Some(first) = menu_item_buttons(&menu_box).first()
            {
                first.grab_focus();
            }
        });
    }
}

/// Collects the menu-item buttons of a context-menu box in visual order.
fn menu_item_buttons(menu_box: &GtkBox) -> Vec<Button> {
    let mut items = Vec::new();
    let mut child = menu_box.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Ok(button) = widget.downcast::<Button>() {
            items.push(button);
        }
    }
    items
}

/// Returns the appropriate icon name for a protocol string
///
/// For ZeroTrust connections, the protocol string may include provider info
/// in the format "zerotrust:provider" (e.g., "zerotrust:aws", "zerotrust:gcloud").
/// All ZeroTrust connections use the same icon regardless of provider.
///
/// Icons are aligned with `rustconn_core::protocol::icons::get_protocol_icon()`.
#[must_use]
pub fn get_protocol_icon(protocol: &str) -> &'static str {
    rustconn_core::get_protocol_icon_by_name(protocol)
}

/// Creates the bulk actions toolbar for group operations mode
///
/// Compact icon-only pill buttons matching the protocol filter bar style.
#[must_use]
pub fn create_bulk_actions_bar() -> GtkBox {
    let bar = GtkBox::new(Orientation::Horizontal, 4);
    bar.set_margin_start(12);
    bar.set_margin_end(12);
    bar.set_margin_top(6);
    bar.set_margin_bottom(6);
    bar.set_halign(gtk4::Align::Center);
    bar.add_css_class("bulk-actions-bar");

    let new_group_button = Button::from_icon_name("folder-new-symbolic");
    new_group_button.add_css_class("pill");
    new_group_button.add_css_class("bulk-action");
    new_group_button.set_tooltip_text(Some(&i18n("New Group")));
    new_group_button.set_action_name(Some("win.new-group"));
    new_group_button
        .update_property(&[gtk4::accessible::Property::Label(&i18n("Create new group"))]);
    bar.append(&new_group_button);

    let move_button = Button::from_icon_name("folder-drag-accept-symbolic");
    move_button.add_css_class("pill");
    move_button.add_css_class("bulk-action");
    move_button.set_tooltip_text(Some(&i18n("Move to Group")));
    move_button.set_action_name(Some("win.move-selected-to-group"));
    move_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Move selected connections to group",
    ))]);
    bar.append(&move_button);

    let cluster_button = Button::from_icon_name("network-workgroup-symbolic");
    cluster_button.add_css_class("pill");
    cluster_button.add_css_class("bulk-action");
    cluster_button.set_tooltip_text(Some(&i18n("Create Cluster")));
    cluster_button.set_action_name(Some("win.cluster-from-selection"));
    cluster_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Create cluster from selected connections",
    ))]);
    bar.append(&cluster_button);

    let batch_edit_button = Button::from_icon_name("document-edit-symbolic");
    batch_edit_button.add_css_class("pill");
    batch_edit_button.add_css_class("bulk-action");
    batch_edit_button.set_tooltip_text(Some(&i18n("Batch Edit")));
    batch_edit_button.set_action_name(Some("win.batch-edit-selected"));
    batch_edit_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Edit selected connections together",
    ))]);
    bar.append(&batch_edit_button);

    let select_all_button = Button::from_icon_name("edit-select-all-symbolic");
    select_all_button.add_css_class("pill");
    select_all_button.add_css_class("bulk-action");
    select_all_button.set_tooltip_text(Some(&i18n("Select All")));
    select_all_button.set_action_name(Some("win.select-all"));
    select_all_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Select all connections",
    ))]);
    bar.append(&select_all_button);

    let clear_button = Button::from_icon_name("edit-clear-symbolic");
    clear_button.add_css_class("pill");
    clear_button.add_css_class("bulk-action");
    clear_button.set_tooltip_text(Some(&i18n("Clear Selection")));
    clear_button.set_action_name(Some("win.clear-selection"));
    clear_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Clear selection"))]);
    bar.append(&clear_button);

    let delete_button = Button::from_icon_name("user-trash-symbolic");
    delete_button.add_css_class("pill");
    delete_button.add_css_class("bulk-action");
    delete_button.add_css_class("bulk-action-destructive");
    delete_button.set_tooltip_text(Some(&i18n("Delete Selected")));
    delete_button.set_action_name(Some("win.delete-selected"));
    delete_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Delete selected connections",
    ))]);
    bar.append(&delete_button);

    bar
}

/// Creates the sidebar bottom toolbar with secondary actions
///
/// Layout: [Group Ops] [History] [A-Z Sort] [Recent] [KeePass] [Smart Folders]
#[must_use]
pub fn create_sidebar_bottom_toolbar() -> (GtkBox, Button) {
    let toolbar = GtkBox::new(Orientation::Horizontal, 4);
    toolbar.set_margin_start(12);
    toolbar.set_margin_end(12);
    toolbar.set_halign(gtk4::Align::Center);

    let group_ops_button = Button::from_icon_name("view-list-symbolic");
    group_ops_button.add_css_class("flat");
    group_ops_button.set_tooltip_text(Some(&i18n("Group operations mode")));
    group_ops_button.set_action_name(Some("win.group-operations"));
    group_ops_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Enable group operations mode for multi-select",
    ))]);
    toolbar.append(&group_ops_button);

    let history_button = Button::from_icon_name("document-open-recent-symbolic");
    history_button.add_css_class("flat");
    history_button.set_tooltip_text(Some(&i18n("Connection History")));
    history_button.set_action_name(Some("win.show-history"));
    history_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "View connection history",
    ))]);
    toolbar.append(&history_button);

    let sort_button = Button::from_icon_name("view-sort-ascending-symbolic");
    sort_button.add_css_class("flat");
    sort_button.set_tooltip_text(Some(&i18n("Sort alphabetically")));
    sort_button.set_action_name(Some("win.sort-connections"));
    sort_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Sort connections alphabetically",
    ))]);
    toolbar.append(&sort_button);

    let sort_recent_button = Button::from_icon_name("view-sort-descending-symbolic");
    sort_recent_button.add_css_class("flat");
    sort_recent_button.set_tooltip_text(Some(&i18n("Sort by recent usage")));
    sort_recent_button.set_action_name(Some("win.sort-recent"));
    sort_recent_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Sort connections by recent usage",
    ))]);
    toolbar.append(&sort_recent_button);

    let keepass_button = Button::from_icon_name("dialog-password-symbolic");
    keepass_button.add_css_class("flat");
    keepass_button.set_tooltip_text(Some(&i18n("Open password vault")));
    keepass_button.set_action_name(Some("win.open-keepass"));
    keepass_button.add_css_class("keepass-button");
    keepass_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Open password vault for credential management",
    ))]);
    toolbar.append(&keepass_button);

    let smart_folders_button = Button::from_icon_name("folder-templates-symbolic");
    smart_folders_button.add_css_class("flat");
    smart_folders_button.set_tooltip_text(Some(&i18n("Toggle smart folders")));
    smart_folders_button.set_action_name(Some("win.toggle-smart-folders"));
    smart_folders_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Show or hide smart folders panel",
    ))]);
    toolbar.append(&smart_folders_button);

    (toolbar, keepass_button)
}
