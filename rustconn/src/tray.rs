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

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
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
    use super::*;
    use ksni::blocking::{Handle, TrayMethods};
    use ksni::{menu::StandardItem, Icon, MenuItem, Tray};
    use std::sync::mpsc::Sender;

    /// Embedded SVG icon data
    const ICON_SVG: &[u8] =
        include_bytes!("../assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg");

    /// Render SVG to ARGB32 pixmap for tray icon
    /// Returns Vec<Icon> with rendered icon at specified size
    pub fn render_svg_to_pixmap(size: u32) -> Vec<Icon> {
        // Parse SVG
        let tree = match resvg::usvg::Tree::from_data(ICON_SVG, &resvg::usvg::Options::default()) {
            Ok(tree) => tree,
            Err(_) => return Vec::new(),
        };

        // Create pixmap
        let mut pixmap = match resvg::tiny_skia::Pixmap::new(size, size) {
            Some(p) => p,
            None => return Vec::new(),
        };

        // Calculate transform to fit SVG into target size
        let svg_size = tree.size();
        let scale = (size as f32 / svg_size.width()).min(size as f32 / svg_size.height());
        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);

        // Render SVG to pixmap
        resvg::render(&tree, transform, &mut pixmap.as_mut());

        // Convert from RGBA to ARGB (StatusNotifierItem format)
        // tiny_skia uses premultiplied RGBA, we need ARGB in network byte order
        let rgba_data = pixmap.data();
        let argb_data: Vec<u8> = rgba_data
            .chunks_exact(4)
            .flat_map(|rgba| [rgba[3], rgba[0], rgba[1], rgba[2]]) // RGBA -> ARGB
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
        pub sender: Sender<TrayMessage>,
        /// Pre-rendered icon pixmap
        pub icon_pixmap: Vec<Icon>,
    }

    impl Tray for RustConnTray {
        fn icon_name(&self) -> String {
            // Return empty - we use icon_pixmap instead
            String::new()
        }

        fn icon_theme_path(&self) -> String {
            String::new()
        }

        fn icon_pixmap(&self) -> Vec<Icon> {
            // Return pre-rendered ARGB32 icon
            self.icon_pixmap.clone()
        }

        fn title(&self) -> String {
            "RustConn".to_string()
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let description = if state.active_sessions > 0 {
                format!("{} active session(s)", state.active_sessions)
            } else {
                "No active sessions".to_string()
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

        fn menu(&self) -> Vec<MenuItem<Self>> {
            // Clone state data to avoid holding lock during menu construction
            let (window_visible, recent_connections, active_sessions) = {
                match self.state.lock() {
                    Ok(state) => (
                        state.window_visible,
                        state.recent_connections.clone(),
                        state.active_sessions,
                    ),
                    Err(e) => {
                        // Poisoned lock - recover with defaults
                        let state = e.into_inner();
                        (
                            state.window_visible,
                            state.recent_connections.clone(),
                            state.active_sessions,
                        )
                    }
                }
            };

            let mut items: Vec<MenuItem<Self>> = Vec::new();

            // Toggle window visibility
            let label = if window_visible {
                "Hide Window"
            } else {
                "Show Window"
            };
            items.push(MenuItem::Standard(StandardItem {
                label: label.to_string(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayMessage::ToggleWindow);
                }),
                ..Default::default()
            }));

            items.push(MenuItem::Separator);

            // Recent connections submenu
            if !recent_connections.is_empty() {
                let recent_items: Vec<MenuItem<Self>> = recent_connections
                    .iter()
                    .take(10)
                    .map(|(id, name)| {
                        let conn_id = *id;
                        MenuItem::Standard(StandardItem {
                            label: name.clone(),
                            activate: Box::new(move |tray: &mut Self| {
                                let _ = tray.sender.send(TrayMessage::Connect(conn_id));
                            }),
                            ..Default::default()
                        })
                    })
                    .collect();

                items.push(MenuItem::SubMenu(ksni::menu::SubMenu {
                    label: "Recent Connections".to_string(),
                    submenu: recent_items,
                    ..Default::default()
                }));

                items.push(MenuItem::Separator);
            }

            // Quick connect
            items.push(MenuItem::Standard(StandardItem {
                label: "Quick Connect...".to_string(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayMessage::QuickConnect);
                }),
                ..Default::default()
            }));

            // Local Shell
            items.push(MenuItem::Standard(StandardItem {
                label: "Local Shell".to_string(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayMessage::LocalShell);
                }),
                ..Default::default()
            }));

            items.push(MenuItem::Separator);

            // Session count (informational)
            if active_sessions > 0 {
                items.push(MenuItem::Standard(StandardItem {
                    label: format!("{} Active Session(s)", active_sessions),
                    enabled: false,
                    ..Default::default()
                }));
                items.push(MenuItem::Separator);
            }

            // About
            items.push(MenuItem::Standard(StandardItem {
                label: "About".to_string(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayMessage::About);
                }),
                ..Default::default()
            }));

            // Quit
            items.push(MenuItem::Standard(StandardItem {
                label: "Quit".to_string(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayMessage::Quit);
                }),
                ..Default::default()
            }));

            items
        }
    }

    /// Tray icon manager (with tray feature enabled)
    pub struct TrayManager {
        state: Arc<Mutex<TrayState>>,
        receiver: Receiver<TrayMessage>,
        handle: Handle<RustConnTray>,
        /// Track if update is needed to avoid excessive D-Bus calls
        needs_update: Arc<Mutex<bool>>,
    }

    impl TrayManager {
        /// Creates a new tray manager and starts the tray icon service
        #[must_use]
        pub fn new() -> Option<Self> {
            let (sender, receiver) = mpsc::channel();
            let state = Arc::new(Mutex::new(TrayState::default()));

            // Render SVG to 32x32 ARGB32 pixmap
            let icon_pixmap = render_svg_to_pixmap(32);

            let tray = RustConnTray {
                state: Arc::clone(&state),
                sender,
                icon_pixmap,
            };

            // Use blocking spawn from ksni 0.3
            let handle = tray.spawn().ok()?;

            Some(Self {
                state,
                receiver,
                handle,
                needs_update: Arc::new(Mutex::new(false)),
            })
        }

        /// Trigger a menu update on the D-Bus service
        fn trigger_update(&self) {
            // Only update if there are actual changes
            if let Ok(mut needs) = self.needs_update.lock() {
                if *needs {
                    let _ = self.handle.update(|_| {});
                    *needs = false;
                }
            }
        }

        pub fn set_active_sessions(&self, count: u32) {
            if let Ok(mut state) = self.state.lock() {
                if state.active_sessions != count {
                    state.active_sessions = count;
                    if let Ok(mut needs) = self.needs_update.lock() {
                        *needs = true;
                    }
                }
            }
            self.trigger_update();
        }

        pub fn set_recent_connections(&self, connections: Vec<(Uuid, String)>) {
            if let Ok(mut state) = self.state.lock() {
                if state.recent_connections != connections {
                    state.recent_connections = connections;
                    if let Ok(mut needs) = self.needs_update.lock() {
                        *needs = true;
                    }
                }
            }
            self.trigger_update();
        }

        pub fn set_window_visible(&self, visible: bool) {
            if let Ok(mut state) = self.state.lock() {
                if state.window_visible != visible {
                    state.window_visible = visible;
                    if let Ok(mut needs) = self.needs_update.lock() {
                        *needs = true;
                    }
                }
            }
            self.trigger_update();
        }

        pub fn try_recv(&self) -> Option<TrayMessage> {
            self.receiver.try_recv().ok()
        }
    }
}

#[cfg(feature = "tray")]
pub use tray_impl::TrayManager;

// ============================================================================
// Stub implementation when the "tray" feature is disabled
// ============================================================================

#[cfg(not(feature = "tray"))]
mod tray_stub {
    use super::*;

    pub struct TrayManager {
        state: Arc<Mutex<TrayState>>,
        receiver: Receiver<TrayMessage>,
    }

    impl TrayManager {
        #[must_use]
        pub fn new() -> Option<Self> {
            None
        }

        pub fn set_active_sessions(&self, count: u32) {
            if let Ok(mut state) = self.state.lock() {
                state.active_sessions = count;
            }
        }

        pub fn set_recent_connections(&self, connections: Vec<(Uuid, String)>) {
            if let Ok(mut state) = self.state.lock() {
                state.recent_connections = connections;
            }
        }

        pub fn set_window_visible(&self, visible: bool) {
            if let Ok(mut state) = self.state.lock() {
                state.window_visible = visible;
            }
        }

        pub fn try_recv(&self) -> Option<TrayMessage> {
            self.receiver.try_recv().ok()
        }
    }

    impl Default for TrayManager {
        fn default() -> Self {
            Self {
                state: Arc::new(Mutex::new(TrayState::default())),
                receiver: mpsc::channel().1,
            }
        }
    }
}

#[cfg(not(feature = "tray"))]
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
        // ARGB32: 4 bytes per pixel, 32x32 = 4096 bytes
        assert_eq!(icon.data.len(), 4096);

        // Check that icon has visible pixels (non-zero alpha)
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
