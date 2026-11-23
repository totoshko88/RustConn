//! Sign-in to the Proton Pass CLI in a terminal of RustConn's own.
//!
//! `pass-cli` keeps its session in its own data directory, so `pass-cli login`
//! has to run where RustConn later runs `pass-cli`. In a sandbox that is inside
//! the sandbox: a host terminal is not visible from there, and one would sign in
//! the host's `pass-cli`, a session RustConn never reads. The login therefore
//! runs in a VTE terminal in a dialog, with the same environment the backend
//! gives `pass-cli`.

use std::path::Path;
use std::sync::mpsc::TryRecvError;

use adw::prelude::*;
use gtk4::{Label, Orientation, gio, glib};
use libadwaita as adw;
use vte4::prelude::*;

use super::detection::detect_proton_pass;
use super::update_status_label;
use crate::i18n::i18n;

/// PCRE2 multiline compile flag — required by VTE's `match_add_regex()`.
const PCRE2_MULTILINE: u32 = 0x0000_0400;

/// The web login prints the URL to open; `pass-cli` does not open a browser.
const URL_PATTERN: &str = r"https://[^\s]+";

/// Opens the sign-in dialog and runs `pass_cli login` in it.
///
/// `status_label` is the Settings row's status; it is refreshed when the login
/// exits. A login that succeeds closes the dialog; one that fails leaves it
/// open, so its error stays readable.
pub(super) fn present(parent: &impl IsA<gtk4::Widget>, pass_cli: &Path, status_label: &Label) {
    let terminal = vte4::Terminal::new();
    terminal.set_hexpand(true);
    terminal.set_vexpand(true);
    terminal.set_scroll_on_output(true);
    make_links_clickable(&terminal);

    let hint = Label::builder()
        .label(i18n(
            "Open the link below in your browser to finish signing in. This window closes once you are signed in.",
        ))
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();

    let content = gtk4::Box::builder()
        .orientation(Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&hint);
    content.append(&terminal);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&content));

    let dialog = adw::Dialog::builder()
        .title(i18n("Sign In to Proton Pass"))
        .content_width(760)
        .content_height(460)
        .child(&toolbar)
        .build();

    {
        let dialog = dialog.downgrade();
        let status_label = status_label.clone();
        terminal.connect_child_exited(move |_, status| {
            let dialog = dialog.clone();
            let status_label = status_label.clone();
            // Not inside `child-exited`: changing the widget tree there can
            // race the pending GTK snapshot (#171).
            glib::idle_add_local_once(move || {
                tracing::info!(status, "pass-cli login exited");
                refresh_status(&status_label);
                // A waitpid status of 0 is exactly "exited with code 0".
                if status == 0
                    && let Some(dialog) = dialog.upgrade()
                {
                    dialog.close();
                }
            });
        });
    }

    let program = pass_cli.to_string_lossy();
    let mut envv = vec![format!(
        "PATH={}",
        rustconn_core::cli_download::get_extended_path()
    )];
    if let Some((key, value)) = rustconn_core::secret::pass_cli_sandbox_env() {
        envv.push(format!("{key}={value}"));
    }
    let envv: Vec<&str> = envv.iter().map(String::as_str).collect();
    let status_for_spawn = status_label.clone();
    // ponytail: closing the dialog mid-login drops the terminal, and with it the
    // PTY, so `pass-cli` gets SIGHUP rather than an explicit kill; enough for a
    // CLI waiting on the browser. Track the pid if one ever ignores SIGHUP.
    terminal.spawn_async(
        vte4::PtyFlags::DEFAULT,
        None,
        &[program.as_ref(), "login"],
        &envv,
        glib::SpawnFlags::DEFAULT,
        || {},
        -1,
        gio::Cancellable::NONE,
        move |result| {
            if let Err(error) = result {
                tracing::warn!(%error, "could not start pass-cli login");
                update_status_label(
                    &status_for_spawn,
                    &i18n("Could not start pass-cli"),
                    "error",
                );
            }
        },
    );

    dialog.present(Some(parent));
    terminal.grab_focus();
}

/// True for an `https://` link on `proton.me` or one of its subdomains. The
/// terminal shows whatever the CLI prints, so only Proton's own sign-in links
/// are opened on click.
fn is_proton_link(uri: &str) -> bool {
    // A backslash or userinfo makes browsers read another host than GLib does
    // (`https://evil.com\@proton.me/`).
    if uri.contains('\\') {
        return false;
    }
    glib::Uri::parse(uri, glib::UriFlags::NONE).is_ok_and(|u| {
        u.scheme() == "https"
            && u.userinfo().is_none()
            && u.host()
                .is_some_and(|h| h == "proton.me" || h.ends_with(".proton.me"))
    })
}

/// Underlines URLs under the pointer and opens one on click.
fn make_links_clickable(terminal: &vte4::Terminal) {
    match vte4::Regex::for_match(URL_PATTERN, PCRE2_MULTILINE) {
        Ok(regex) => {
            let tag = terminal.match_add_regex(&regex, 0);
            terminal.match_set_cursor_name(tag, "pointer");
        }
        Err(error) => tracing::warn!(%error, "URL regex rejected by VTE"),
    }

    let click = gtk4::GestureClick::new();
    click.connect_released(|gesture, _, x, y| {
        let Some(terminal) = gesture.widget().and_downcast::<vte4::Terminal>() else {
            return;
        };
        let (Some(uri), _) = terminal.check_match_at(x, y) else {
            return;
        };
        if !is_proton_link(&uri) {
            tracing::warn!("refused to open a sign-in link outside proton.me");
            return;
        }
        let window = terminal.root().and_downcast::<gtk4::Window>();
        gtk4::UriLauncher::new(&uri).launch(window.as_ref(), gio::Cancellable::NONE, |result| {
            if let Err(error) = result
                && !error.matches(gtk4::DialogError::Dismissed)
            {
                tracing::warn!(%error, "could not open the Proton sign-in link");
            }
        });
    });
    terminal.add_controller(click);
}

/// Re-reads the CLI's session state into the Settings status label, off the
/// main thread: `pass-cli info` reaches the Proton API.
fn refresh_status(status_label: &Label) {
    update_status_label(status_label, &i18n("Detecting…"), "dim-label");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(detect_proton_pass().1);
    });
    let status_label = status_label.clone();
    // 50 ms timeout rather than an idle source, which would spin the main loop.
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        match rx.try_recv() {
            Ok(Some((text, css))) => update_status_label(&status_label, &text, css),
            Ok(None) | Err(TryRecvError::Disconnected) => {
                update_status_label(&status_label, &i18n("Error checking status"), "error");
            }
            Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
        }
        glib::ControlFlow::Break
    });
}

#[cfg(test)]
mod tests {
    use super::is_proton_link;

    #[test]
    fn only_https_proton_hosts_open() {
        assert!(is_proton_link("https://account.proton.me/auth?x=1"));
        assert!(is_proton_link("https://proton.me/"));
        assert!(!is_proton_link("http://proton.me/"));
        assert!(!is_proton_link("https://proton.me.evil.com/"));
        assert!(!is_proton_link("https://evilproton.me/"));
        assert!(!is_proton_link("https://user@evil.com/proton.me"));
    }
}
