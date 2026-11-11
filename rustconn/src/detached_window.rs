//! Top-level windows that host a single detached session.
//!
//! A detached window is a pure **view host**: it borrows the widget subtree of a
//! session that [`crate::terminal::TerminalNotebook`] still owns. It stores no
//! session state beyond the two ids it needs for its own callbacks, so every
//! per-session map keeps working while the session lives outside the main
//! window.
//!
//! Replaces the never-constructed `ExternalWindow` / `ExternalWindowManager`
//! pair: that one reparented a bare `vte4::Terminal`, had no toast overlay, no
//! per-window accelerator wiring, and no way back into a tab.
//!
//! Callbacks installed here are plain boxed closures. The window layer wires
//! them with `Weak` handles to the notebook and the registry, so nothing in this
//! module can form an `Rc` cycle that outlives a session.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, DirectionType, MenuButton, gdk, gio, glib};
use libadwaita as adw;
use libadwaita::prelude::*;
use uuid::Uuid;

use crate::i18n::{i18n, i18n_f};
use crate::terminal::{DetachMonitor, DetachPresentation};

/// Default window width, matching a comfortable 80-column terminal plus chrome.
const DEFAULT_WIDTH: i32 = 900;
/// Default window height; the default size stays at least 800x600.
const DEFAULT_HEIGHT: i32 = 650;
/// Smallest width a detached window may be shrunk to before the session content
/// stops being usable.
const MIN_WIDTH: i32 = 400;
/// Smallest usable height for a detached window.
const MIN_HEIGHT: i32 = 300;

/// Callback slot invoked with the window's own session id.
type SessionCallback = Rc<RefCell<Option<Box<dyn Fn(Uuid)>>>>;

/// Identity and labelling of the session a detached window is built for.
pub struct DetachedWindowParams<'a> {
    /// Session whose widget the window hosts.
    pub session_id: Uuid,
    /// Connection the session belongs to.
    pub connection_id: Uuid,
    /// Connection name, shown in the window and header bar title.
    pub title: &'a str,
    /// Human-readable protocol name, shown as the header bar subtitle.
    pub protocol: &'a str,
}

/// Builds the window title for a detached session (pure, testable).
///
/// Uses an em dash, matching the GNOME convention of "document — application".
/// A connection with no name falls back to its protocol, so the title never
/// starts with a stray dash.
fn detached_window_title(connection: &str, protocol: &str) -> String {
    i18n_f("{} — RustConn", &[display_name(connection, protocol)])
}

/// Picks what to call a session: its connection name, or its protocol when the
/// connection has no usable name.
fn display_name<'a>(connection: &'a str, protocol: &'a str) -> &'a str {
    if connection.trim().is_empty() {
        protocol
    } else {
        connection
    }
}

/// Decides whether the header bar should be shown for a given full-screen state.
///
/// Pure so it can be unit-tested headlessly; the actual visibility change needs
/// a display. Full screen hides the header bar entirely rather than going
/// through `ToolbarView::set_reveal_top_bars(false)`, because that call would
/// also hide the disconnect banner, which must stay visible even in full
/// screen (issue #236).
const fn header_visible(fullscreen: bool) -> bool {
    !fullscreen
}

/// A top-level window hosting exactly one detached session.
pub struct DetachedSessionWindow {
    window: adw::ApplicationWindow,
    session_id: Uuid,
    connection_id: Uuid,
    /// Protocol label, kept for the title fallback when a connection is renamed
    /// to an empty name.
    protocol: String,
    toast_overlay: adw::ToastOverlay,
    window_title: adw::WindowTitle,
    /// Persistent "this session dropped its connection" marker.
    ///
    /// A detached session has no tab to carry the offline indicator, and an
    /// embedded viewer gets no reconnect banner either (those protocols cannot
    /// reconnect in place), so the window itself says so (issue #236). A state
    /// that needs attention belongs in a banner, per the project's GNOME HIG
    /// rules.
    disconnect_banner: adw::Banner,
    /// Kept alive for its `clicked` handler — dropping it breaks the attach
    /// button (see the widget-lifecycle note in `main.rs`).
    attach_button: Button,
    /// The primary menu (F10). The window layer turns its `primary` flag off
    /// while keyboard passthrough is on, as the main window does with its own.
    menu_button: MenuButton,
    /// Set while the window is closing because its session moved back into a
    /// tab, so the close handler skips teardown. Same distinction
    /// `parked_in_split` makes for tabs.
    attaching: Rc<Cell<bool>>,
    presentation: Rc<RefCell<DetachPresentation>>,
    on_attach: SessionCallback,
    on_close: SessionCallback,
}

impl DetachedSessionWindow {
    /// Creates a window around a session's content box and wires its chrome.
    ///
    /// `content` is the box handed over by
    /// [`crate::terminal::TerminalNotebook::take_session_content`]; it is placed
    /// as-is so the session's monitoring bar and highlight overlay travel with
    /// it. The window is not presented yet — call [`Self::present`] or
    /// [`Self::present_fullscreen_on`].
    #[must_use]
    pub fn new(
        app: &adw::Application,
        params: &DetachedWindowParams<'_>,
        content: &GtkBox,
    ) -> Self {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title(detached_window_title(params.title, params.protocol))
            .default_width(DEFAULT_WIDTH)
            .default_height(DEFAULT_HEIGHT)
            .width_request(MIN_WIDTH)
            .height_request(MIN_HEIGHT)
            .build();

        #[cfg(target_os = "macos")]
        window.add_css_class("macos");

        let window_title =
            adw::WindowTitle::new(display_name(params.title, params.protocol), params.protocol);
        let attach_button = build_attach_button();
        let menu_button = build_menu_button();

        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&window_title));
        header.pack_start(&attach_button);
        header.pack_end(&menu_button);

        // Hidden until the session's connection drops; reuses the wording of the
        // tab reconnect banner so both placements say the same thing.
        let disconnect_banner = crate::toast::plain_banner(&i18n("Session disconnected"));
        disconnect_banner.set_revealed(false);

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.add_top_bar(&disconnect_banner);
        toolbar_view.set_content(Some(content));

        // Mirrors the main window's fullscreen chrome (`window/fullscreen_header.rs`):
        // react to the `fullscreened` property, not to the toggle action, so F11,
        // a window-manager fullscreen and `present_fullscreen_on` all hide and
        // restore the header bar identically. Applied once up front in case the
        // window is presented already fullscreen.
        //
        // ponytail: this is the minimal faithful piece (hide/restore only, no
        // top-edge hover reveal) rather than a shared helper — the main window's
        // `State` is hard-wired to its notebook, tab bar and split view. Upgrade
        // path: generalise `fullscreen_header::State` over an optional
        // notebook/split view if a detached window ever needs hover reveal too.
        header.set_visible(header_visible(window.is_fullscreen()));
        {
            let header = header.downgrade();
            let session_id = params.session_id;
            window.connect_fullscreened_notify(move |win| {
                if let Some(header) = header.upgrade() {
                    let fullscreen = win.is_fullscreen();
                    tracing::debug!(session = %session_id, fullscreen, "detached window fullscreen changed");
                    header.set_visible(header_visible(fullscreen));
                }
            });
        }

        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&toolbar_view));
        window.set_content(Some(&toast_overlay));

        // Per-window wiring the main window gets in `app.rs`: the non-Latin
        // layout accelerator fallback plus the compact chrome. `set_compact_prefs`
        // already iterates every application window, so later preference changes
        // reach this window without extra code.
        crate::app::install_layout_independent_accels(&window, app);
        crate::app::watch_window_for_compact(window.upcast_ref());
        crate::app::recompute_window_compact(window.upcast_ref());

        let detached = Self {
            window,
            session_id: params.session_id,
            connection_id: params.connection_id,
            protocol: params.protocol.to_owned(),
            toast_overlay,
            window_title,
            disconnect_banner,
            attach_button,
            menu_button,
            attaching: Rc::new(Cell::new(false)),
            presentation: Rc::new(RefCell::new(DetachPresentation::default())),
            on_attach: Rc::new(RefCell::new(None)),
            on_close: Rc::new(RefCell::new(None)),
        };
        detached.connect_attach_button();
        detached.connect_close_handler();
        detached.focus_content_on_idle(content);
        tracing::info!(
            session = %params.session_id,
            connection = %params.connection_id,
            protocol = params.protocol,
            "detached window created"
        );
        detached
    }

    /// Fires the attach callback when the header bar button is clicked.
    ///
    /// The button deliberately carries no `GAction`: the window layer owns the
    /// `win.attach-session` wiring and installs it as this callback, which keeps
    /// this module free of action-name coupling.
    fn connect_attach_button(&self) {
        let on_attach = Rc::clone(&self.on_attach);
        let session_id = self.session_id;
        self.attach_button.connect_clicked(move |_button| {
            if let Some(ref callback) = *on_attach.borrow() {
                callback(session_id);
            } else {
                tracing::warn!(session = %session_id, "attach requested with no handler installed");
            }
        });
    }

    /// Runs session teardown when the user closes the window.
    ///
    /// A close that follows [`Self::begin_attach`] means the session moved back
    /// into a tab, so it proceeds silently.
    fn connect_close_handler(&self) {
        let attaching = Rc::clone(&self.attaching);
        let on_close = Rc::clone(&self.on_close);
        let session_id = self.session_id;
        self.window.connect_close_request(move |_window| {
            if attaching.get() {
                tracing::debug!(session = %session_id, "detached window closing for attach");
                return glib::Propagation::Proceed;
            }
            tracing::info!(session = %session_id, "detached window closed by user");
            if let Some(ref callback) = *on_close.borrow() {
                callback(session_id);
            }
            glib::Propagation::Proceed
        });
    }

    /// Nudges a repaint and moves input focus into the session content.
    ///
    /// Runs on idle so GTK has settled the re-parented widget's allocation
    /// first: embedded viewers keep their frames in a Rust-side buffer, so
    /// nothing else triggers the first draw in the new window.
    fn focus_content_on_idle(&self, content: &GtkBox) {
        let content = content.clone();
        let session_id = self.session_id;
        glib::idle_add_local_once(move || {
            content.queue_draw();
            if !content.child_focus(DirectionType::TabForward) {
                tracing::debug!(
                    session = %session_id,
                    "detached window: session content took no focus"
                );
            }
        });
    }

    /// Shows the window normally and gives it keyboard focus.
    pub fn present(&self) {
        *self.presentation.borrow_mut() = DetachPresentation::default();
        self.window.present();
    }

    /// Shows the window fullscreen without a monitor preference.
    pub fn present_fullscreen(&self) {
        *self.presentation.borrow_mut() = DetachPresentation {
            fullscreen: true,
            monitor: None,
        };
        self.window.fullscreen();
        self.window.present();
    }

    /// Shows the window fullscreen while retaining an unavailable monitor preference.
    pub fn present_fullscreen_preferred(&self, preference: DetachMonitor) {
        *self.presentation.borrow_mut() = DetachPresentation {
            fullscreen: true,
            monitor: Some(preference),
        };
        self.window.fullscreen();
        self.window.present();
    }

    /// Shows the window fullscreen on a specific monitor.
    ///
    /// Wayland offers no window positioning by coordinates, so a monitor choice
    /// is expressed as fullscreen-on-monitor, which the compositor honors.
    pub fn present_fullscreen_on(&self, monitor: &gdk::Monitor, preference: DetachMonitor) {
        *self.presentation.borrow_mut() = DetachPresentation {
            fullscreen: true,
            monitor: Some(preference),
        };
        self.window.fullscreen_on_monitor(monitor);
        self.window.present();
    }

    /// Returns the current detached-window presentation.
    #[must_use]
    pub fn presentation(&self) -> DetachPresentation {
        self.presentation.borrow().clone()
    }

    /// Returns the shared presentation state for window-scoped actions.
    #[must_use]
    pub fn presentation_handle(&self) -> Rc<RefCell<DetachPresentation>> {
        Rc::clone(&self.presentation)
    }

    /// Updates the window and header bar title after a connection rename.
    pub fn set_session_title(&self, title: &str) {
        self.title_handles().apply(title);
    }

    /// Clones out everything needed to retitle this window.
    ///
    /// Lets [`DetachedWindowRegistry::rename_connection`] drop its map borrow
    /// before it touches GTK: `set_session_title` needs the window value, and
    /// reaching it through the registry would hold the borrow across two setters.
    #[must_use]
    pub fn title_handles(&self) -> TitleHandles {
        TitleHandles {
            window: self.window.clone(),
            window_title: self.window_title.clone(),
            protocol: self.protocol.clone(),
        }
    }

    /// Reveals or hides the "session disconnected" banner of this window.
    ///
    /// The notebook calls this through the registry when a detached session's
    /// connection drops or comes back, so a window whose session went offline is
    /// never silently frozen (issue #236).
    pub fn set_session_disconnected(&self, disconnected: bool) {
        self.disconnect_banner.set_revealed(disconnected);
    }

    /// Returns the disconnect banner, for callers that must not hold a registry
    /// borrow while they change it.
    #[must_use]
    pub const fn disconnect_banner(&self) -> &adw::Banner {
        &self.disconnect_banner
    }

    /// Returns the overlay that toasts for this session must be shown on.
    #[must_use]
    pub const fn toast_overlay(&self) -> &adw::ToastOverlay {
        &self.toast_overlay
    }

    /// Returns the primary menu button, whose `primary` flag (the F10 binding)
    /// follows keyboard passthrough.
    #[must_use]
    pub const fn menu_button(&self) -> &MenuButton {
        &self.menu_button
    }

    /// Returns the underlying window, for parenting dialogs to it.
    #[must_use]
    pub const fn window(&self) -> &adw::ApplicationWindow {
        &self.window
    }

    /// Returns the session hosted by this window.
    #[must_use]
    pub const fn session_id(&self) -> Uuid {
        self.session_id
    }

    /// Returns the connection the hosted session belongs to.
    #[must_use]
    pub const fn connection_id(&self) -> Uuid {
        self.connection_id
    }

    /// Marks the pending close as an attach, so no session teardown runs.
    ///
    /// Call immediately before [`Self::close`] once the session's content is
    /// back in a tab.
    pub fn begin_attach(&self) {
        self.attaching.set(true);
    }

    /// Closes the window.
    pub fn close(&self) {
        self.window.close();
    }

    /// Sets the callback invoked when the attach button is clicked.
    pub fn set_on_attach<F>(&self, callback: F)
    where
        F: Fn(Uuid) + 'static,
    {
        *self.on_attach.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the callback invoked when the user closes the window.
    ///
    /// The window layer runs the standard session teardown from it, so a
    /// detached-window close ends the session exactly as a tab close does.
    pub fn set_on_close<F>(&self, callback: F)
    where
        F: Fn(Uuid) + 'static,
    {
        *self.on_close.borrow_mut() = Some(Box::new(callback));
    }
}

/// The widgets a rename has to update, cloned out of a detached window.
///
/// GTK objects are reference-counted handles, so cloning them is cheap and the
/// clone drives the same widget — which is what lets a caller release its
/// registry borrow before it calls into GTK.
pub struct TitleHandles {
    window: adw::ApplicationWindow,
    window_title: adw::WindowTitle,
    protocol: String,
}

impl TitleHandles {
    /// Applies a new connection name to the window and its header bar.
    pub fn apply(&self, title: &str) {
        self.window
            .set_title(Some(&detached_window_title(title, &self.protocol)));
        self.window_title
            .set_title(display_name(title, &self.protocol));
    }
}

/// Builds the header bar button that moves the session back into a tab.
fn build_attach_button() -> Button {
    let button = Button::from_icon_name("view-restore-symbolic");
    button.add_css_class("flat");
    let label = i18n("Move to Main Window");
    button.set_tooltip_text(Some(&label));
    button.update_property(&[gtk4::accessible::Property::Label(&label)]);
    button
}

/// Builds the primary menu button, so F10 opens a menu here as it does in the
/// main window.
///
/// The menu holds the application-wide entries of the main menu: Settings
/// (`win.settings`, which this window forwards to the main window), Keyboard
/// Shortcuts, Help and About. Labels, tooltip and accessible name are the main
/// menu's own strings.
fn build_menu_button() -> MenuButton {
    let settings_section = gio::Menu::new();
    settings_section.append(Some(&i18n("Settings…")), Some("win.settings"));
    let app_section = gio::Menu::new();
    app_section.append(Some(&i18n("Keyboard Shortcuts…")), Some("app.shortcuts"));
    app_section.append(Some(&i18n("Help")), Some("app.help"));
    app_section.append(Some(&i18n("About RustConn")), Some("app.about"));
    let menu = gio::Menu::new();
    menu.append_section(None, &settings_section);
    menu.append_section(None, &app_section);

    let button = MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text(i18n("Menu (F10)"))
        .menu_model(&menu)
        .primary(true)
        .build();
    button.update_property(&[gtk4::accessible::Property::Label(&i18n("Menu"))]);
    button
}

/// The detached windows currently open, keyed by session.
///
/// Owned by the main window and shared as an `Rc`. It holds the only strong
/// reference to each [`DetachedSessionWindow`] value; dropping the entry and
/// closing the window releases it.
#[derive(Default)]
pub struct DetachedWindowRegistry {
    windows: RefCell<HashMap<Uuid, DetachedSessionWindow>>,
}

impl DetachedWindowRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a window under its own session id.
    pub fn insert(&self, window: DetachedSessionWindow) {
        self.windows.borrow_mut().insert(window.session_id, window);
    }

    /// Removes and returns the window of a session, if it has one.
    pub fn take(&self, session_id: Uuid) -> Option<DetachedSessionWindow> {
        self.windows.borrow_mut().remove(&session_id)
    }

    /// Presents the window of a session; reports whether one was found.
    ///
    /// This is what a focus request for a detached session resolves to, so
    /// sidebar activation and the session manager keep working unchanged.
    ///
    /// The window is cloned out and the borrow released before it is presented:
    /// `present` emits signals, and a handler reached from them that inserts
    /// into or takes from the registry would otherwise panic on the live borrow.
    pub fn present(&self, session_id: Uuid) -> bool {
        let window = self
            .windows
            .borrow()
            .get(&session_id)
            .map(|detached| detached.window().clone());
        match window {
            Some(window) => {
                window.present();
                true
            }
            None => false,
        }
    }

    /// Reports whether a session currently has a detached window.
    #[must_use]
    pub fn contains(&self, session_id: Uuid) -> bool {
        self.windows.borrow().contains_key(&session_id)
    }

    /// Snapshots a detached session's presentation state.
    #[must_use]
    pub fn presentation(&self, session_id: Uuid) -> Option<DetachPresentation> {
        self.windows
            .borrow()
            .get(&session_id)
            .map(DetachedSessionWindow::presentation)
    }

    /// Retitles every window of a connection after it was renamed.
    ///
    /// Returns how many windows were retitled. The title widgets are cloned out
    /// first and the map borrow released, so no borrow is held while GTK updates
    /// a window title — the same shape [`Self::present`] uses.
    pub fn rename_connection(&self, connection_id: Uuid, new_name: &str) -> usize {
        let affected: Vec<TitleHandles> = self
            .windows
            .borrow()
            .values()
            .filter(|window| window.connection_id() == connection_id)
            .map(DetachedSessionWindow::title_handles)
            .collect();
        for handles in &affected {
            handles.apply(new_name);
        }
        affected.len()
    }

    /// Marks (or unmarks) a session's window as disconnected.
    ///
    /// Reports whether a window was found. The banner is cloned out before it is
    /// revealed, so the map borrow is released before GTK runs any handler that
    /// could reach back into the registry.
    pub fn set_session_disconnected(&self, session_id: Uuid, disconnected: bool) -> bool {
        let banner = self
            .windows
            .borrow()
            .get(&session_id)
            .map(|window| window.disconnect_banner().clone());
        match banner {
            Some(banner) => {
                banner.set_revealed(disconnected);
                true
            }
            None => false,
        }
    }

    /// Closes every detached window, running the normal session teardown.
    ///
    /// Entries are removed before anything is closed, so a close handler that
    /// calls back into the registry cannot hit an active borrow.
    pub fn close_all(&self) {
        let windows: Vec<DetachedSessionWindow> =
            self.windows.borrow_mut().drain().map(|(_, w)| w).collect();
        tracing::info!(count = windows.len(), "closing all detached windows");
        for window in &windows {
            window.close();
        }
    }

    /// Runs a closure on the window of a session, if it has one.
    ///
    /// `DetachedSessionWindow` is not cloneable, so this is how callers reach a
    /// registered window.
    pub fn with_window<F, R>(&self, session_id: Uuid, f: F) -> Option<R>
    where
        F: FnOnce(&DetachedSessionWindow) -> R,
    {
        self.windows.borrow().get(&session_id).map(f)
    }
}

#[cfg(test)]
mod tests {
    use super::{detached_window_title, header_visible};

    #[test]
    fn window_title_names_the_connection_and_the_application() {
        assert_eq!(
            detached_window_title("prod-db", "SSH"),
            "prod-db — RustConn"
        );
    }

    #[test]
    fn window_title_falls_back_to_the_protocol_without_a_connection_name() {
        assert_eq!(detached_window_title("", "SSH"), "SSH — RustConn");
        assert_eq!(detached_window_title("   ", "VNC"), "VNC — RustConn");
    }

    #[test]
    fn header_is_visible_when_not_fullscreen() {
        assert!(header_visible(false));
    }

    #[test]
    fn header_is_hidden_when_fullscreen() {
        assert!(!header_visible(true));
    }
}
