//! RDP session widget with FreeRDP integration
//!
//! This module provides the `EmbeddedRdpWidget` struct for RDP session management
//! within the GTK4 application.
//!
//! # Architecture
//!
//! Unlike VNC which uses a pure Rust client (`vnc-rs`) for true embedded rendering,
//! RDP sessions use FreeRDP subprocess (wlfreerdp/xfreerdp) which opens its own window.
//! The widget displays connection status and manages the FreeRDP process lifecycle.
//!
//! ## Why not true embedded RDP?
//!
//! True embedded RDP (rendering frames directly in our GTK widget) would require:
//! - A pure Rust RDP client like `ironrdp` (complex API, limited documentation)
//! - Or FreeRDP with custom frame capture (requires FreeRDP modifications)
//!
//! The current approach provides:
//! - Reliable RDP connections via mature FreeRDP
//! - Session management (start/stop/status)
//! - Automatic client detection (wlfreerdp, xfreerdp3, xfreerdp)
//! - Qt/Wayland warning suppression for better compatibility
//!
//! # Client Mode
//!
//! - **Embedded mode**: Uses wlfreerdp (preferred) - opens separate window but managed by widget
//! - **External mode**: Uses xfreerdp - explicit external window mode
//!
//! Both modes open FreeRDP in a separate window; the difference is in client selection
//! and user expectations.

pub mod buffer;
pub mod detect;
pub mod launcher;
pub mod thread;
pub mod types;
pub mod ui;

mod autotype;
mod clipboard;
mod connection;
mod drawing;
mod ephemeral_args;
pub mod file_dnd;
mod input;
mod resize;

// Re-export types for external use
pub use buffer::{CairoBackedBuffer, PixelBuffer, WaylandSurfaceHandle};
pub use file_dnd::{FileDndCircuitBreaker, LocalFileInfo};
pub use launcher::SafeFreeRdpLauncher;
pub use thread::FreeRdpThread;
#[cfg(feature = "rdp-embedded")]
pub use thread::{ClipboardFileTransfer, FileDownloadState};
pub use types::{
    EmbeddedRdpError, EmbeddedSharedFolder, FreeRdpThreadState, RdpCommand, RdpConfig,
    RdpConnectionState, RdpEvent,
};

use types::{ErrorCallback, FallbackCallback, StateCallback};

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, DrawingArea, Label, Orientation};
use std::cell::RefCell;
use std::process::Child;
use std::rc::Rc;

use crate::i18n::i18n;

#[cfg(feature = "rdp-embedded")]
use rustconn_core::rdp_client::RdpClientCommand;

/// Inter-character delay when autotyping a command into the Windows Run dialog.
///
/// Matches the previous scancode path (~30 ms/key) — slow enough that the
/// remote Run dialog registers every character reliably over a busy session.
#[cfg(feature = "rdp-embedded")]
const RUN_DIALOG_AUTOTYPE_DELAY_MS: u32 = 30;

/// Delay before the first character is autotyped into the Run dialog.
///
/// Win+R needs time to open the dialog before input is accepted; 300 ms is
/// comfortably above the ~200 ms a typical server takes to show it.
#[cfg(feature = "rdp-embedded")]
const RUN_DIALOG_OPEN_DELAY_MS: u32 = 300;

/// Pixel slack for treating a framebuffer and the drawing area as the same size.
///
/// Absorbs the ≤1 px even-rounding residual from [`round_rdp_desktop`] plus
/// sub-pixel scale-factor rounding. Applied loosely — device pixels in the
/// post-connect snap check, CSS pixels in the 1:1 blit — the exact unit is
/// immaterial at this magnitude.
const DESKTOP_MATCH_SLACK_PX: u32 = 4;

/// RDP desktop resolution ceiling in device pixels (matches the initial-connect
/// clamp). Stays under the MS-RDPEDISP 8192 px hard limit: a server rejects a
/// `DISPLAYCONTROL_MONITOR_LAYOUT` whose width or height exceeds 8192.
#[cfg(feature = "rdp-embedded")]
const RDP_MAX_DESKTOP_W: u32 = 7680;
/// Height counterpart of [`RDP_MAX_DESKTOP_W`].
#[cfg(feature = "rdp-embedded")]
const RDP_MAX_DESKTOP_H: u32 = 4320;

/// Rounds a measured device-pixel desktop size into a valid RDP resize request.
///
/// Both dimensions are forced even and clamped to [`RDP_MAX_DESKTOP_W`] ×
/// [`RDP_MAX_DESKTOP_H`]. MS-RDPEDISP mandates an even width; an even height is
/// a safe superset that keeps RemoteFX/H.264 tiling artifact-free — the old
/// "multiple of 4" rule was over-conservative and cost up to 3 px of edge.
/// Callers must still reject a sub-640×480 result (widget mid-layout).
#[cfg(feature = "rdp-embedded")]
const fn round_rdp_desktop(width: u32, height: u32) -> (u32, u32) {
    let w = width & !1;
    let h = height & !1;
    (
        if w > RDP_MAX_DESKTOP_W {
            RDP_MAX_DESKTOP_W
        } else {
            w
        },
        if h > RDP_MAX_DESKTOP_H {
            RDP_MAX_DESKTOP_H
        } else {
            h
        },
    )
}

/// Launches `command` through the Windows Run dialog, layout-independently.
///
/// Opens Run with the Win+R scancode hotkey, types `command` via Unicode
/// autotype (`TS_UNICODE_KEYBOARD_EVENT`, immune to the remote keyboard layout
/// — issue #184), then presses Enter. The three commands are processed in order
/// by the client's single command loop.
///
/// # Errors
/// Returns `SendError` if the IronRDP command channel is closed.
#[cfg(feature = "rdp-embedded")]
fn send_run_dialog_command(
    sender: &std::sync::mpsc::Sender<RdpClientCommand>,
    command: &str,
) -> Result<(), std::sync::mpsc::SendError<RdpClientCommand>> {
    sender.send(RdpClientCommand::SendKeySequence {
        keys: rustconn_core::build_open_run_dialog(),
    })?;
    sender.send(RdpClientCommand::AutotypeText {
        text: command.to_string(),
        inter_char_delay_ms: RUN_DIALOG_AUTOTYPE_DELAY_MS,
        initial_delay_ms: RUN_DIALOG_OPEN_DELAY_MS,
    })?;
    sender.send(RdpClientCommand::SendKeySequence {
        keys: rustconn_core::build_enter_sequence(),
    })
}

/// Invokes a callback stored in a `RefCell<Option<T>>` using the take-invoke-restore
/// pattern. This prevents `BorrowMutError` panics when the callback re-enters and
/// borrows the same cell.
fn with_callback<T>(cell: &RefCell<Option<T>>, f: impl FnOnce(&T)) {
    let cb = cell.borrow_mut().take();
    if let Some(ref callback) = cb {
        f(callback);
        *cell.borrow_mut() = cb;
    }
}

/// Embedded RDP widget using Wayland subsurface
///
/// This widget provides native RDP session embedding within GTK4 applications.
/// It uses a `DrawingArea` for rendering and integrates with FreeRDP for
/// protocol handling.
///
/// # Features
///
/// - Native Wayland subsurface integration
/// - FreeRDP frame capture via EndPaint callback
/// - Keyboard and mouse input forwarding
/// - Dynamic resolution changes on resize
/// - Automatic fallback to external xfreerdp
///
/// # Example
///
/// ```ignore
/// use rustconn::embedded_rdp::{EmbeddedRdpWidget, RdpConfig};
///
/// let widget = EmbeddedRdpWidget::new();
///
/// // Configure connection
/// let config = RdpConfig::new("192.168.1.100")
///     .with_username("admin")
///     .with_resolution(1920, 1080);
///
/// // Connect
/// widget.connect(&config)?;
/// ```
pub struct EmbeddedRdpWidget {
    /// Main container widget
    container: GtkBox,
    /// Toolbar with Ctrl+Alt+Del button
    toolbar: GtkBox,
    /// Status label for reconnect indicator
    status_label: Label,
    /// Copy button
    copy_button: Button,
    /// Paste button
    paste_button: Button,
    /// Ctrl+Alt+Del button
    ctrl_alt_del_button: Button,
    /// Separator between buttons
    separator: gtk4::Separator,
    /// Drawing area for rendering RDP frames
    drawing_area: DrawingArea,
    /// Wayland surface handle
    wl_surface: Rc<RefCell<WaylandSurfaceHandle>>,
    /// Pixel buffer for frame data
    pixel_buffer: Rc<RefCell<PixelBuffer>>,
    /// Persistent Cairo-backed pixel buffer for zero-copy rendering.
    /// Used by IronRDP embedded mode to avoid 33MB copies per frame at 4K.
    cairo_buffer: Rc<RefCell<CairoBackedBuffer>>,
    /// Current connection state
    state: Rc<RefCell<RdpConnectionState>>,
    /// Current configuration
    config: Rc<RefCell<Option<RdpConfig>>>,
    /// FreeRDP child process (for external mode)
    process: Rc<RefCell<Option<Child>>>,
    /// FreeRDP thread wrapper for embedded mode
    freerdp_thread: Rc<RefCell<Option<FreeRdpThread>>>,
    /// IronRDP command sender for embedded mode
    #[cfg(feature = "rdp-embedded")]
    ironrdp_command_tx: Rc<RefCell<Option<std::sync::mpsc::Sender<RdpClientCommand>>>>,
    /// Whether using embedded mode (wlfreerdp) or external mode (xfreerdp)
    is_embedded: Rc<RefCell<bool>>,
    /// Whether using IronRDP (true) or FreeRDP (false) for embedded mode
    is_ironrdp: Rc<RefCell<bool>>,
    /// Current widget width
    width: Rc<RefCell<u32>>,
    /// Current widget height
    height: Rc<RefCell<u32>>,
    /// RDP server framebuffer width (for coordinate transformation)
    rdp_width: Rc<RefCell<u32>>,
    /// RDP server framebuffer height (for coordinate transformation)
    rdp_height: Rc<RefCell<u32>>,
    /// State change callback
    on_state_changed: Rc<RefCell<Option<StateCallback>>>,
    /// Error callback
    on_error: Rc<RefCell<Option<ErrorCallback>>>,
    /// Fallback notification callback
    on_fallback: Rc<RefCell<Option<FallbackCallback>>>,
    /// Reconnect callback
    on_reconnect: Rc<RefCell<Option<Box<dyn Fn() + 'static>>>>,
    /// Reconnect banner (shown when disconnected, at bottom of container)
    reconnect_banner: GtkBox,
    /// Reconnect button inside the banner
    reconnect_button: Button,
    /// Reconnect timer source ID for debounced resize reconnect
    reconnect_timer: Rc<RefCell<Option<glib::SourceId>>>,
    /// Remote clipboard text (received from server via CLIPRDR)
    remote_clipboard_text: Rc<RefCell<Option<String>>>,
    /// Available clipboard formats from server
    remote_clipboard_formats: Rc<RefCell<Vec<rustconn_core::ClipboardFormatInfo>>>,
    /// Audio player for RDP audio redirection
    #[cfg(feature = "rdp-audio")]
    audio_player: Rc<RefCell<Option<crate::audio::RdpAudioPlayer>>>,
    /// Clipboard file transfer state
    #[cfg(feature = "rdp-embedded")]
    file_transfer: Rc<RefCell<ClipboardFileTransfer>>,
    /// Save Files button (shown when files available on remote clipboard)
    #[cfg(feature = "rdp-embedded")]
    save_files_button: Button,
    /// File transfer progress callback
    #[cfg(feature = "rdp-embedded")]
    on_file_progress: Rc<RefCell<Option<Box<dyn Fn(f64, &str) + 'static>>>>,
    /// File transfer complete callback
    #[cfg(feature = "rdp-embedded")]
    on_file_complete: Rc<RefCell<Option<Box<dyn Fn(usize, &str) + 'static>>>>,
    /// Connection generation counter to track stale callbacks
    /// Incremented on each connect() call to invalidate old polling loops
    connection_generation: Rc<RefCell<u64>>,
    /// Unique widget ID for debugging
    widget_id: u64,
    /// Signal handler ID for the drawing area resize handler
    resize_handler_id: Rc<RefCell<Option<glib::SignalHandlerId>>>,
    /// Signal handler ID for local clipboard change monitoring (Phase 3)
    #[cfg(feature = "rdp-embedded")]
    clipboard_handler_id: Rc<RefCell<Option<glib::SignalHandlerId>>>,
    /// Flag to suppress clipboard change events when we set the clipboard
    /// ourselves (Copy button or Phase 2 auto-sync), preventing feedback loops.
    clipboard_sync_suppressed: Rc<RefCell<bool>>,
    /// Mouse jiggler timer source ID (sends periodic mouse moves to prevent idle disconnect)
    jiggler_timer: Rc<RefCell<Option<glib::SourceId>>>,
    /// Circuit breaker for file drag-and-drop (auto-disables after repeated failures)
    file_dnd_circuit_breaker: Rc<RefCell<file_dnd::FileDndCircuitBreaker>>,
    /// Toast overlay for file DnD notifications (set via `set_toast_overlay`)
    toast_overlay: Rc<RefCell<Option<libadwaita::ToastOverlay>>>,
    /// Scripts dropdown button (kept for dynamic menu updates with user snippets)
    scripts_button: gtk4::MenuButton,
}

/// String extraction for xgettext — labels defined in `rustconn-core` as
/// `&'static str` constants that are passed to `i18n()` at runtime.
/// xgettext cannot extract them from the loop, so we list them here as literals.
/// This function is never called; it exists solely for `po/update-pot.sh`.
#[expect(
    unreachable_code,
    reason = "kept alive for GTK widget lifecycle / future API exposure"
)]
fn _extract_builtin_labels() {
    return;
    // Quick Actions labels
    i18n("Task Manager");
    i18n("Settings");
    i18n("Registry Editor");
    i18n("Device Manager");
    i18n("Event Viewer");
    i18n("Services");
    i18n("Disk Management");
    i18n("Resource Monitor");
    i18n("Computer Management");
    // ScriptDelivery display names (defined in rustconn-core)
    i18n("Auto");
    i18n("Clipboard (fast)");
    i18n("Autotype (reliable)");
    // Shell Launchers section header
    i18n("Shell Launchers");
    // Scripts section header
    i18n("Scripts");
    // Shell launcher labels
    i18n("PowerShell (Admin)");
    i18n("CMD (Admin)");
}

/// Shared handles the mouse jiggler needs to inject keep-alive input on a timer.
///
/// Bundled into one cloneable struct so the jiggler can be armed from both
/// [`EmbeddedRdpWidget::set_state`] **and** the connection-established event
/// handlers in [`connection`], which set the state directly and never route
/// through `set_state`. That gap is why #185 stayed broken even after the
/// keystroke fix: the timer was only ever armed from `set_state`, which embedded
/// mode skips on connect, so `start_jiggler` was never reached.
///
/// [`connection`]: self::connection
// ponytail: 7 shared Rc handles bundled rather than passed individually; if the
// jiggler ever grows real state, promote this into a proper controller object.
#[derive(Clone)]
struct JigglerHandles {
    timer: Rc<RefCell<Option<glib::SourceId>>>,
    state: Rc<RefCell<RdpConnectionState>>,
    is_ironrdp: Rc<RefCell<bool>>,
    #[cfg(feature = "rdp-embedded")]
    ironrdp_tx: Rc<RefCell<Option<std::sync::mpsc::Sender<RdpClientCommand>>>>,
    freerdp_thread: Rc<RefCell<Option<FreeRdpThread>>>,
    rdp_width: Rc<RefCell<u32>>,
    rdp_height: Rc<RefCell<u32>>,
}

impl JigglerHandles {
    /// Arms the mouse-jiggler timer, replacing any timer already running.
    ///
    /// Sends a tiny ±1px mouse move **and** a no-op Scroll Lock keystroke every
    /// `interval_secs` seconds (clamped to 10–600). The keystroke is required
    /// because Windows does not reset its workstation lock / screensaver timer
    /// on RDP-injected pointer motion alone ([#185]); Scroll Lock is
    /// layout-independent, produces no character, and a make/break/make/break
    /// sequence leaves the remote toggle state untouched.
    ///
    /// [#185]: https://github.com/totoshko88/RustConn/issues/185
    fn start(&self, interval_secs: u32) {
        self.stop();

        let interval = interval_secs.clamp(10, 600);
        let state = self.state.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        #[cfg(feature = "rdp-embedded")]
        let ironrdp_tx = self.ironrdp_tx.clone();
        let freerdp_thread = self.freerdp_thread.clone();
        let rdp_width = self.rdp_width.clone();
        let rdp_height = self.rdp_height.clone();
        // Toggle between +1 and -1 so the cursor oscillates in place
        let jiggle_toggle = Rc::new(std::cell::Cell::new(false));

        let source_id = glib::timeout_add_seconds_local(interval, move || {
            // GDK keyval 0xFF14 / scancode 0x46 (set 1) are layout-independent.
            const SCROLL_LOCK_KEYVAL: u32 = 0xFF14;

            if *state.borrow() != RdpConnectionState::Connected {
                return glib::ControlFlow::Break;
            }

            // Use center as a safe reference point but only move ±1px
            let cx = *rdp_width.borrow() / 2;
            let cy = *rdp_height.borrow() / 2;
            let toggle = jiggle_toggle.get();
            jiggle_toggle.set(!toggle);
            let offset: i32 = if toggle { 1 } else { -1 };

            let using_ironrdp = *is_ironrdp.borrow();
            if using_ironrdp {
                #[cfg(feature = "rdp-embedded")]
                if let Some(ref tx) = *ironrdp_tx.borrow() {
                    const SCROLL_LOCK_SCANCODE: u16 = 0x46;
                    let _ = tx.send(RdpClientCommand::PointerEvent {
                        x: (cx as i32 + offset).max(0) as u16,
                        y: cy as u16,
                        buttons: 0,
                    });
                    for pressed in [true, false, true, false] {
                        let _ = tx.send(RdpClientCommand::KeyEvent {
                            scancode: SCROLL_LOCK_SCANCODE,
                            pressed,
                            extended: false,
                        });
                    }
                }
            } else if let Some(ref thread) = *freerdp_thread.borrow() {
                let _ = thread.send_command(RdpCommand::MouseEvent {
                    x: cx as i32 + offset,
                    y: cy as i32,
                    button: 0,
                    pressed: false,
                });
                for pressed in [true, false, true, false] {
                    let _ = thread.send_command(RdpCommand::KeyEvent {
                        keyval: SCROLL_LOCK_KEYVAL,
                        pressed,
                    });
                }
            }

            tracing::trace!("Mouse jiggler tick");
            glib::ControlFlow::Continue
        });

        *self.timer.borrow_mut() = Some(source_id);
        tracing::debug!(interval_secs = interval, "Mouse jiggler started");
    }

    /// Stops the mouse-jiggler timer if one is running.
    fn stop(&self) {
        if let Some(source_id) = self.timer.borrow_mut().take() {
            source_id.remove();
            tracing::debug!("Mouse jiggler stopped");
        }
    }
}

impl EmbeddedRdpWidget {
    /// Creates a new embedded RDP widget
    #[must_use]
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        // Create toolbar with clipboard and Ctrl+Alt+Del buttons (right-aligned like VNC)
        let toolbar = GtkBox::new(Orientation::Horizontal, 4);
        toolbar.add_css_class("embedded-toolbar");
        toolbar.set_margin_start(6);
        toolbar.set_margin_end(6);
        toolbar.set_margin_top(6);
        toolbar.set_margin_bottom(6);
        // Fill the width so the leading "fit resolution" button can sit in the
        // left corner while the clipboard/admin buttons stay right-aligned.
        toolbar.set_hexpand(true);
        toolbar.set_halign(gtk4::Align::Fill);

        // Fit-resolution button — left-aligned. Re-requests the RDP resolution
        // to match the current window size (same effect as resizing the window),
        // covering servers without dynamic resolution where the session did not
        // fill the window after connecting.
        let fit_resolution_button = Button::from_icon_name("video-display-symbolic");
        fit_resolution_button.add_css_class("flat");
        fit_resolution_button.set_tooltip_text(Some(&i18n("Fit resolution to window")));
        fit_resolution_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Fit resolution to window",
        ))]);
        toolbar.append(&fit_resolution_button);

        // Expanding spacer pushes the remaining buttons to the right edge
        let toolbar_spacer = GtkBox::new(Orientation::Horizontal, 0);
        toolbar_spacer.set_hexpand(true);
        toolbar.append(&toolbar_spacer);

        // Status label for reconnect indicator (hidden by default)
        let status_label = Label::new(None);
        status_label.set_visible(false);
        status_label.set_margin_end(12);
        status_label.add_css_class("dim-label");
        toolbar.append(&status_label);

        // Copy button - copies remote clipboard to local (enabled when data available)
        let copy_button = Button::with_label(&i18n("Copy"));
        copy_button.set_tooltip_text(Some(&i18n(
            "Copy remote clipboard to local (waiting for remote data...)",
        )));
        copy_button.set_sensitive(false); // Disabled until we receive clipboard data
        toolbar.append(&copy_button);

        // Paste button - pastes from local clipboard to remote
        let paste_button = Button::with_label(&i18n("Paste"));
        paste_button.set_tooltip_text(Some(&i18n("Paste from local clipboard to remote session")));
        toolbar.append(&paste_button);

        // Autotype dropdown — consolidates "Type Clipboard" and "Type Text…" (GNOME HIG)
        let autotype_button = gtk4::MenuButton::new();
        autotype_button.set_label(&i18n("Autotype"));
        autotype_button.set_tooltip_text(Some(&i18n(
            "Send text as keystrokes (bypasses paste restrictions)",
        )));
        autotype_button.add_css_class("flat");
        autotype_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Autotype"))]);
        {
            let menu = gtk4::gio::Menu::new();
            menu.append(
                Some(&i18n("Type Clipboard")),
                Some("rdp-autotype.type-clipboard"),
            );
            menu.append(Some(&i18n("Type Text…")), Some("rdp-autotype.type-text"));
            autotype_button.set_menu_model(Some(&menu));
        }
        toolbar.append(&autotype_button);

        // Hidden buttons for autotype actions (signal handlers attached later)
        let type_clipboard_button = Button::new();
        type_clipboard_button.set_visible(false);
        let type_text_button = Button::new();
        type_text_button.set_visible(false);

        // Scripts dropdown menu — run PowerShell scripts via clipboard+paste
        let scripts_button = gtk4::MenuButton::new();
        scripts_button.set_icon_name("utilities-terminal-symbolic");
        scripts_button.set_tooltip_text(Some(&i18n("Run PowerShell script on remote")));
        scripts_button.add_css_class("flat");
        scripts_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Run script on remote",
        ))]);
        {
            // Initially empty; shell launchers + user snippets added via update_scripts_menu()
            let menu = gtk4::gio::Menu::new();
            scripts_button.set_menu_model(Some(&menu));
        }
        toolbar.append(&scripts_button);

        // Separator
        let separator = gtk4::Separator::new(Orientation::Vertical);
        separator.set_margin_start(6);
        separator.set_margin_end(6);
        toolbar.append(&separator);

        let ctrl_alt_del_button = Button::with_label(&i18n("Ctrl+Alt+Del"));
        ctrl_alt_del_button.add_css_class("suggested-action"); // Blue button style
        ctrl_alt_del_button.set_tooltip_text(Some(&i18n("Send Ctrl+Alt+Del to remote session")));
        toolbar.append(&ctrl_alt_del_button);

        // Windows Admin quick actions dropdown menu
        let quick_actions_button = gtk4::MenuButton::new();
        quick_actions_button.set_icon_name("view-more-symbolic");
        quick_actions_button.set_tooltip_text(Some(&i18n("Windows admin tools")));
        quick_actions_button.add_css_class("flat");
        quick_actions_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Windows admin tools",
        ))]);
        {
            let menu = gtk4::gio::Menu::new();
            let mut current_group = rustconn_core::QUICK_ACTIONS.first().map_or(0, |a| a.group);
            let mut section = gtk4::gio::Menu::new();
            for action in rustconn_core::QUICK_ACTIONS {
                if action.group != current_group {
                    menu.append_section(None, &section);
                    section = gtk4::gio::Menu::new();
                    current_group = action.group;
                }
                section.append(
                    Some(&i18n(action.label)),
                    Some(&format!("rdp.{}", action.id)),
                );
            }
            menu.append_section(None, &section);
            quick_actions_button.set_menu_model(Some(&menu));
        }
        toolbar.append(&quick_actions_button);

        // Save Files button (shown when files available on remote clipboard)
        #[cfg(feature = "rdp-embedded")]
        let save_files_button = Button::with_label(&i18n("Save Files"));
        #[cfg(feature = "rdp-embedded")]
        {
            save_files_button.set_tooltip_text(Some(&i18n("Save files from remote clipboard")));
            save_files_button.set_visible(false); // Hidden until files available
            toolbar.append(&save_files_button);
        }

        // Hide toolbar initially (show when connected)
        toolbar.set_visible(false);

        container.append(&toolbar);

        let drawing_area = DrawingArea::new();
        drawing_area.set_hexpand(true);
        drawing_area.set_vexpand(true);
        // Explicitly set content size to 0 so the DrawingArea does not report
        // a natural size based on the RDP framebuffer dimensions. Without this,
        // AdwTabOverview warns "exceeds AdwApplicationWindow size" because it
        // measures the child's natural size for thumbnail generation.
        drawing_area.set_content_width(0);
        drawing_area.set_content_height(0);
        // Don't set fixed content size - let the widget expand to fill available space
        // The actual RDP resolution will be set when connect() is called
        drawing_area.set_can_focus(true);
        drawing_area.set_focusable(true);

        container.append(&drawing_area);

        // Reconnect banner (shown when disconnected, at bottom like VTE sessions)
        let reconnect_banner = GtkBox::new(Orientation::Horizontal, 6);
        reconnect_banner.set_margin_start(12);
        reconnect_banner.set_margin_end(12);
        reconnect_banner.set_margin_top(6);
        reconnect_banner.set_margin_bottom(6);
        reconnect_banner.set_halign(gtk4::Align::Center);
        reconnect_banner.set_widget_name("reconnect-banner");
        reconnect_banner.set_visible(false);

        let reconnect_label = Label::new(Some(&i18n("Session disconnected")));
        reconnect_label.add_css_class("dim-label");

        let reconnect_button = Button::with_label(&i18n("Reconnect"));
        reconnect_button.add_css_class("suggested-action");
        reconnect_button.set_tooltip_text(Some(&i18n("Reconnect to this session")));

        reconnect_banner.append(&reconnect_label);
        reconnect_banner.append(&reconnect_button);

        container.append(&reconnect_banner);

        let pixel_buffer = Rc::new(RefCell::new(PixelBuffer::new(1280, 720)));
        let cairo_buffer = Rc::new(RefCell::new(CairoBackedBuffer::new(1280, 720)));
        let state = Rc::new(RefCell::new(RdpConnectionState::Disconnected));
        let width = Rc::new(RefCell::new(1280u32));
        let height = Rc::new(RefCell::new(720u32));
        let rdp_width = Rc::new(RefCell::new(1280u32));
        let rdp_height = Rc::new(RefCell::new(720u32));
        let is_embedded = Rc::new(RefCell::new(false));
        let is_ironrdp = Rc::new(RefCell::new(false));

        #[cfg(feature = "rdp-embedded")]
        let ironrdp_command_tx: Rc<
            RefCell<Option<std::sync::mpsc::Sender<RdpClientCommand>>>,
        > = Rc::new(RefCell::new(None));

        let widget = Self {
            container,
            toolbar,
            status_label,
            copy_button: copy_button.clone(),
            paste_button: paste_button.clone(),
            ctrl_alt_del_button: ctrl_alt_del_button.clone(),
            separator,
            drawing_area,
            wl_surface: Rc::new(RefCell::new(WaylandSurfaceHandle::new())),
            pixel_buffer,
            cairo_buffer,
            state,
            config: Rc::new(RefCell::new(None)),
            process: Rc::new(RefCell::new(None)),
            freerdp_thread: Rc::new(RefCell::new(None)),
            #[cfg(feature = "rdp-embedded")]
            ironrdp_command_tx,
            is_embedded,
            is_ironrdp,
            width,
            height,
            rdp_width,
            rdp_height,
            on_state_changed: Rc::new(RefCell::new(None)),
            on_error: Rc::new(RefCell::new(None)),
            on_fallback: Rc::new(RefCell::new(None)),
            on_reconnect: Rc::new(RefCell::new(None)),
            reconnect_banner,
            reconnect_button,
            reconnect_timer: Rc::new(RefCell::new(None)),
            remote_clipboard_text: Rc::new(RefCell::new(None)),
            remote_clipboard_formats: Rc::new(RefCell::new(Vec::new())),
            #[cfg(feature = "rdp-audio")]
            audio_player: Rc::new(RefCell::new(None)),
            #[cfg(feature = "rdp-embedded")]
            file_transfer: Rc::new(RefCell::new(ClipboardFileTransfer::new())),
            #[cfg(feature = "rdp-embedded")]
            save_files_button: save_files_button.clone(),
            #[cfg(feature = "rdp-embedded")]
            on_file_progress: Rc::new(RefCell::new(None)),
            #[cfg(feature = "rdp-embedded")]
            on_file_complete: Rc::new(RefCell::new(None)),
            connection_generation: Rc::new(RefCell::new(0)),
            widget_id: {
                use std::sync::atomic::{AtomicU64, Ordering};
                static WIDGET_COUNTER: AtomicU64 = AtomicU64::new(1);
                WIDGET_COUNTER.fetch_add(1, Ordering::SeqCst)
            },
            resize_handler_id: Rc::new(RefCell::new(None)),
            #[cfg(feature = "rdp-embedded")]
            clipboard_handler_id: Rc::new(RefCell::new(None)),
            clipboard_sync_suppressed: Rc::new(RefCell::new(false)),
            jiggler_timer: Rc::new(RefCell::new(None)),
            file_dnd_circuit_breaker: Rc::new(RefCell::new(file_dnd::FileDndCircuitBreaker::new())),
            toast_overlay: Rc::new(RefCell::new(None)),
            scripts_button: scripts_button.clone(),
        };

        widget.setup_drawing();
        widget.setup_input_handlers();
        widget.setup_resize_handler();
        widget.setup_fit_resolution_button(&fit_resolution_button);
        widget.setup_clipboard_buttons(&copy_button, &paste_button);
        widget.setup_ctrl_alt_del_button(&ctrl_alt_del_button);
        #[cfg(feature = "rdp-embedded")]
        widget.setup_autotype_clipboard_button(&type_clipboard_button);
        #[cfg(feature = "rdp-embedded")]
        widget.setup_autotype_dialog_button(&type_text_button);
        widget.setup_autotype_actions(&type_clipboard_button, &type_text_button);
        widget.setup_quick_actions();
        widget.setup_script_actions();
        widget.setup_reconnect_button();
        widget.setup_visibility_handler();
        #[cfg(feature = "rdp-embedded")]
        widget.setup_save_files_button(&save_files_button);
        widget.setup_file_dnd();

        widget
    }

    /// Sets up file drag-and-drop on the RDP drawing area.
    ///
    /// When files are dropped, they are announced to the RDP server via the
    /// CLIPRDR file clipboard channel. Includes circuit breaker logic that
    /// auto-disables after 3 consecutive failures with a toast notification.
    fn setup_file_dnd(&self) {
        #[cfg(feature = "rdp-embedded")]
        {
            let circuit_breaker = self.file_dnd_circuit_breaker.clone();
            let ironrdp_tx = self.ironrdp_command_tx.clone();
            let state = self.state.clone();
            let is_embedded = self.is_embedded.clone();
            let status_label = self.status_label.clone();
            let cb_for_callback = self.file_dnd_circuit_breaker.clone();
            let toast_overlay_ref = self.toast_overlay.clone();

            file_dnd::setup_rdp_file_drop_target(
                self.drawing_area.upcast_ref::<gtk4::Widget>(),
                circuit_breaker,
                toast_overlay_ref,
                move |files| {
                    let current_state = *state.borrow();
                    let embedded = *is_embedded.borrow();

                    if current_state != RdpConnectionState::Connected || !embedded {
                        tracing::debug!(
                            protocol = "rdp",
                            ?current_state,
                            embedded,
                            "File DnD: not connected or not embedded"
                        );
                        return;
                    }

                    if let Some(ref sender) = *ironrdp_tx.borrow() {
                        // Build FileGroupDescriptorW and announce to server
                        let descriptor = file_dnd::build_file_group_descriptor(&files);

                        // Store file paths in the backend for later file contents requests
                        let paths: Vec<std::path::PathBuf> =
                            files.iter().map(|f| f.path.clone()).collect();
                        let _ = sender.send(RdpClientCommand::StoreLocalFiles { paths });

                        // Store file paths for later on_file_contents_request
                        // and announce CF_HDROP format to server
                        let format_info = rustconn_core::ClipboardFormatInfo::new(
                            rustconn_core::ClipboardFormatInfo::FILE_LIST,
                            Some("FileGroupDescriptorW".to_string()),
                        );

                        // Send clipboard copy announcement with file list format
                        let _ = sender.send(RdpClientCommand::ClipboardCopy(vec![format_info]));

                        // Store the descriptor data as pending copy data
                        let _ = sender.send(RdpClientCommand::ClipboardData {
                            format_id: rustconn_core::ClipboardFormatInfo::FILE_LIST,
                            data: descriptor,
                        });

                        cb_for_callback.borrow_mut().record_success();

                        // Show brief status feedback
                        let file_count = files.len().to_string();
                        status_label.set_text(&crate::i18n::i18n_f(
                            "{} file(s) ready to paste on remote",
                            &[&file_count],
                        ));
                        status_label.set_visible(true);
                        let hide_label = status_label.clone();
                        glib::timeout_add_local_once(
                            std::time::Duration::from_secs(3),
                            move || {
                                hide_label.set_visible(false);
                            },
                        );

                        tracing::info!(
                            protocol = "rdp",
                            file_count = files.len(),
                            "Announced files to RDP server via CLIPRDR"
                        );
                    } else {
                        tracing::warn!(
                            protocol = "rdp",
                            "File DnD: IronRDP command channel not available"
                        );
                        cb_for_callback
                            .borrow_mut()
                            .record_failure("Clipboard channel not ready");
                    }
                },
            );
        }
    }

    /// Returns a reference to the file DnD circuit breaker for external monitoring.
    #[must_use]
    pub fn file_dnd_circuit_breaker(&self) -> &Rc<RefCell<file_dnd::FileDndCircuitBreaker>> {
        &self.file_dnd_circuit_breaker
    }

    /// Sets the toast overlay for file DnD notifications.
    ///
    /// Should be called after the widget is added to the window hierarchy.
    /// The toast overlay is used to show "File drag disabled" messages when
    /// the circuit breaker trips.
    pub fn set_toast_overlay(&self, overlay: libadwaita::ToastOverlay) {
        *self.toast_overlay.borrow_mut() = Some(overlay);
    }

    /// Updates the Scripts dropdown menu with shell launchers and user-defined
    /// Windows-compatible snippets.
    ///
    /// Called after the widget is created, when the application state is available.
    /// Rebuilds the menu to include 4 shell launchers and user snippets that
    /// have `target == Windows` or `target == Any`.
    pub fn update_scripts_menu(&self, user_snippets: &[&rustconn_core::models::Snippet]) {
        use gtk4::gio;

        let menu = gio::Menu::new();

        // Shell Launchers section
        let launchers_section = gio::Menu::new();
        launchers_section.append(Some(&i18n("PowerShell")), Some("rdp-shell.powershell"));
        launchers_section.append(
            Some(&i18n("PowerShell (Admin)")),
            Some("rdp-shell.powershell-admin"),
        );
        launchers_section.append(Some(&i18n("CMD")), Some("rdp-shell.cmd"));
        launchers_section.append(Some(&i18n("CMD (Admin)")), Some("rdp-shell.cmd-admin"));
        menu.append_section(Some(&i18n("Shell Launchers")), &launchers_section);

        // User snippets section (only Windows-compatible ones)
        let windows_snippets: Vec<&rustconn_core::models::Snippet> = user_snippets
            .iter()
            .filter(|s| s.target.is_windows_compatible())
            .copied()
            .collect();

        if !windows_snippets.is_empty() {
            let user_section = gio::Menu::new();
            for snippet in &windows_snippets {
                user_section.append(
                    Some(&snippet.name),
                    Some(&format!("rdp-user-script.{}", snippet.id)),
                );
            }
            menu.append_section(Some(&i18n("Scripts")), &user_section);

            // Register actions for user snippets
            self.register_user_script_actions(&windows_snippets);
        }

        self.scripts_button.set_menu_model(Some(&menu));
    }

    /// Registers GIO actions for user-defined Windows snippets.
    ///
    /// Each snippet gets an action `rdp-user-script.<uuid>` that sends
    /// the snippet command via autotype (Unicode keyboard events) into
    /// the already-open shell, followed by Enter after 500ms.
    fn register_user_script_actions(&self, snippets: &[&rustconn_core::models::Snippet]) {
        use gtk4::gio;

        let action_group = gio::SimpleActionGroup::new();

        for snippet in snippets {
            let action_name = snippet.id.to_string();
            let action = gio::SimpleAction::new(&action_name, None);

            #[cfg(feature = "rdp-embedded")]
            {
                let tx = self.ironrdp_command_tx.clone();
                let script_text = snippet.command.clone();
                let snippet_name = snippet.name.clone();
                let status_label = self.status_label.clone();
                let config = self.config.clone();
                let snippet_delivery = snippet.delivery;

                action.connect_activate(move |_, _| {
                    let Some(ref sender) = *tx.borrow() else {
                        return;
                    };

                    let use_clipboard = match snippet_delivery {
                        rustconn_core::models::ScriptDelivery::Clipboard => true,
                        rustconn_core::models::ScriptDelivery::Autotype => false,
                        rustconn_core::models::ScriptDelivery::Auto => config
                            .borrow()
                            .as_ref()
                            .is_none_or(|c| c.script_paste_via_clipboard),
                    };

                    if use_clipboard {
                        // Fast path: send via clipboard + Ctrl+V (instant)
                        if let Err(e) = sender.send(rustconn_core::RdpClientCommand::ClipboardText(
                            script_text.clone(),
                        )) {
                            tracing::warn!(
                                protocol = "rdp",
                                snippet = %snippet_name,
                                error = %e,
                                "Script clipboard send failed: channel not ready"
                            );
                            status_label.set_text(&i18n("Channel not ready"));
                            status_label.set_visible(true);
                            let hide = status_label.clone();
                            glib::timeout_add_local_once(
                                std::time::Duration::from_secs(3),
                                move || {
                                    hide.set_visible(false);
                                },
                            );
                            return;
                        }

                        // After 150ms (let server process clipboard), send Ctrl+V
                        let tx_paste = tx.clone();
                        let tx_enter = tx.clone();
                        let status = status_label.clone();
                        glib::timeout_add_local_once(
                            std::time::Duration::from_millis(150),
                            move || {
                                if let Some(ref sender) = *tx_paste.borrow() {
                                    // Ctrl+V: paste from clipboard
                                    let keys = vec![
                                        (0x1D_u16, true, false),  // Ctrl down
                                        (0x2F_u16, true, false),  // V down
                                        (0x2F_u16, false, false), // V up
                                        (0x1D_u16, false, false), // Ctrl up
                                    ];
                                    let _ = sender.send(
                                        rustconn_core::RdpClientCommand::SendKeySequence { keys },
                                    );
                                }

                                // After 200ms send Enter to execute
                                glib::timeout_add_local_once(
                                    std::time::Duration::from_millis(200),
                                    move || {
                                        if let Some(ref sender) = *tx_enter.borrow() {
                                            let enter_keys = rustconn_core::build_enter_sequence();
                                            let _ = sender.send(
                                                rustconn_core::RdpClientCommand::SendKeySequence {
                                                    keys: enter_keys,
                                                },
                                            );
                                        }

                                        status.set_text(&i18n("Script sent"));
                                        status.set_visible(true);
                                        let hide = status.clone();
                                        glib::timeout_add_local_once(
                                            std::time::Duration::from_secs(3),
                                            move || {
                                                hide.set_visible(false);
                                            },
                                        );
                                    },
                                );
                            },
                        );
                    } else {
                        // Slow path: character-by-character autotype (reliable fallback)
                        if let Err(e) = sender.send(rustconn_core::RdpClientCommand::AutotypeText {
                            text: script_text.clone(),
                            inter_char_delay_ms: 5,
                            initial_delay_ms: 0,
                        }) {
                            tracing::warn!(
                                protocol = "rdp",
                                snippet = %snippet_name,
                                error = %e,
                                "User script execution aborted: channel not ready"
                            );
                            status_label.set_text(&i18n("Channel not ready"));
                            status_label.set_visible(true);
                            let hide = status_label.clone();
                            glib::timeout_add_local_once(
                                std::time::Duration::from_secs(3),
                                move || {
                                    hide.set_visible(false);
                                },
                            );
                            return;
                        }

                        // After 500ms send Enter to execute
                        let tx_enter = tx.clone();
                        let status = status_label.clone();
                        glib::timeout_add_local_once(
                            std::time::Duration::from_millis(500),
                            move || {
                                if let Some(ref sender) = *tx_enter.borrow() {
                                    let enter_keys = rustconn_core::build_enter_sequence();
                                    let _ = sender.send(
                                        rustconn_core::RdpClientCommand::SendKeySequence {
                                            keys: enter_keys,
                                        },
                                    );
                                }

                                status.set_text(&i18n("Script sent"));
                                status.set_visible(true);
                                let hide = status.clone();
                                glib::timeout_add_local_once(
                                    std::time::Duration::from_secs(3),
                                    move || {
                                        hide.set_visible(false);
                                    },
                                );
                            },
                        );
                    }

                    tracing::info!(
                        protocol = "rdp",
                        snippet = %snippet_name,
                        clipboard = use_clipboard,
                        "User script sent"
                    );
                });
            }

            action_group.add_action(&action);
        }

        self.container
            .insert_action_group("rdp-user-script", Some(&action_group));
    }

    /// Sets up visibility handler to redraw when widget becomes visible again
    /// This fixes the issue where the image disappears when switching tabs
    fn setup_visibility_handler(&self) {
        let drawing_area = self.drawing_area.clone();

        // Redraw when the widget becomes visible (e.g., switching back to this tab)
        self.container.connect_map(move |_| {
            drawing_area.queue_draw();
        });
    }

    /// Sets up the reconnect button click handler
    fn setup_reconnect_button(&self) {
        let on_reconnect = self.on_reconnect.clone();

        self.reconnect_button.connect_clicked(move |_| {
            if let Some(ref callback) = *on_reconnect.borrow() {
                callback();
            }
        });
    }

    /// Registers GIO actions for the Autotype dropdown menu.
    ///
    /// Maps `rdp-autotype.type-clipboard` and `rdp-autotype.type-text` actions
    /// to programmatic clicks on the hidden autotype buttons (which have their
    /// signal handlers already connected via `setup_autotype_*_button`).
    fn setup_autotype_actions(&self, type_clipboard_btn: &Button, type_text_btn: &Button) {
        use gtk4::gio;

        let action_group = gio::SimpleActionGroup::new();

        let clipboard_action = gio::SimpleAction::new("type-clipboard", None);
        let btn = type_clipboard_btn.clone();
        clipboard_action.connect_activate(move |_, _| {
            btn.emit_clicked();
        });
        action_group.add_action(&clipboard_action);

        let text_action = gio::SimpleAction::new("type-text", None);
        let btn = type_text_btn.clone();
        text_action.connect_activate(move |_, _| {
            btn.emit_clicked();
        });
        action_group.add_action(&text_action);

        self.container
            .insert_action_group("rdp-autotype", Some(&action_group));
    }

    /// Sets up the Windows admin quick actions menu.
    ///
    /// Registers a GIO action for each entry in [`rustconn_core::QUICK_ACTIONS`].
    /// When triggered, the action builds the corresponding key sequence and
    /// sends it to the IronRDP client via `SendKeySequence`.
    fn setup_quick_actions(&self) {
        use gtk4::gio;

        let action_group = gio::SimpleActionGroup::new();

        for action_def in rustconn_core::QUICK_ACTIONS {
            let action = gio::SimpleAction::new(action_def.id, None);

            #[cfg(feature = "rdp-embedded")]
            {
                let tx = self.ironrdp_command_tx.clone();
                let action_id = action_def.id;
                action.connect_activate(move |_, _| {
                    let Some(ref sender) = *tx.borrow() else {
                        return;
                    };
                    // Hotkey actions (Task Manager, Settings) are pure modifier
                    // combos — sent as scancodes. Run-dialog actions type their
                    // command via Unicode autotype so they are correct on any
                    // remote keyboard layout (issue #184).
                    let sent = if let Some(keys) = rustconn_core::build_hotkey_sequence(action_id) {
                        sender
                            .send(RdpClientCommand::SendKeySequence { keys })
                            .is_ok()
                    } else if let Some(cmd) = rustconn_core::run_command_for(action_id) {
                        send_run_dialog_command(sender, cmd).is_ok()
                    } else {
                        false
                    };
                    if sent {
                        tracing::info!(
                            protocol = "rdp",
                            action = action_id,
                            "Quick action triggered"
                        );
                    }
                });
            }

            action_group.add_action(&action);
        }

        self.container
            .insert_action_group("rdp", Some(&action_group));
    }

    /// Sets up the shell launcher actions for the Scripts menu.
    ///
    /// Registers 4 GIO actions (`rdp-shell.powershell`, `rdp-shell.powershell-admin`,
    /// `rdp-shell.cmd`, `rdp-shell.cmd-admin`) that open shells via Win+R.
    fn setup_script_actions(&self) {
        use gtk4::gio;

        let action_group = gio::SimpleActionGroup::new();

        // Shell launcher definitions: (action_name, Win+R command)
        let launchers: &[(&str, &str)] = &[
            ("powershell", "powershell"),
            (
                "powershell-admin",
                "powershell -Command \"Start-Process powershell -Verb RunAs\"",
            ),
            ("cmd", "cmd"),
            (
                "cmd-admin",
                "powershell -Command \"Start-Process cmd -Verb RunAs\"",
            ),
        ];

        for &(action_name, command) in launchers {
            let action = gio::SimpleAction::new(action_name, None);

            #[cfg(feature = "rdp-embedded")]
            {
                let tx = self.ironrdp_command_tx.clone();
                let status_label = self.status_label.clone();
                let cmd = command.to_string();

                action.connect_activate(move |_, _| {
                    let Some(ref sender) = *tx.borrow() else {
                        return;
                    };

                    // Open Run, type the command via Unicode autotype (layout
                    // independent — issue #184), then press Enter.
                    if let Err(e) = send_run_dialog_command(sender, &cmd) {
                        tracing::warn!(
                            protocol = "rdp",
                            launcher = cmd.as_str(),
                            error = %e,
                            "Shell launcher aborted: channel not ready"
                        );
                        status_label.set_text(&i18n("Channel not ready"));
                        status_label.set_visible(true);
                        let hide = status_label.clone();
                        glib::timeout_add_local_once(
                            std::time::Duration::from_secs(3),
                            move || {
                                hide.set_visible(false);
                            },
                        );
                        return;
                    }

                    tracing::info!(
                        protocol = "rdp",
                        launcher = cmd.as_str(),
                        "Shell launcher triggered"
                    );
                });
            }

            action_group.add_action(&action);
        }

        self.container
            .insert_action_group("rdp-shell", Some(&action_group));
    }

    /// Sets up the Save Files button click handler for clipboard file transfer
    #[cfg(feature = "rdp-embedded")]
    fn setup_save_files_button(&self, button: &Button) {
        let file_transfer = self.file_transfer.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();
        let on_progress = self.on_file_progress.clone();
        let on_complete = self.on_file_complete.clone();
        let status_label = self.status_label.clone();
        let save_btn = button.clone();

        button.connect_clicked(move |_| {
            let files = file_transfer.borrow().available_files.clone();
            if files.is_empty() {
                return;
            }

            // Show file chooser dialog for target directory
            let dialog = gtk4::FileDialog::builder()
                .title(i18n("Select folder to save files"))
                .modal(true)
                .build();

            let file_transfer_clone = file_transfer.clone();
            let ironrdp_tx_clone = ironrdp_tx.clone();
            let on_progress_clone = on_progress.clone();
            let _on_complete_clone = on_complete.clone();
            let status_label_clone = status_label.clone();
            let save_btn_clone = save_btn.clone();
            let files_clone = files.clone();

            dialog.select_folder(
                None::<&gtk4::Window>,
                None::<&gtk4::gio::Cancellable>,
                move |result| {
                    if let Ok(folder) = result
                        && let Some(path) = folder.path()
                    {
                        // Set target directory and start downloads
                        {
                            let mut transfer = file_transfer_clone.borrow_mut();
                            transfer.target_directory = Some(path.clone());
                            transfer.total_files = files_clone.len();
                            transfer.completed_count = 0;
                        }

                        // Disable button during transfer
                        save_btn_clone.set_sensitive(false);
                        save_btn_clone.set_label(&i18n("Downloading..."));

                        // Request file contents for each file
                        if let Some(ref sender) = *ironrdp_tx_clone.borrow() {
                            for (idx, file) in files_clone.iter().enumerate() {
                                let stream_id = {
                                    let mut transfer = file_transfer_clone.borrow_mut();
                                    transfer.start_download(idx as u32)
                                };

                                if let Some(sid) = stream_id {
                                    // First request size, then data
                                    let _ = sender.send(RdpClientCommand::RequestFileContents {
                                        stream_id: sid,
                                        file_index: file.index,
                                        request_size: true,
                                        offset: 0,
                                        length: 0,
                                    });

                                    // Then request actual data
                                    let _ = sender.send(RdpClientCommand::RequestFileContents {
                                        stream_id: sid,
                                        file_index: file.index,
                                        request_size: false,
                                        offset: 0,
                                        length: u32::MAX, // Request all data
                                    });
                                }
                            }
                        }

                        // Show progress
                        status_label_clone.set_text(&i18n("Downloading files..."));
                        status_label_clone.set_visible(true);

                        if let Some(ref callback) = *on_progress_clone.borrow() {
                            callback(0.0, "Starting download...");
                        }
                    }
                },
            );
        });
    }

    /// Connects a callback for file transfer progress updates
    #[cfg(feature = "rdp-embedded")]
    pub fn connect_file_progress<F>(&self, callback: F)
    where
        F: Fn(f64, &str) + 'static,
    {
        *self.on_file_progress.borrow_mut() = Some(Box::new(callback));
    }

    /// Connects a callback for file transfer completion
    #[cfg(feature = "rdp-embedded")]
    pub fn connect_file_complete<F>(&self, callback: F)
    where
        F: Fn(usize, &str) + 'static,
    {
        *self.on_file_complete.borrow_mut() = Some(Box::new(callback));
    }

    /// Connects a callback for reconnect button clicks
    ///
    /// The callback is invoked when the user clicks the Reconnect button
    /// after a session has disconnected or encountered an error.
    pub fn connect_reconnect<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        *self.on_reconnect.borrow_mut() = Some(Box::new(callback));
    }

    /// Returns the main container widget
    #[must_use]
    pub const fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Returns the drawing area widget
    #[must_use]
    pub const fn drawing_area(&self) -> &DrawingArea {
        &self.drawing_area
    }

    /// Queues a redraw of the drawing area
    pub fn queue_draw(&self) {
        self.drawing_area.queue_draw();
    }

    /// Returns the current connection state
    #[must_use]
    pub fn state(&self) -> RdpConnectionState {
        *self.state.borrow()
    }

    /// Returns whether the widget is using embedded mode
    #[must_use]
    pub fn is_embedded(&self) -> bool {
        *self.is_embedded.borrow()
    }

    /// Returns the current width
    #[must_use]
    pub fn width(&self) -> u32 {
        *self.width.borrow()
    }

    /// Returns the current height
    #[must_use]
    pub fn height(&self) -> u32 {
        *self.height.borrow()
    }

    /// Connects a callback for state changes
    pub fn connect_state_changed<F>(&self, callback: F)
    where
        F: Fn(RdpConnectionState) + 'static,
    {
        let reconnect_banner = self.reconnect_banner.clone();
        let copy_button = self.copy_button.clone();
        let paste_button = self.paste_button.clone();
        let ctrl_alt_del_button = self.ctrl_alt_del_button.clone();
        let separator = self.separator.clone();
        let toolbar = self.toolbar.clone();

        *self.on_state_changed.borrow_mut() = Some(Box::new(move |state| {
            // Update button visibility based on state
            let show_reconnect = matches!(
                state,
                RdpConnectionState::Disconnected | RdpConnectionState::Error
            );

            // Show/hide reconnect banner at bottom
            reconnect_banner.set_visible(show_reconnect);

            // When disconnected, hide toolbar buttons
            copy_button.set_visible(!show_reconnect);
            paste_button.set_visible(!show_reconnect);
            ctrl_alt_del_button.set_visible(!show_reconnect);
            separator.set_visible(!show_reconnect);

            // Hide toolbar when disconnected (no reconnect button there anymore)
            if show_reconnect {
                toolbar.set_visible(false);
            }
            // Call the user's callback
            callback(state);
        }));
    }

    /// Connects a callback for errors
    pub fn connect_error<F>(&self, callback: F)
    where
        F: Fn(&str) + 'static,
    {
        *self.on_error.borrow_mut() = Some(Box::new(callback));
    }

    /// Connects a callback for fallback notifications
    ///
    /// This callback is invoked when embedded mode fails and the system
    /// falls back to external xfreerdp mode.
    pub fn connect_fallback<F>(&self, callback: F)
    where
        F: Fn(&str) + 'static,
    {
        *self.on_fallback.borrow_mut() = Some(Box::new(callback));
    }

    /// Reports a fallback and notifies listeners
    fn report_fallback(&self, message: &str) {
        with_callback(&self.on_fallback, |cb| cb(message));
    }

    /// Sets the connection state and notifies listeners
    fn set_state(&self, new_state: RdpConnectionState) {
        *self.state.borrow_mut() = new_state;
        self.drawing_area.queue_draw();

        // Auto-manage jiggler based on connection state
        match new_state {
            RdpConnectionState::Connected => {
                // Check if jiggler is enabled in config
                if let Some(ref config) = *self.config.borrow()
                    && config.jiggler_enabled
                {
                    self.start_jiggler(config.jiggler_interval_secs);
                }
            }
            RdpConnectionState::Disconnected | RdpConnectionState::Error => {
                self.stop_jiggler();
            }
            RdpConnectionState::Connecting => {}
        }

        with_callback(&self.on_state_changed, |cb| cb(new_state));
    }

    /// Bundles the shared handles the mouse jiggler timer operates on.
    ///
    /// Lets the jiggler be armed/disarmed from the connection event handlers
    /// (which set the state directly, bypassing `set_state`) as well as from
    /// `set_state` itself — see [`JigglerHandles`] and #185.
    fn jiggler_handles(&self) -> JigglerHandles {
        JigglerHandles {
            timer: self.jiggler_timer.clone(),
            state: self.state.clone(),
            is_ironrdp: self.is_ironrdp.clone(),
            #[cfg(feature = "rdp-embedded")]
            ironrdp_tx: self.ironrdp_command_tx.clone(),
            freerdp_thread: self.freerdp_thread.clone(),
            rdp_width: self.rdp_width.clone(),
            rdp_height: self.rdp_height.clone(),
        }
    }

    /// Starts the mouse jiggler timer. See [`JigglerHandles::start`].
    pub fn start_jiggler(&self, interval_secs: u32) {
        self.jiggler_handles().start(interval_secs);
    }

    /// Stops the mouse jiggler timer. See [`JigglerHandles::stop`].
    pub fn stop_jiggler(&self) {
        self.jiggler_handles().stop();
    }

    /// Reports an error and notifies listeners
    fn report_error(&self, message: &str) {
        self.set_state(RdpConnectionState::Error);

        with_callback(&self.on_error, |cb| cb(message));
    }
}

impl Default for EmbeddedRdpWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for EmbeddedRdpWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddedRdpWidget")
            .field("state", &self.state.borrow())
            .field("is_embedded", &self.is_embedded.borrow())
            .field("width", &self.width.borrow())
            .field("height", &self.height.borrow())
            .finish_non_exhaustive()
    }
}

// ============================================================================
// Paint callbacks and direct input API
// ============================================================================

impl EmbeddedRdpWidget {
    /// Handles FreeRDP BeginPaint callback
    ///
    /// This is called by FreeRDP before rendering a frame region.
    /// In embedded mode, this prepares the pixel buffer for updates.
    pub fn on_begin_paint(&self) {
        // In a real implementation, this would:
        // 1. Lock the pixel buffer
        // 2. Prepare for incoming frame data
    }

    /// Handles FreeRDP EndPaint callback
    ///
    /// This is called by FreeRDP after rendering a frame region.
    /// The pixel data is blitted to the Wayland surface.
    ///
    /// # Arguments
    ///
    /// * `x` - X coordinate of the updated region
    /// * `y` - Y coordinate of the updated region
    /// * `width` - Width of the updated region
    /// * `height` - Height of the updated region
    /// * `data` - Pixel data for the region
    /// * `stride` - Stride of the pixel data
    pub fn on_end_paint(&self, x: i32, y: i32, width: i32, height: i32, data: &[u8], stride: u32) {
        // Update the pixel buffer with the new frame data
        self.pixel_buffer.borrow_mut().update_region(
            x.unsigned_abs(),
            y.unsigned_abs(),
            width.unsigned_abs(),
            height.unsigned_abs(),
            data,
            stride,
        );

        // Damage the Wayland surface region
        self.wl_surface.borrow().damage(x, y, width, height);

        // Commit the surface
        self.wl_surface.borrow().commit();

        // Queue a redraw of the GTK widget
        self.drawing_area.queue_draw();
    }

    /// Sends a keyboard event to the RDP session
    ///
    /// # Arguments
    ///
    /// * `keyval` - GTK key value
    /// * `pressed` - Whether the key is pressed or released
    pub fn send_key(&self, keyval: u32, pressed: bool) {
        if !*self.is_embedded.borrow() {
            return;
        }

        if *self.state.borrow() != RdpConnectionState::Connected {
            return;
        }

        // Send keyboard event via FreeRDP thread
        if let Some(ref thread) = *self.freerdp_thread.borrow() {
            let _ = thread.send_command(RdpCommand::KeyEvent { keyval, pressed });
        }
    }

    /// Sends Ctrl+Alt+Del key sequence to the RDP session
    ///
    /// This is commonly used to unlock Windows login screens or access
    /// the security options menu.
    pub fn send_ctrl_alt_del(&self) {
        if !*self.is_embedded.borrow() {
            return;
        }

        if *self.state.borrow() != RdpConnectionState::Connected {
            return;
        }

        // Send the Ctrl+Alt+Del command to the FreeRDP thread
        if let Some(ref thread) = *self.freerdp_thread.borrow() {
            let _ = thread.send_command(RdpCommand::SendCtrlAltDel);
        }
    }

    /// Sends a mouse event to the RDP session
    ///
    /// # Arguments
    ///
    /// * `x` - X coordinate
    /// * `y` - Y coordinate
    /// * `button` - Mouse button (0 = none/motion, 1 = left, 2 = middle, 3 = right)
    /// * `pressed` - Whether the button is pressed or released
    pub fn send_mouse(&self, x: i32, y: i32, button: u32, pressed: bool) {
        if !*self.is_embedded.borrow() {
            return;
        }

        if *self.state.borrow() != RdpConnectionState::Connected {
            return;
        }

        // Send mouse event via FreeRDP thread
        if let Some(ref thread) = *self.freerdp_thread.borrow() {
            let _ = thread.send_command(RdpCommand::MouseEvent {
                x,
                y,
                button,
                pressed,
            });
        }
    }

    /// Notifies the RDP session of a resolution change
    ///
    /// # Arguments
    ///
    /// * `width` - New width in pixels
    /// * `height` - New height in pixels
    pub fn notify_resize(&self, width: u32, height: u32) {
        if !*self.is_embedded.borrow() {
            return;
        }

        if *self.state.borrow() != RdpConnectionState::Connected {
            return;
        }

        // Update internal dimensions
        *self.width.borrow_mut() = width;
        *self.height.borrow_mut() = height;

        // Resize pixel buffer
        self.pixel_buffer.borrow_mut().resize(width, height);

        // Send resize command via FreeRDP thread
        if let Some(ref thread) = *self.freerdp_thread.borrow() {
            let _ = thread.send_command(RdpCommand::Resize { width, height });
        }
    }

    /// Returns whether the RDP session is connected
    #[must_use]
    pub fn is_connected(&self) -> bool {
        *self.state.borrow() == RdpConnectionState::Connected
    }

    /// Returns the current configuration
    #[must_use]
    pub fn config(&self) -> Option<RdpConfig> {
        self.config.borrow().clone()
    }
}

impl Drop for EmbeddedRdpWidget {
    fn drop(&mut self) {
        self.disconnect();
    }
}

impl crate::embedded_trait::EmbeddedWidget for EmbeddedRdpWidget {
    fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    fn state(&self) -> crate::embedded_trait::EmbeddedConnectionState {
        match *self.state.borrow() {
            RdpConnectionState::Disconnected => {
                crate::embedded_trait::EmbeddedConnectionState::Disconnected
            }
            RdpConnectionState::Connecting => {
                crate::embedded_trait::EmbeddedConnectionState::Connecting
            }
            RdpConnectionState::Connected => {
                crate::embedded_trait::EmbeddedConnectionState::Connected
            }
            RdpConnectionState::Error => crate::embedded_trait::EmbeddedConnectionState::Error,
        }
    }

    fn is_embedded(&self) -> bool {
        *self.is_embedded.borrow()
    }

    fn disconnect(&self) -> Result<(), crate::embedded_trait::EmbeddedError> {
        // Call the existing disconnect method (returns ())
        Self::disconnect(self);
        Ok(())
    }

    fn reconnect(&self) -> Result<(), crate::embedded_trait::EmbeddedError> {
        Self::reconnect(self)
            .map_err(|e| crate::embedded_trait::EmbeddedError::ConnectionFailed(e.to_string()))
    }

    fn send_ctrl_alt_del(&self) {
        Self::send_ctrl_alt_del(self);
    }

    fn protocol_name(&self) -> &'static str {
        "RDP"
    }
}

// Tests moved to types.rs, buffer.rs, and launcher.rs submodules
