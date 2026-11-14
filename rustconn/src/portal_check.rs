//! Startup probe for the `xdg-desktop-portal` file-chooser backend.
//!
//! GTK4 opens `GtkFileDialog` through the portal. With the service or its backend
//! missing (a bare window manager, a minimal KDE install without
//! `xdg-desktop-portal-kde`) the dialog fails or waits for a bus activation
//! timeout, and the user sees nothing. Probing once at startup, with a hard
//! timeout, turns that into one warning toast and one log line.

use gtk4::prelude::*;
use gtk4::{gio, glib};

const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
/// Longer than a normal bus activation, shorter than a user's patience.
const PROBE_TIMEOUT_MS: i32 = 3000;

/// What the probe found.
#[derive(Debug, PartialEq, Eq)]
pub enum PortalProblem {
    /// The portal service does not answer (not installed, not activatable, hung).
    NoService,
    /// The service answers but no backend provides the file chooser.
    NoFileChooser,
}

/// Maps the D-Bus error name of a failed `Properties.Get` to a problem.
/// `None` means the call failed locally (no reply, timeout, no bus).
fn classify(remote_error: Option<&str>) -> PortalProblem {
    match remote_error {
        Some(
            "org.freedesktop.DBus.Error.UnknownInterface"
            | "org.freedesktop.DBus.Error.UnknownObject"
            | "org.freedesktop.DBus.Error.UnknownProperty",
        ) => PortalProblem::NoFileChooser,
        _ => PortalProblem::NoService,
    }
}

#[expect(
    clippy::future_not_send,
    reason = "GIO futures are main-thread only; run via spawn_future_local"
)]
async fn probe() -> Result<(), PortalProblem> {
    let conn = gio::bus_get_future(gio::BusType::Session)
        .await
        .map_err(|_| PortalProblem::NoService)?;
    let args = ("org.freedesktop.portal.FileChooser", "version").to_variant();
    conn.call_future(
        Some(PORTAL_NAME),
        PORTAL_PATH,
        "org.freedesktop.DBus.Properties",
        "Get",
        Some(&args),
        None,
        gio::DBusCallFlags::NONE,
        PROBE_TIMEOUT_MS,
    )
    .await
    .map(|_| ())
    .map_err(|e| {
        let remote = gio::DBusError::remote_error(&e);
        classify(remote.as_ref().map(glib::GString::as_str))
    })
}

/// Probes the portal in the background and warns on `window` if it is unusable.
pub fn warn_if_missing(window: &libadwaita::ApplicationWindow) {
    // The Flatpak runtime ships the portal frontend; the host backend is still
    // probed there, so no early return for sandboxes.
    let window = window.downgrade();
    glib::spawn_future_local(async move {
        let Err(problem) = probe().await else {
            return;
        };
        tracing::warn!(
            ?problem,
            "xdg-desktop-portal file chooser unavailable; install xdg-desktop-portal \
             and a backend (xdg-desktop-portal-kde / -gnome / -gtk)"
        );
        if let Some(win) = window.upgrade() {
            crate::toast::show_toast_on_window(
                &win,
                &crate::i18n::i18n(
                    "Desktop portal not found — file dialogs may not open. See Troubleshooting in the User Guide",
                ),
                crate::toast::ToastType::Warning,
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_interface_means_no_backend() {
        assert_eq!(
            classify(Some("org.freedesktop.DBus.Error.UnknownInterface")),
            PortalProblem::NoFileChooser
        );
    }

    #[test]
    fn anything_else_means_no_service() {
        assert_eq!(classify(None), PortalProblem::NoService);
        assert_eq!(
            classify(Some("org.freedesktop.DBus.Error.ServiceUnknown")),
            PortalProblem::NoService
        );
    }
}
