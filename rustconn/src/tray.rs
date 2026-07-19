//! System tray icon implementation
//!
//! This module provides tray icon support using the StatusNotifierItem D-Bus protocol
//! via the ksni crate, which is the standard for system tray icons on modern Linux
//! desktops (GNOME, KDE, etc.) and works with Wayland.
//!
//! # Icon Rendering
//!
//! The tray icon is rendered from SVG to ARGB32 pixmap format using resvg.
//! This ensures compatibility with all StatusNotifierItem implementations
//! including GNOME's AppIndicator extension.
//!
//! # System Requirements
//!
//! This feature requires the `libdbus-1-dev` package to be installed:
//! - Ubuntu/Debian: `sudo apt install libdbus-1-dev pkg-config`
//! - Fedora: `sudo dnf install dbus-devel pkgconf-pkg-config`
//!
//! # Feature Flag
//!
//! The tray icon feature is enabled by default but can be disabled by building
//! with `--no-default-features` if the D-Bus dependency is not available.

use std::sync::{Arc, Mutex};

use gettextrs::gettext;
use uuid::Uuid;

/// Messages sent from the tray icon to the main application
#[derive(Debug, Clone)]
pub enum TrayMessage {
    /// Show the main window
    ShowWindow,
    /// Hide the main window
    HideWindow,
    /// Toggle window visibility
    ToggleWindow,
    /// Connect to a specific connection by ID
    Connect(Uuid),
    /// Open quick connect dialog
    QuickConnect,
    /// Open local shell
    LocalShell,
    /// Show about dialog
    About,
    /// Quit the application
    Quit,
}

/// Tray icon state
#[derive(Debug, Clone)]
pub struct TrayState {
    /// Number of active sessions
    pub active_sessions: u32,
    /// Recent connections (id, name)
    pub recent_connections: Vec<(Uuid, String)>,
    /// Whether the main window is visible
    pub window_visible: bool,
}

impl Default for TrayState {
    fn default() -> Self {
        Self {
            active_sessions: 0,
            recent_connections: Vec::new(),
            window_visible: true,
        }
    }
}

// ============================================================================
// Tray implementation when the "tray" feature is enabled
// ============================================================================

#[cfg(feature = "tray")]
mod tray_impl {
    use std::sync::mpsc;

    use ksni::blocking::{Handle, TrayMethods};
    use ksni::menu::StandardItem;
    use ksni::{Icon, MenuItem, Tray};

    use super::*;

    /// Embedded SVG icon data (tray-specific variant with cream halo for
    /// visibility on dark KDE/Plasma panels — see issue #157).
    const ICON_SVG: &[u8] = include_bytes!(
        "../assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn-tray.svg"
    );

    /// Render SVG to ARGB32 pixmap for tray icon
    /// Returns Vec<Icon> with rendered icon at specified size
    pub fn render_svg_to_pixmap(size: u32) -> Vec<Icon> {
        let tree = match resvg::usvg::Tree::from_data(ICON_SVG, &resvg::usvg::Options::default()) {
            Ok(tree) => tree,
            Err(_) => return Vec::new(),
        };
        let mut pixmap = match resvg::tiny_skia::Pixmap::new(size, size) {
            Some(p) => p,
            None => return Vec::new(),
        };
        let svg_size = tree.size();
        let scale = (size as f32 / svg_size.width()).min(size as f32 / svg_size.height());
        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        let rgba_data = pixmap.data();
        let argb_data: Vec<u8> = rgba_data
            .chunks_exact(4)
            .flat_map(|rgba| [rgba[3], rgba[0], rgba[1], rgba[2]])
            .collect();
        vec![Icon {
            width: size as i32,
            height: size as i32,
            data: argb_data,
        }]
    }

    /// RustConn tray icon implementation
    pub struct RustConnTray {
        pub state: Arc<Mutex<TrayState>>,
        /// Unbounded channel to the GTK main loop; `try_send` never blocks
        /// the ksni D-Bus thread and wakes the main loop only on real events.
        pub sender: async_channel::Sender<TrayMessage>,
        pub icon_pixmap: Vec<Icon>,
    }

    impl Tray for RustConnTray {
        fn icon_name(&self) -> String {
            String::new()
        }
        fn icon_theme_path(&self) -> String {
            String::new()
        }
        fn icon_pixmap(&self) -> Vec<Icon> {
            self.icon_pixmap.clone()
        }
        fn title(&self) -> String {
            "RustConn".to_string()
        }
        fn tool_tip(&self) -> ksni::ToolTip {
            let state = match self.state.lock() {
                Ok(s) => s,
                Err(e) => e.into_inner(),
            };
            let description = if state.active_sessions > 0 {
                let mut msg = gettext("{} active session(s)");
                if let Some(pos) = msg.find("{}") {
                    msg.replace_range(pos..pos + 2, &state.active_sessions.to_string());
                }
                msg
            } else {
                gettext("No active sessions")
            };
            ksni::ToolTip {
                icon_name: String::new(),
                icon_pixmap: Vec::new(),
                title: "RustConn".to_string(),
                description,
            }
        }
        fn id(&self) -> String {
            "io.github.totoshko88.RustConn".to_string()
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            let _ = self.sender.try_send(TrayMessage::ToggleWindow);
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            // Read state — lock is held briefly just to clone data.
            let (window_visible, recent_connections, active_sessions) = {
                let state = match self.state.lock() {
                    Ok(s) => s,
                    Err(e) => e.into_inner(),
                };
                (
                    state.window_visible,
                    state.recent_connections.clone(),
                    state.active_sessions,
                )
            };

            let mut items: Vec<MenuItem<Self>> = Vec::new();

            let toggle_label = if window_visible {
                gettext("Hide Window")
            } else {
                gettext("Show Window")
            };
            items.push(MenuItem::Standard(StandardItem {
                label: toggle_label,
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.try_send(TrayMessage::ToggleWindow);
                }),
                ..Default::default()
            }));
            items.push(MenuItem::Separator);

            if !recent_connections.is_empty() {
                let recent_items: Vec<MenuItem<Self>> = recent_connections
                    .iter()
                    .take(10)
                    .map(|(id, name)| {
                        let conn_id = *id;
                        MenuItem::Standard(StandardItem {
                            label: name.clone(),
                            activate: Box::new(move |tray: &mut Self| {
                                let _ = tray.sender.try_send(TrayMessage::Connect(conn_id));
                            }),
                            ..Default::default()
                        })
                    })
                    .collect();
                items.push(MenuItem::SubMenu(ksni::menu::SubMenu {
                    label: gettext("Recent Connections"),
                    submenu: recent_items,
                    ..Default::default()
                }));
                items.push(MenuItem::Separator);
            }

            items.push(MenuItem::Standard(StandardItem {
                label: gettext("Quick Connect..."),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.try_send(TrayMessage::QuickConnect);
                }),
                ..Default::default()
            }));
            items.push(MenuItem::Standard(StandardItem {
                label: gettext("Local Shell"),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.try_send(TrayMessage::LocalShell);
                }),
                ..Default::default()
            }));
            items.push(MenuItem::Separator);

            if active_sessions > 0 {
                let mut label = gettext("{} Active Session(s)");
                if let Some(pos) = label.find("{}") {
                    label.replace_range(pos..pos + 2, &active_sessions.to_string());
                }
                items.push(MenuItem::Standard(StandardItem {
                    label,
                    enabled: false,
                    ..Default::default()
                }));
                items.push(MenuItem::Separator);
            }

            items.push(MenuItem::Standard(StandardItem {
                label: gettext("About RustConn"),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.try_send(TrayMessage::About);
                }),
                ..Default::default()
            }));
            items.push(MenuItem::Standard(StandardItem {
                label: gettext("Quit"),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.try_send(TrayMessage::Quit);
                }),
                ..Default::default()
            }));

            items
        }
    }

    /// Tray icon manager (with tray feature enabled)
    ///
    /// All D-Bus updates are dispatched to a dedicated background thread to
    /// avoid blocking the GTK main loop.  `handle.update()` internally calls
    /// `compat::block_on` which parks the *calling* thread until the D-Bus
    /// service loop processes the request.  Running that on the GTK thread
    /// can deadlock (the D-Bus thread may need the `TrayState` mutex that
    /// the GTK thread is about to take) or simply stall the UI.
    pub struct TrayManager {
        state: Arc<Mutex<TrayState>>,
        receiver: async_channel::Receiver<TrayMessage>,
        /// Channel to the background updater thread.
        update_tx: std::sync::mpsc::SyncSender<()>,
        /// Keep the handle alive so the D-Bus service loop is not dropped.
        _handle: Handle<RustConnTray>,
    }

    impl TrayManager {
        #[must_use]
        pub fn new() -> Option<Self> {
            let (sender, receiver) = async_channel::unbounded();
            let state = Arc::new(Mutex::new(TrayState::default()));
            let icon_pixmap = render_svg_to_pixmap(32);
            let tray = RustConnTray {
                state: Arc::clone(&state),
                sender,
                icon_pixmap,
            };

            // In Flatpak sandboxes the D-Bus well-known name
            // `StatusNotifierItem-PID-ID` cannot be owned; ksni documents
            // `disable_dbus_name(true)` as the required workaround.
            let in_flatpak = rustconn_core::flatpak::is_flatpak();
            let handle = tray.disable_dbus_name(in_flatpak).spawn().ok()?;

            // Spawn a dedicated thread that serialises all `handle.update()`
            // calls off the GTK main thread.  We use a bounded(1) channel so
            // that multiple rapid state changes coalesce into a single update
            // (the sender simply drops the message if the channel is full).
            let (update_tx, update_rx) = mpsc::sync_channel::<()>(1);
            let bg_handle = handle.clone();
            std::thread::Builder::new()
                .name("tray-updater".into())
                .spawn(move || {
                    while update_rx.recv().is_ok() {
                        // Drain any extra coalesced signals so we do one
                        // update per burst.
                        while update_rx.try_recv().is_ok() {}
                        if bg_handle.is_closed() {
                            break;
                        }
                        let _ = bg_handle.update(|_| {});
                    }
                })
                .ok()?;

            Some(Self {
                state,
                receiver,
                update_tx,
                _handle: handle,
            })
        }

        /// Request a D-Bus menu/property refresh (non-blocking).
        ///
        /// The actual `handle.update()` runs on the background updater
        /// thread.  If an update is already queued the new request is
        /// coalesced (bounded channel capacity = 1).
        fn request_update(&self) {
            // `try_send` never blocks; if the channel is full an update is
            // already pending — exactly what we want.
            let _ = self.update_tx.try_send(());
        }

        pub fn force_refresh(&self) {
            self.request_update();
        }

        pub fn set_active_sessions(&self, count: u32) {
            if let Ok(mut state) = self.state.lock()
                && state.active_sessions != count
            {
                state.active_sessions = count;
                self.request_update();
            }
        }

        pub fn set_recent_connections(&self, connections: Vec<(Uuid, String)>) {
            if let Ok(mut state) = self.state.lock()
                && state.recent_connections != connections
            {
                state.recent_connections = connections;
                self.request_update();
            }
        }

        pub fn set_window_visible(&self, visible: bool) {
            if let Ok(mut state) = self.state.lock()
                && state.window_visible != visible
            {
                state.window_visible = visible;
                self.request_update();
            }
        }

        /// Returns a clone of the tray message receiver for the GTK main
        /// loop to await on (event-driven, no polling).
        #[must_use]
        pub fn message_receiver(&self) -> async_channel::Receiver<TrayMessage> {
            self.receiver.clone()
        }
    }
}

#[cfg(feature = "tray")]
pub use tray_impl::TrayManager;

// ============================================================================
// macOS tray implementation using tray-icon + muda (NSStatusItem)
// ============================================================================

#[cfg(feature = "tray-macos")]
mod tray_macos_impl {
    use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    use tray_icon::TrayIconBuilder;

    use super::*;

    /// Embedded SVG icon data (same as Linux tray)
    const ICON_SVG: &[u8] =
        include_bytes!("../assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg");

    /// Render SVG to RGBA pixmap for tray-icon crate
    fn render_svg_to_rgba(size: u32) -> Option<Vec<u8>> {
        let tree = resvg::usvg::Tree::from_data(ICON_SVG, &resvg::usvg::Options::default()).ok()?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
        let svg_size = tree.size();
        let scale = (size as f32 / svg_size.width()).min(size as f32 / svg_size.height());
        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        Some(pixmap.data().to_vec())
    }

    /// Menu item IDs for the macOS tray
    const ID_TOGGLE_WINDOW: &str = "toggle-window";
    const ID_QUICK_CONNECT: &str = "quick-connect";
    const ID_LOCAL_SHELL: &str = "local-shell";
    const ID_ABOUT: &str = "about";
    const ID_QUIT: &str = "quit";
    const ID_CONNECT_PREFIX: &str = "connect:";

    /// macOS tray icon manager using NSStatusItem via tray-icon crate.
    ///
    /// **IMPORTANT:** Must be created on the main thread — macOS AppKit
    /// requires `NSStatusItem` allocation on the main thread.
    pub struct TrayManager {
        state: Arc<Mutex<TrayState>>,
        receiver: async_channel::Receiver<TrayMessage>,
        tray_icon: tray_icon::TrayIcon,
    }

    impl TrayManager {
        /// Creates a new macOS tray icon.
        ///
        /// **Must be called from the main thread** (macOS AppKit requirement).
        /// Unlike the Linux `ksni` tray which uses D-Bus and can be spawned
        /// on a background thread, `NSStatusItem` will silently fail or crash
        /// if created off the main thread.
        #[must_use]
        pub fn new() -> Option<Self> {
            let (sender, receiver) = async_channel::unbounded();
            let state = Arc::new(Mutex::new(TrayState::default()));

            // Render icon at 44×44px (Retina 2x) — macOS menu bar auto-scales
            // NSImage to fit the 22pt status item height. Providing 44px ensures
            // crisp rendering on Retina displays without blur.
            let rgba_data = if let Some(data) = render_svg_to_rgba(44) {
                data
            } else {
                tracing::warn!("macOS tray: failed to render SVG icon");
                return None;
            };
            let icon = match tray_icon::Icon::from_rgba(rgba_data, 44, 44) {
                Ok(i) => i,
                Err(e) => {
                    tracing::warn!(%e, "macOS tray: failed to create icon from RGBA");
                    return None;
                }
            };

            // Build menu
            let menu = Self::build_menu(&state);

            // Create tray icon (must be on main thread)
            // Note: icon_as_template=false shows the full-color icon in the menu bar.
            // Template mode (true) requires a monochrome black+alpha image;
            // our SVG is full-color so template mode would render it invisible.
            let tray_icon = match TrayIconBuilder::new()
                .with_icon(icon)
                .with_icon_as_template(false)
                .with_tooltip("RustConn")
                .with_menu(Box::new(menu))
                .build()
            {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(%e, "macOS tray: TrayIconBuilder::build() failed");
                    return None;
                }
            };

            // macOS workaround: explicitly set visible after creation.
            // On some macOS versions (Ventura+), the NSStatusItem is created
            // but not shown until explicitly toggled. This ensures visibility.
            tray_icon.set_visible(true).ok();

            // Set up menu event handler on a background thread.
            // MenuEvent::receiver() is thread-safe — only the TrayIcon itself
            // must live on the main thread.
            let sender_for_events = sender.clone();
            std::thread::Builder::new()
                .name("tray-macos-events".into())
                .spawn(move || {
                    let menu_rx = MenuEvent::receiver();
                    while let Ok(event) = menu_rx.recv() {
                        let id_str = event.id().0.as_str();
                        let msg = match id_str {
                            ID_TOGGLE_WINDOW => Some(TrayMessage::ToggleWindow),
                            ID_QUICK_CONNECT => Some(TrayMessage::QuickConnect),
                            ID_LOCAL_SHELL => Some(TrayMessage::LocalShell),
                            ID_ABOUT => Some(TrayMessage::About),
                            ID_QUIT => Some(TrayMessage::Quit),
                            other if other.starts_with(ID_CONNECT_PREFIX) => {
                                let uuid_str = &other[ID_CONNECT_PREFIX.len()..];
                                Uuid::parse_str(uuid_str).ok().map(TrayMessage::Connect)
                            }
                            _ => None,
                        };
                        if let Some(m) = msg {
                            let _ = sender_for_events.try_send(m);
                        }
                    }
                })
                .ok()?;

            Some(Self {
                state,
                receiver,
                tray_icon,
            })
        }

        fn build_menu(state: &Arc<Mutex<TrayState>>) -> Menu {
            let menu = Menu::new();

            let toggle_label = {
                let s = state.lock().unwrap_or_else(|e| e.into_inner());
                if s.window_visible {
                    gettext("Hide Window")
                } else {
                    gettext("Show Window")
                }
            };

            let _ = menu.append(&MenuItem::with_id(
                muda::MenuId(ID_TOGGLE_WINDOW.into()),
                &toggle_label,
                true,
                None,
            ));
            let _ = menu.append(&PredefinedMenuItem::separator());

            // Recent connections submenu
            {
                let s = state.lock().unwrap_or_else(|e| e.into_inner());
                if !s.recent_connections.is_empty() {
                    let submenu = Submenu::new(&gettext("Recent Connections"), true);
                    for (id, name) in s.recent_connections.iter().take(10) {
                        let menu_id = format!("{ID_CONNECT_PREFIX}{id}");
                        let _ = submenu.append(&MenuItem::with_id(
                            muda::MenuId(menu_id),
                            name,
                            true,
                            None,
                        ));
                    }
                    let _ = menu.append(&submenu);
                    let _ = menu.append(&PredefinedMenuItem::separator());
                }
            }

            // Active sessions count (informational, disabled)
            {
                let s = state.lock().unwrap_or_else(|e| e.into_inner());
                if s.active_sessions > 0 {
                    let mut label = gettext("{} Active Session(s)");
                    if let Some(pos) = label.find("{}") {
                        label.replace_range(pos..pos + 2, &s.active_sessions.to_string());
                    }
                    let _ = menu.append(&MenuItem::with_id(
                        muda::MenuId("active-sessions".into()),
                        &label,
                        false,
                        None,
                    ));
                    let _ = menu.append(&PredefinedMenuItem::separator());
                }
            }

            let _ = menu.append(&MenuItem::with_id(
                muda::MenuId(ID_QUICK_CONNECT.into()),
                &gettext("Quick Connect..."),
                true,
                None,
            ));
            let _ = menu.append(&MenuItem::with_id(
                muda::MenuId(ID_LOCAL_SHELL.into()),
                &gettext("Local Shell"),
                true,
                None,
            ));
            let _ = menu.append(&PredefinedMenuItem::separator());
            let _ = menu.append(&MenuItem::with_id(
                muda::MenuId(ID_ABOUT.into()),
                &gettext("About RustConn"),
                true,
                None,
            ));
            let _ = menu.append(&MenuItem::with_id(
                muda::MenuId(ID_QUIT.into()),
                &gettext("Quit"),
                true,
                None,
            ));

            menu
        }

        /// Rebuilds and replaces the tray menu to reflect current state.
        ///
        /// Unlike Linux ksni which has `handle.update()`, macOS `tray-icon`
        /// does not automatically rebuild the menu on open. We must explicitly
        /// call `set_menu()` whenever state changes.
        fn rebuild_menu(&self) {
            let menu = Self::build_menu(&self.state);
            self.tray_icon.set_menu(Some(Box::new(menu)));
        }

        pub fn force_refresh(&self) {
            self.rebuild_menu();
        }

        pub fn set_active_sessions(&self, count: u32) {
            let changed = if let Ok(mut state) = self.state.lock() {
                if state.active_sessions == count {
                    false
                } else {
                    state.active_sessions = count;
                    true
                }
            } else {
                false
            };
            if changed {
                self.rebuild_menu();
            }
        }

        pub fn set_recent_connections(&self, connections: Vec<(Uuid, String)>) {
            let changed = if let Ok(mut state) = self.state.lock() {
                if state.recent_connections == connections {
                    false
                } else {
                    state.recent_connections = connections;
                    true
                }
            } else {
                false
            };
            if changed {
                self.rebuild_menu();
            }
        }

        pub fn set_window_visible(&self, visible: bool) {
            let changed = if let Ok(mut state) = self.state.lock() {
                if state.window_visible == visible {
                    false
                } else {
                    state.window_visible = visible;
                    true
                }
            } else {
                false
            };
            if changed {
                self.rebuild_menu();
            }
        }

        /// Returns a clone of the tray message receiver for the GTK main
        /// loop to await on (event-driven, no polling).
        #[must_use]
        pub fn message_receiver(&self) -> async_channel::Receiver<TrayMessage> {
            self.receiver.clone()
        }
    }
}

#[cfg(feature = "tray-macos")]
pub use tray_macos_impl::TrayManager;

// ============================================================================
// Stub implementation when no tray feature is enabled
// ============================================================================

#[cfg(not(any(feature = "tray", feature = "tray-macos")))]
mod tray_stub {
    use super::*;

    pub struct TrayManager;

    impl TrayManager {
        #[must_use]
        pub fn new() -> Option<Self> {
            None
        }
        pub fn set_active_sessions(&self, _count: u32) {}
        pub fn set_recent_connections(&self, _connections: Vec<(Uuid, String)>) {}
        pub fn set_window_visible(&self, _visible: bool) {}
        pub fn force_refresh(&self) {}
        /// Returns an already-closed receiver: the stub never produces
        /// messages, so the consumer loop exits immediately.
        #[must_use]
        pub fn message_receiver(&self) -> async_channel::Receiver<TrayMessage> {
            let (_, receiver) = async_channel::unbounded();
            receiver
        }
    }

    impl Default for TrayManager {
        fn default() -> Self {
            Self
        }
    }
}

#[cfg(not(any(feature = "tray", feature = "tray-macos")))]
pub use tray_stub::TrayManager;

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(all(test, feature = "tray"))]
mod tests {
    use super::tray_impl::render_svg_to_pixmap;

    #[test]
    fn test_render_svg_to_pixmap_32x32() {
        let icons = render_svg_to_pixmap(32);
        assert_eq!(icons.len(), 1, "Should render exactly one icon");
        let icon = &icons[0];
        assert_eq!(icon.width, 32);
        assert_eq!(icon.height, 32);
        assert_eq!(icon.data.len(), 4096);
        let has_visible = icon.data.chunks(4).any(|argb| argb[0] > 0);
        assert!(has_visible, "Icon should have visible pixels");
    }

    #[test]
    fn test_render_svg_to_pixmap_64x64() {
        let icons = render_svg_to_pixmap(64);
        assert_eq!(icons.len(), 1);
        let icon = &icons[0];
        assert_eq!(icon.width, 64);
        assert_eq!(icon.height, 64);
        assert_eq!(icon.data.len(), 64 * 64 * 4);
    }
}
