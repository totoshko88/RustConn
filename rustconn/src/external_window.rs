//! External window support for connections
//!
//! This module provides functionality for opening connections in separate
//! external windows instead of embedded in the main application window.

use gtk4::prelude::*;
use gtk4::{ApplicationWindow, Box as GtkBox, HeaderBar, Label, Orientation};
use rustconn_core::models::{Connection, WindowGeometry};
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;
use vte4::Terminal;

/// Callback type for when an external window is closed
pub type WindowCloseCallback = Rc<RefCell<Option<Box<dyn Fn(Uuid, Option<WindowGeometry>)>>>>;

/// External window for a connection session
pub struct ExternalWindow {
    window: ApplicationWindow,
    connection_id: Uuid,
    session_id: Uuid,
    on_close: WindowCloseCallback,
}

impl ExternalWindow {
    /// Creates a new external window for a connection
    #[must_use]
    pub fn new(
        app: &gtk4::Application,
        connection: &Connection,
        session_id: Uuid,
        terminal: &Terminal,
    ) -> Self {
        let window = ApplicationWindow::builder()
            .application(app)
            .title(format!("{} - RustConn", connection.name))
            .default_width(800)
            .default_height(600)
            .build();

        // Apply saved geometry if available and remember_window_position is enabled
        if connection.remember_window_position {
            if let Some(geometry) = &connection.window_geometry {
                if geometry.is_valid() {
                    window.set_default_size(geometry.width, geometry.height);
                }
            }
        }

        // Create header bar
        let header = HeaderBar::new();
        header.set_show_title_buttons(true);

        let title = Label::new(Some(&connection.name));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));

        window.set_titlebar(Some(&header));

        // Create content container
        let content = GtkBox::new(Orientation::Vertical, 0);
        content.set_vexpand(true);
        content.set_hexpand(true);

        // Clone terminal widget for the external window
        // Note: VTE terminals can only have one parent, so we need to reparent
        terminal.unparent();
        content.append(terminal);

        window.set_child(Some(&content));

        let on_close: WindowCloseCallback = Rc::new(RefCell::new(None));

        let external_window = Self {
            window,
            connection_id: connection.id,
            session_id,
            on_close,
        };

        external_window.setup_close_handler();

        external_window
    }

    /// Creates a new fullscreen window for a connection
    #[must_use]
    pub fn new_fullscreen(
        app: &gtk4::Application,
        connection: &Connection,
        session_id: Uuid,
        terminal: &Terminal,
    ) -> Self {
        let window = ApplicationWindow::builder()
            .application(app)
            .title(format!("{} - RustConn", connection.name))
            .decorated(false)
            .build();

        // Create content container
        let content = GtkBox::new(Orientation::Vertical, 0);
        content.set_vexpand(true);
        content.set_hexpand(true);

        // Clone terminal widget for the external window
        terminal.unparent();
        content.append(terminal);

        window.set_child(Some(&content));

        // Set fullscreen after showing
        window.fullscreen();

        let on_close: WindowCloseCallback = Rc::new(RefCell::new(None));

        let external_window = Self {
            window,
            connection_id: connection.id,
            session_id,
            on_close,
        };

        external_window.setup_close_handler();

        external_window
    }

    /// Sets up the close handler to capture geometry and notify callback
    fn setup_close_handler(&self) {
        let _connection_id = self.connection_id;
        let session_id = self.session_id;
        let on_close = self.on_close.clone();
        let window_weak = self.window.downgrade();

        self.window.connect_close_request(move |_window| {
            // Get current geometry before closing
            let geometry = if let Some(win) = window_weak.upgrade() {
                let (width, height) = win.default_size();
                // Note: On Wayland, we can't reliably get window position
                // We'll use 0,0 as placeholder - the size is what matters most
                Some(WindowGeometry::new(0, 0, width, height))
            } else {
                None
            };

            // Call the close callback
            if let Some(ref cb) = *on_close.borrow() {
                cb(session_id, geometry);
            }

            // Allow the window to close
            glib::Propagation::Proceed
        });

        // Also handle when the session ends (terminal child exits)
        // This will be connected by the caller
    }

    /// Sets the callback for when the window is closed
    pub fn set_on_close<F>(&self, callback: F)
    where
        F: Fn(Uuid, Option<WindowGeometry>) + 'static,
    {
        *self.on_close.borrow_mut() = Some(Box::new(callback));
    }

    /// Shows the window
    pub fn show(&self) {
        self.window.present();
    }

    /// Gets the window widget
    #[must_use]
    pub const fn window(&self) -> &ApplicationWindow {
        &self.window
    }

    /// Gets the connection ID
    #[must_use]
    pub const fn connection_id(&self) -> Uuid {
        self.connection_id
    }

    /// Gets the session ID
    #[must_use]
    pub const fn session_id(&self) -> Uuid {
        self.session_id
    }

    /// Closes the window
    pub fn close(&self) {
        self.window.close();
    }
}

/// Manager for external windows
pub struct ExternalWindowManager {
    windows: Rc<RefCell<Vec<ExternalWindow>>>,
}

impl Default for ExternalWindowManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ExternalWindowManager {
    /// Creates a new external window manager
    #[must_use]
    pub fn new() -> Self {
        Self {
            windows: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Adds an external window to the manager
    pub fn add_window(&self, window: ExternalWindow) {
        self.windows.borrow_mut().push(window);
    }

    /// Removes a window by session ID
    #[must_use]
    pub fn remove_window(&self, session_id: Uuid) -> Option<ExternalWindow> {
        let mut windows = self.windows.borrow_mut();
        windows
            .iter()
            .position(|w| w.session_id == session_id)
            .map(|pos| windows.remove(pos))
    }

    /// Checks if a window exists for the given session ID
    #[must_use]
    pub fn has_window(&self, session_id: Uuid) -> bool {
        self.windows
            .borrow()
            .iter()
            .any(|w| w.session_id == session_id)
    }

    /// Executes a closure with a reference to the window if found
    ///
    /// This is the safe way to access windows since `ExternalWindow` cannot be cloned.
    pub fn with_window<F, R>(&self, session_id: Uuid, f: F) -> Option<R>
    where
        F: FnOnce(&ExternalWindow) -> R,
    {
        self.windows
            .borrow()
            .iter()
            .find(|w| w.session_id == session_id)
            .map(f)
    }

    /// Executes a closure with a mutable reference to the window if found
    pub fn with_window_mut<F, R>(&self, session_id: Uuid, f: F) -> Option<R>
    where
        F: FnOnce(&mut ExternalWindow) -> R,
    {
        self.windows
            .borrow_mut()
            .iter_mut()
            .find(|w| w.session_id == session_id)
            .map(f)
    }

    /// Gets all session IDs of open windows
    #[must_use]
    pub fn session_ids(&self) -> Vec<Uuid> {
        self.windows.borrow().iter().map(|w| w.session_id).collect()
    }

    /// Closes all external windows
    pub fn close_all(&self) {
        for window in self.windows.borrow().iter() {
            window.close();
        }
        self.windows.borrow_mut().clear();
    }

    /// Returns the number of open external windows
    #[must_use]
    pub fn window_count(&self) -> usize {
        self.windows.borrow().len()
    }
}

use gtk4::glib;
