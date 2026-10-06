//! UI helper functions for connection sidebar
//!
//! This module contains UI-related helper functions for creating popovers,
//! context menus, and other visual elements used by the sidebar widget.

use std::cell::RefCell;

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Label, Orientation, Separator, gdk, gio, glib};
use libadwaita as adw;

use crate::i18n::i18n;

thread_local! {
    /// Tracks the currently open context menu popover across the entire application.
    /// When a new context menu is requested (sidebar or split view), the previous
    /// one is closed first to prevent GTK4 popover lifecycle conflicts (issue #87).
    static ACTIVE_POPOVER: RefCell<Option<gtk4::Popover>> = const { RefCell::new(None) };
    /// True while one of our own handlers is popping a context menu down.
    /// The `closed` handler reads it (the emission is synchronous) to tell
    /// an intentional close apart from a compositor dismissal (#157).
    static INTENTIONAL_POPDOWN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Set once this session has seen a non-grabbing context menu cancelled by
    /// the compositor.
    ///
    /// The display-server check in [`pointer_row_takes_grab`] covers the known
    /// cases, but "which compositors cancel a grab-less xdg_popup" is not a
    /// question a client can ask. So the answer is also *learned*: one cancelled
    /// menu switches every later one to a grab for the rest of the session,
    /// which is how an environment nobody anticipated still ends up with a
    /// working context menu after the first attempt (#299).
    static NONGRABBING_POPUP_CANCELLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Compositor-dismissal window (#157, #299): a Wayland compositor cancels a
/// non-grabbing xdg_popup (autohide=false) on the focus change that follows the
/// click — KWin does, and mutter 50 does too (7–27 ms, reported on Fedora 44 /
/// GTK 4.22). A context-menu popover that closes within this window with no
/// user interaction was therefore not closed by the user.
const EARLY_DISMISS_WINDOW: std::time::Duration = std::time::Duration::from_millis(300);

/// Whether a per-row right-click menu should take the input grab.
///
/// `false` is the nicer behaviour and the reason [`MenuActivation::PointerRow`]
/// exists: without a grab, a right-click on a *different* row reaches that row's
/// gesture directly, so switching the menu between rows costs one click instead
/// of two (#87). It is only available where a grab-less popup survives.
///
/// On Wayland it does not. The compositor cancels the popup milliseconds after
/// it maps, and the deferred `autohide=true` retry cannot rescue it because a
/// grab must be requested against the serial of a real input event and the retry
/// runs from an idle callback (#157). Every Wayland session therefore grabs from
/// the start: two right-clicks to switch rows is a smaller price than a menu
/// that never appears (#299). X11 has no such cancellation and keeps the
/// one-click behaviour.
fn pointer_row_takes_grab() -> bool {
    if NONGRABBING_POPUP_CANCELLED.with(std::cell::Cell::get) {
        return true;
    }
    // `Unknown` (no display server identified) is treated as X11: the grab-less
    // path is the one with the better behaviour, and a cancellation flips the
    // sticky flag above on the first attempt anyway.
    crate::display::DisplayServer::detect().is_wayland()
}

/// Resolves a popover anchor that survives a `ListView` re-layout, with the
/// click point translated into the anchor's coordinate space.
///
/// A sidebar row is a virtualized widget: `ListView` recycles, re-realizes and
/// re-allocates it as the list changes. A `GtkPopover` is a native surface tied
/// to its parent, so any of that while the menu is parented to the row unmaps
/// the popup — and the menu disappears roughly one frame after it opened, which
/// is the 7–27 ms measured in issue #299 (and why forcing a different `GSK`
/// renderer changed how often it happened, without fixing it: it changed the
/// frame timing, not the anchor).
///
/// The enclosing `ScrolledWindow` is not recycled, so it becomes the parent.
/// `ancestor()` includes the widget itself, so the empty-space menu — already
/// invoked on the `ScrolledWindow` — resolves to itself with the coordinates
/// unchanged. Anything outside a `ScrolledWindow` keeps its original anchor.
fn stable_anchor(widget: &impl IsA<gtk4::Widget>, x: f64, y: f64) -> (gtk4::Widget, f64, f64) {
    let widget: &gtk4::Widget = widget.upcast_ref();
    let Some(scrolled) = widget.ancestor(gtk4::ScrolledWindow::static_type()) else {
        return (widget.clone(), x, y);
    };
    let point = gtk4::graphene::Point::new(x as f32, y as f32);
    widget.compute_point(&scrolled, &point).map_or_else(
        || (widget.clone(), x, y),
        |p| (scrolled.clone(), f64::from(p.x()), f64::from(p.y())),
    )
}

/// Largest height a context menu is allowed to request, in logical pixels.
///
/// A `GtkPopover` can never be smaller than its child, and a connection menu is
/// up to twenty rows and six separators tall — around 700 px. With no cap that
/// is the popover's *minimum* height, so on a short display, or with the window
/// low enough that neither anchoring below nor flipping above the pointer leaves
/// that much room, the popup did not map at all: no menu, no error, nothing in
/// the log. Moving the window up made the same right-click work, which is what
/// identified it (issue [#298](https://github.com/totoshko88/RustConn/issues/298)).
///
/// Two thirds of the monitor keeps room for the flip in either direction and
/// still shows most of the menu at once; the remainder scrolls. Falls back to
/// the floor when the monitor cannot be resolved — before the window's surface
/// is realized, or on a backend that reports no monitor for it.
fn menu_max_height(window: &gtk4::ApplicationWindow) -> i32 {
    /// Floor for the cap, so the menu stays usable on a very short display and
    /// is still a sane value when the monitor is unknown.
    const MIN_MAX_HEIGHT: i32 = 240;

    window
        .surface()
        .and_then(|surface| surface.display().monitor_at_surface(&surface))
        .map_or(MIN_MAX_HEIGHT, |monitor| {
            (monitor.geometry().height() * 2 / 3).max(MIN_MAX_HEIGHT)
        })
}

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
/// - `PointerRow`: per-row right-click gesture. Prefers `autohide=false` so a
///   right-click on a *different* row reaches that row's gesture directly
///   (#87), but takes the grab where a grab-less popup does not survive —
///   see [`pointer_row_takes_grab`] (#157, #299).
/// - `PointerFallback`: ListView-level right-click / touch long-press
///   fallback used when per-row dispatch fails (deep nesting, #157). Pops
///   up with `autohide=true` immediately: the grab is then tied to the
///   fresh input serial of the triggering press, which the compositor
///   honours — a deferred re-popup with a stale serial is dismissed again.
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
        match self {
            Self::PointerFallback | Self::Keyboard => true,
            Self::PointerRow => pointer_row_takes_grab(),
        }
    }
}

/// A single item in the context menu.
pub enum ContextMenuItem {
    /// A clickable action.
    ///
    /// `steps` are window actions (without the `win.` prefix) activated in
    /// order, each with an optional target value. A single step covers the
    /// common case; two express "select this row, then act on the selection",
    /// which is how a menu built for a connection identified by id reaches the
    /// actions that operate on the sidebar's current selection.
    Action {
        label: String,
        steps: Vec<(String, Option<glib::Variant>)>,
        destructive: bool,
    },
    /// A visual separator between groups of actions.
    Separator,
    /// An item that slides to a page of its own items, like `GtkPopoverMenu`.
    ///
    /// The page lives inside the same popover rather than in a second one: the
    /// focus-loss handler in [`show_popover`] closes the menu as soon as focus
    /// leaves the popover, which a sibling popover would do on opening.
    Submenu { label: String, items: Vec<Self> },
}

/// CSS class marking a button that opens a submenu page.
const SUBMENU_CLASS: &str = "context-menu-submenu";
/// CSS class marking a submenu page's back button.
const BACK_CLASS: &str = "context-menu-back";
/// Stack page name of the top-level menu page.
const MAIN_PAGE: &str = "main";

/// Builds the "Copy" items for a connection, given its id.
type CopyItemsProvider = Box<dyn Fn(&str) -> Vec<ContextMenuItem>>;

thread_local! {
    /// Supplies the "Copy" submenu of a connection. Set once by the main window,
    /// which owns the application state the menu needs; the sidebar only knows
    /// the connection id.
    static COPY_ITEMS_PROVIDER: RefCell<Option<CopyItemsProvider>> =
        const { RefCell::new(None) };
}

/// Installs the function that builds a connection's "Copy" submenu items.
pub fn set_copy_items_provider(provider: impl Fn(&str) -> Vec<ContextMenuItem> + 'static) {
    COPY_ITEMS_PROVIDER.with(|cell| *cell.borrow_mut() = Some(Box::new(provider)));
}

/// The "Copy ▸" submenu for the connection `conn_id`, or `None` when it has
/// nothing to copy (or no provider is installed).
#[must_use]
pub fn copy_submenu(conn_id: &str) -> Option<ContextMenuItem> {
    let items = COPY_ITEMS_PROVIDER.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|provider| provider(conn_id))
            .unwrap_or_default()
    });
    (!items.is_empty()).then(|| ContextMenuItem::Submenu {
        label: i18n("Copy"),
        items,
    })
}

impl ContextMenuItem {
    /// A one-step action with no target value.
    pub fn action(label: &str, action: &str) -> Self {
        Self::Action {
            label: label.to_string(),
            steps: vec![(action.to_string(), None)],
            destructive: false,
        }
    }

    /// A one-step action carrying a target value.
    pub fn action_with_target(label: &str, action: &str, target: &glib::Variant) -> Self {
        Self::Action {
            label: label.to_string(),
            steps: vec![(action.to_string(), Some(target.clone()))],
            destructive: false,
        }
    }

    /// Marks the item as destructive, so it is styled apart from the rest.
    #[must_use]
    pub fn destructive(mut self) -> Self {
        if let Self::Action {
            ref mut destructive,
            ..
        } = self
        {
            *destructive = true;
        }
        self
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
    conn_id: &str,
    is_group: bool,
    is_ssh: bool,
    is_connected: bool,
    is_recording: bool,
    has_external_session: bool,
    is_pinned: bool,
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
                &i18n("Enable Cloud Sync…"),
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
        // External-viewer session (issue #209): Disconnect is the primary
        // action, placed at the very top per GNOME HIG (R5.1/5.6).
        if has_external_session {
            items.push(ContextMenuItem::action(
                &i18n("Disconnect"),
                "external-disconnect",
            ));
        }
        items.push(ContextMenuItem::action(&i18n("Connect"), "connect"));
        // Favorite toggle. The label names the resulting state so the user knows
        // what the click does; the action itself (`toggle-pin`) is unchanged.
        let favorite_label = if is_pinned {
            i18n("Remove from Favorites")
        } else {
            i18n("Add to Favorites")
        };
        items.push(ContextMenuItem::action(&favorite_label, "toggle-pin"));
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
            &i18n("Move to Group…"),
            "move-to-group",
        ));
        // Opening a second, independent session for a connection that already
        // has one (issue #302). `win.open-new-session` force-launches one,
        // bypassing the smart double-click that would otherwise just focus the
        // existing tab — the behaviour Ásbrú calls "Duplicate connection".
        //
        // It was offered only for external-viewer sessions (issue #209 / R7.5),
        // where a plain double-click shows an "already running" toast and
        // nothing else. That left an existing action undiscoverable for every
        // embedded session: the alternative was the global
        // "Open a new session on every double-click" preference, which changes
        // what *every* double-click does rather than making one extra session.
        //
        // Gated on there being a session to duplicate, so it does not sit next
        // to Connect doing the same thing on an idle connection. `is_connected`
        // only became trustworthy on a pinned row once the sidebar stopped
        // updating just the Favorites copy of it.
        if is_connected || has_external_session {
            items.push(ContextMenuItem::action(
                &i18n("Open new session"),
                "open-new-session",
            ));
        }
        // § Utilities (copy, tools, network)
        items.push(ContextMenuItem::Separator);
        // Only the fields this connection actually has (issue #357).
        if let Some(copy) = copy_submenu(conn_id) {
            items.push(copy);
        }
        items.push(ContextMenuItem::action(
            &i18n("Run Snippet…"),
            "run-snippet-for-connection",
        ));
        // Opens the log viewer where this connection writes its session logs.
        // Always offered: the point is to show the location even when nothing
        // has been recorded yet (issue #247).
        items.push(ContextMenuItem::action(
            &i18n("Session Log…"),
            "show-connection-log",
        ));
        if is_ssh {
            items.push(ContextMenuItem::action(&i18n("Open SFTP"), "open-sftp"));
            items.push(ContextMenuItem::action(
                &i18n("Open Browser via Tunnel"),
                "open-browser-via-tunnel",
            ));
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

    // External-viewer session (issue #209): Stop tracking deregisters the
    // session without terminating the viewer (R5.4). Placed near the bottom,
    // just above the destructive Delete item, per GNOME HIG (R5.6).
    if has_external_session && !is_group {
        items.push(ContextMenuItem::Separator);
        items.push(ContextMenuItem::action(
            &i18n("Stop tracking"),
            "external-stop-tracking",
        ));
    }

    // Delete section (always last, visually separated)
    // Import groups: hide "Delete" (group lifecycle managed by sync)
    let is_import_group = is_group && sync_mode == "import";
    if !is_import_group {
        items.push(ContextMenuItem::Separator);
        items.push(ContextMenuItem::action(&i18n("Delete"), "delete-connection").destructive());
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
        ContextMenuItem::action(&i18n("Import…"), "import"),
        ContextMenuItem::action(&i18n("Export…"), "export"),
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
pub fn show_popover(
    widget: &impl IsA<gtk4::Widget>,
    window: &gtk4::ApplicationWindow,
    items: &[ContextMenuItem],
    x: f64,
    y: f64,
    activation: MenuActivation,
) {
    close_active_popover();

    // Anchor to something the ListView cannot pull out from under the menu,
    // translating the click point into that widget's coordinates (#299).
    let (anchor, x, y) = stable_anchor(widget, x, y);

    let popover = gtk4::Popover::new();
    popover.set_parent(&anchor);
    // Pin the popover's own background/foreground to the libadwaita popover
    // palette. Third-party GTK themes (e.g. Breeze on KDE) otherwise colour
    // the flat-button text to clash with the popover background, rendering the
    // menu rows invisible (#181). Styled in assets/style.css.
    popover.add_css_class("context-menu-popover");

    // One stack page per menu level, sliding like `GtkPopoverMenu`. A menu
    // with no submenu is a single page, so it looks exactly as before.
    let stack = gtk4::Stack::builder()
        .transition_type(gtk4::StackTransitionType::SlideLeftRight)
        .hhomogeneous(false)
        .vhomogeneous(false)
        .interpolate_size(true)
        .build();
    let main_page = build_menu_page(items, window, &popover, &stack, MAIN_PAGE, None);
    stack.add_named(&main_page, Some(MAIN_PAGE));
    stack.set_visible_child_name(MAIN_PAGE);

    // Cap the menu's height so the popover always has somewhere to go — see
    // [`menu_max_height`] for why an uncapped menu simply failed to open (#298).
    // `propagate_natural_*` keeps a short menu exactly as tall and wide as its
    // items, so nothing changes for the common case; a long one scrolls.
    // Arrow-key navigation still works: a `ScrolledWindow` scrolls to whichever
    // child takes focus.
    let scroller = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .propagate_natural_height(true)
        .propagate_natural_width(true)
        .max_content_height(menu_max_height(window))
        .child(&stack)
        .build();
    popover.set_child(Some(&scroller));

    #[expect(
        clippy::cast_possible_truncation,
        reason = "value range fits the target type by construction in this code path"
    )]
    let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
    popover.set_pointing_to(Some(&rect));
    // Resolved once and reused by the `closed` handler below: the per-row
    // decision can flip mid-session (see [`pointer_row_takes_grab`]) and the
    // handler must reason about the popup it actually created, not about what a
    // fresh one would do now.
    let takes_grab = activation.takes_grab();

    // Without a grab, a right-click on a *different* sidebar row reaches that
    // row's GestureClick directly, so switching the menu between rows takes one
    // click rather than two (issue #87). Dismissal is then handled manually:
    // - Left-click dismiss gesture on ScrolledWindow (CAPTURE phase)
    // - close_active_popover() called before every new context menu
    // - Each button closes the popover before activating its action
    // - Escape key handler below
    //
    // Wayland does not allow that: the compositor cancels a non-grabbing
    // xdg_popup on the focus change that follows the click, and the deferred
    // re-popup cannot acquire a grab because its input serial is stale
    // (#157, #299). There the grab is taken from the start — as it always was
    // for PointerFallback and Keyboard.
    popover.set_autohide(takes_grab);
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
    // on popup, so the controller receives key events immediately. Right
    // opens a submenu and Left goes back, as in `GtkPopoverMenu`.
    let stack_for_nav = stack.downgrade();
    let nav_controller = gtk4::EventControllerKey::new();
    nav_controller.connect_key_pressed(move |_, key, _, _| {
        let Some(menu_box) = stack_for_nav
            .upgrade()
            .and_then(|s| s.visible_child())
            .and_downcast::<GtkBox>()
        else {
            return gtk4::glib::Propagation::Proceed;
        };
        let items = menu_item_buttons(&menu_box);
        if items.is_empty() {
            return gtk4::glib::Propagation::Proceed;
        }
        let focused = items.iter().position(|b| b.has_focus());
        match key {
            gdk::Key::Right => {
                if let Some(button) = focused.map(|i| &items[i])
                    && button.has_css_class(SUBMENU_CLASS)
                {
                    button.emit_clicked();
                    return gtk4::glib::Propagation::Stop;
                }
                return gtk4::glib::Propagation::Proceed;
            }
            gdk::Key::Left | gdk::Key::BackSpace => {
                if let Some(back) = items.iter().find(|b| b.has_css_class(BACK_CLASS)) {
                    back.emit_clicked();
                    return gtk4::glib::Propagation::Stop;
                }
                return gtk4::glib::Propagation::Proceed;
            }
            _ => {}
        }
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
        // Only a non-grabbing popup can be cancelled this way: a grabbing popup
        // dismissed by the compositor cannot be saved by re-popping either (the
        // input serial is already stale, #157).
        if !takes_grab
            && popup_at.elapsed() < EARLY_DISMISS_WINDOW
            && !retried.get()
            && !intentional
        {
            retried.set(true);
            // Remember it for the session. The deferred re-popup below is a long
            // shot — it is the same stale-serial grab that #157 established
            // cannot be acquired — so what actually fixes the menu is that every
            // later one starts with a grab (#299).
            NONGRABBING_POPUP_CANCELLED.with(|flag| flag.set(true));
            tracing::debug!(
                "Context menu dismissed {}ms after popup — retrying with autohide=true; \
                 later menus will take the grab immediately",
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

        // A close this soon after popup was not the user's doing. The retry
        // above only covers the grab-less case, so without this line a grabbing
        // menu that the compositor refused went away with nothing in the log —
        // which is how both #299 and #298 arrived as "the menu just does not
        // open", with no way to tell a refused popup from a handler that never
        // ran.
        if !intentional && popup_at.elapsed() < EARLY_DISMISS_WINDOW {
            tracing::debug!(
                elapsed_ms = popup_at.elapsed().as_millis(),
                takes_grab,
                "Context menu closed immediately after popup without user interaction"
            );
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
        let main_for_focus = main_page.downgrade();
        gtk4::glib::idle_add_local_once(move || {
            if let Some(menu_box) = main_for_focus.upgrade()
                && let Some(first) = menu_item_buttons(&menu_box).first()
            {
                first.grab_focus();
            }
        });
    }
}

/// Builds one page of a context menu: a box of item buttons.
///
/// A [`ContextMenuItem::Submenu`] adds a page of its own to `stack`, named
/// after its position under `page_name`, and a button here that slides to it.
/// `back_to` is the parent page name and the submenu title for a submenu page,
/// which then starts with a back button.
fn build_menu_page(
    items: &[ContextMenuItem],
    window: &gtk4::ApplicationWindow,
    popover: &gtk4::Popover,
    stack: &gtk4::Stack,
    page_name: &str,
    back_to: Option<(&str, &str)>,
) -> GtkBox {
    // `accessible-role` is construct-only — use the builder so screen
    // readers announce the container as a menu.
    let vbox = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(0)
        .accessible_role(gtk4::AccessibleRole::Menu)
        .build();
    vbox.add_css_class("context-menu");

    if let Some((parent, title)) = back_to {
        let back = menu_button_with_icon(title, "go-previous-symbolic", true);
        back.add_css_class(BACK_CLASS);
        back.update_property(&[gtk4::accessible::Property::Label(&i18n("Back"))]);
        let stack_weak = stack.downgrade();
        let parent = parent.to_string();
        let opener = page_name.to_string();
        back.connect_clicked(move |_| {
            if let Some(stack) = stack_weak.upgrade() {
                show_menu_page(&stack, &parent, Some(&opener));
            }
        });
        vbox.append(&back);
        vbox.append(&Separator::new(Orientation::Horizontal));
    }

    for (index, item) in items.iter().enumerate() {
        match item {
            ContextMenuItem::Action {
                label,
                steps,
                destructive,
            } => {
                let button = Button::builder()
                    .accessible_role(gtk4::AccessibleRole::MenuItem)
                    .build();
                button.add_css_class("flat");
                button.add_css_class("context-menu-item");
                if *destructive {
                    button.add_css_class("context-menu-destructive");
                }

                let lbl = Label::new(Some(label));
                lbl.set_xalign(0.0);
                button.set_child(Some(&lbl));

                let window_weak = window.downgrade();
                let steps = steps.clone();
                let popover_weak = popover.downgrade();
                button.connect_clicked(move |_| {
                    if let Some(p) = popover_weak.upgrade() {
                        popdown_intentionally(&p);
                    }
                    if let Some(w) = window_weak.upgrade() {
                        for (action_name, target) in &steps {
                            gtk4::prelude::ActionGroupExt::activate_action(
                                &w,
                                action_name,
                                target.as_ref(),
                            );
                        }
                    }
                });

                vbox.append(&button);
            }
            ContextMenuItem::Separator => {
                vbox.append(&Separator::new(Orientation::Horizontal));
            }
            ContextMenuItem::Submenu {
                label,
                items: sub_items,
            } => {
                let sub_name = format!("{page_name}/{index}");
                let sub_page = build_menu_page(
                    sub_items,
                    window,
                    popover,
                    stack,
                    &sub_name,
                    Some((page_name, label)),
                );
                stack.add_named(&sub_page, Some(&sub_name));

                let button = menu_button_with_icon(label, "go-next-symbolic", false);
                button.add_css_class(SUBMENU_CLASS);
                // Lets the back button return focus to the item that opened
                // the page, as `GtkPopoverMenu` does.
                button.set_widget_name(&sub_name);
                button.update_property(&[gtk4::accessible::Property::HasPopup(true)]);
                let stack_weak = stack.downgrade();
                button.connect_clicked(move |_| {
                    if let Some(stack) = stack_weak.upgrade() {
                        show_menu_page(&stack, &sub_name, None);
                    }
                });
                vbox.append(&button);
            }
        }
    }
    vbox
}

/// A menu-item button holding a label and an icon: the icon leads for a back
/// button (`icon_first`) and trails for a submenu opener.
fn menu_button_with_icon(label: &str, icon: &str, icon_first: bool) -> Button {
    let button = Button::builder()
        .accessible_role(gtk4::AccessibleRole::MenuItem)
        .build();
    button.add_css_class("flat");
    button.add_css_class("context-menu-item");
    let row = GtkBox::new(Orientation::Horizontal, 6);
    let image = gtk4::Image::from_icon_name(icon);
    let lbl = Label::new(Some(label));
    lbl.set_xalign(0.0);
    lbl.set_hexpand(true);
    if icon_first {
        lbl.add_css_class("heading");
        row.append(&image);
        row.append(&lbl);
    } else {
        row.append(&lbl);
        row.append(&image);
    }
    button.set_child(Some(&row));
    button
}

/// Slides `stack` to the page `name` and moves focus into it.
///
/// Focus must move with the page: the popover closes itself once focus leaves
/// it, and a focused button on a page that has slid away no longer counts as
/// inside. `focus_opener` names the submenu button to focus when going back;
/// otherwise the first item that is not a back button takes focus.
fn show_menu_page(stack: &gtk4::Stack, name: &str, focus_opener: Option<&str>) {
    stack.set_visible_child_name(name);
    let Some(page) = stack.child_by_name(name).and_downcast::<GtkBox>() else {
        return;
    };
    let buttons = menu_item_buttons(&page);
    let target = focus_opener
        .and_then(|opener| buttons.iter().find(|b| b.widget_name() == opener))
        .or_else(|| buttons.iter().find(|b| !b.has_css_class(BACK_CLASS)))
        .or_else(|| buttons.first());
    if let Some(button) = target {
        button.grab_focus();
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
    // 6px inter-icon gap matches AdwHeaderBar's built-in child spacing, and the
    // buttons use Adwaita's standard flat-icon metrics (no CSS size override, see
    // style.css), so the bottom row reads identically to the header (GNOME HIG).
    let toolbar = GtkBox::new(Orientation::Horizontal, 6);
    toolbar.set_margin_start(6);
    toolbar.set_margin_end(6);
    toolbar.set_halign(gtk4::Align::Center);

    let history_button = Button::from_icon_name("document-open-recent-symbolic");
    history_button.add_css_class("flat");
    history_button.set_tooltip_text(Some(&i18n("Connection History")));
    history_button.set_action_name(Some("win.show-history"));
    history_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "View connection history",
    ))]);
    toolbar.append(&history_button);

    // Sort: two standalone buttons (Alphabetical / Recent Usage) rather than a
    // single menu button. Five direct icons read more clearly than four icons
    // plus a dropdown, and sorting is a one-click action either way. Actions
    // unchanged (`win.sort-connections` / `win.sort-recent`).
    let sort_az_button = Button::from_icon_name("view-sort-ascending-symbolic");
    sort_az_button.add_css_class("flat");
    sort_az_button.set_tooltip_text(Some(&i18n("Sort alphabetically")));
    sort_az_button.set_action_name(Some("win.sort-connections"));
    sort_az_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Sort connections alphabetically",
    ))]);
    toolbar.append(&sort_az_button);

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

/// Creates the sidebar's own top `AdwHeaderBar`.
///
/// Giving the sidebar a headerbar of its own is what makes the
/// `AdwOverlaySplitView` read as two distinct panels (like GNOME Files /
/// Settings) instead of one column hanging under a single global headerbar —
/// the visual separation users expect, and the fix for the "sidebar looks
/// resizable" confusion. The buttons reference existing `win.*` actions, so no
/// new wiring is needed; they fire the same handlers as the content header.
pub fn create_sidebar_header() -> (adw::HeaderBar, gtk4::ToggleButton) {
    // Window controls are left to libadwaita: inside an AdwOverlaySplitView an
    // AdwHeaderBar hides the title buttons that are not at a window edge, and
    // shows them again when the sidebar is hidden or collapsed. Forcing one
    // side off here is what lost the controls for start-side layouts.
    let header = adw::HeaderBar::new();
    // The split view already carries the window title on the content side; a
    // short static title here just labels the panel. Tagged `sidebar-panel-title`
    // so compact mode can collapse it by an exact class match rather than
    // guessing the internal AdwWindowTitle node structure.
    let sidebar_title = adw::WindowTitle::new(&i18n("Connections"), "");
    sidebar_title.add_css_class("sidebar-panel-title");
    header.set_title_widget(Some(&sidebar_title));

    // Search toggle (leading) — Nautilus/Settings pattern: the search row is
    // hidden behind an icon in the header and revealed on demand (click,
    // Ctrl+F, or type-to-search). Returned so the sidebar can bind it to the
    // GtkSearchBar's search-mode-enabled property.
    let search_toggle = gtk4::ToggleButton::new();
    search_toggle.set_icon_name("system-search-symbolic");
    search_toggle.set_tooltip_text(Some(&i18n("Search (Ctrl+F)")));
    search_toggle.update_property(&[gtk4::accessible::Property::Label(&i18n("Toggle search"))]);
    search_toggle.set_size_request(HEADER_BUTTON_SIZE, HEADER_BUTTON_SIZE);
    header.pack_start(&search_toggle);

    // Secondary menu (trailing). This is the sidebar's own home for every
    // action that operates on the connection list — the fast, logical place
    // users reach for, right beside the list it acts on, instead of crossing to
    // the content header's primary ☰ and digging through a Tools submenu. The
    // 0.23 redesign moved these out on a one-menu-per-window reading of the HIG;
    // in practice that made connection management two clicks farther away, so
    // Create / Tools / List live here again. The content header's ☰ keeps the
    // same entries as the app-wide menu — this is a deliberate, convenient
    // duplication of the list-scoped subset, not a split. Icon stays
    // `view-more` (⋯): a secondary, panel-scoped menu, not a second hamburger.
    let menu = gio::Menu::new();

    // Create section.
    let create_section = gio::Menu::new();
    create_section.append(Some(&i18n("New Connection")), Some("win.new-connection"));
    create_section.append(
        Some(&i18n("New Connection (Advanced)…")),
        Some("win.new-connection-advanced"),
    );
    create_section.append(Some(&i18n("New Group")), Some("win.new-group"));
    menu.append_section(None, &create_section);

    // Tools section — quick access to cluster / workspace / snippet management.
    let tools_section = gio::Menu::new();
    tools_section.append(Some(&i18n("Manage Clusters")), Some("win.manage-clusters"));
    tools_section.append(
        Some(&i18n("Manage Workspaces")),
        Some("win.manage-workspaces"),
    );
    tools_section.append(Some(&i18n("Manage Snippets")), Some("win.manage-snippets"));
    menu.append_section(Some(&i18n("Tools")), &tools_section);

    // List section — act on the connection list. Delete is intentionally NOT
    // here: deleting via a menu (open → aim → click) is a worse path than the
    // per-row context menu / Delete key, so destructive deletion lives on the
    // row's right-click menu.
    let list_section = gio::Menu::new();
    list_section.append(Some(&i18n("Quick Connect")), Some("win.quick-connect"));
    list_section.append(Some(&i18n("Import…")), Some("win.import"));
    list_section.append(Some(&i18n("Export…")), Some("win.export"));
    menu.append_section(None, &list_section);

    // Select + sort — multi-select mode and the two orderings.
    let select_section = gio::Menu::new();
    // Stateful `win.group-operations`: renders as a checkable item.
    select_section.append(
        Some(&i18n("Select Connections")),
        Some("win.group-operations"),
    );
    menu.append_section(None, &select_section);
    let sort_section = gio::Menu::new();
    sort_section.append(
        Some(&i18n("Sort Alphabetically")),
        Some("win.sort-connections"),
    );
    sort_section.append(Some(&i18n("Sort by Recent Use")), Some("win.sort-recent"));
    menu.append_section(None, &sort_section);

    let menu_label = i18n("Connection list menu");
    let menu_button = gtk4::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text(menu_label.as_str())
        .menu_model(&menu)
        .build();
    menu_button.update_property(&[gtk4::accessible::Property::Label(&menu_label)]);
    menu_button.set_size_request(HEADER_BUTTON_SIZE, HEADER_BUTTON_SIZE);
    header.pack_end(&menu_button);

    (header, search_toggle)
}

/// Minimum size of an icon-only header-bar button, shared by the sidebar and
/// content headers so the two bars come out the same height.
///
/// 44px is the GNOME HIG tap target. macOS has no touch input and its native
/// toolbar buttons are ~28px; `AdwHeaderBar` takes its minimum height from its
/// tallest child, so 44 there would keep the header taller than any native
/// window and defeat the compact/macOS CSS.
pub const HEADER_BUTTON_SIZE: i32 = if cfg!(target_os = "macos") { 28 } else { 44 };
