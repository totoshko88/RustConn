//! "Copy" menu actions: put one field of a connection on the clipboard.
//!
//! Every entry is addressed by connection id, never by the sidebar selection,
//! so the same action serves the sidebar, a smart folder and a session tab
//! (issue #357). Which fields a connection offers, and their text, is decided
//! in `rustconn_core::connection::copy_field`.

use rustconn_core::connection::{CopyField, copy_fields, copy_text, resolve_proxy_jump_value};
use secrecy::ExposeSecret;
use zeroize::Zeroizing;

use super::*;
use crate::i18n::{i18n, i18n_f};

/// Seconds after which a copied secret is removed from the clipboard, if the
/// clipboard still holds it. Long enough to paste into a login form, short
/// enough that it does not linger for the next paste.
const SECRET_CLIPBOARD_TTL_SECS: u32 = 30;

// "Password copied (auto-clears in 30s)" is an existing, fully translated
// string shared with the password generator, so it names the number instead
// of taking it as a placeholder. Changing the TTL means changing that string.
const _: () = assert!(SECRET_CLIPBOARD_TTL_SECS == 30);

/// Name of the parameterised copy action, without the `win.` prefix.
pub(crate) const COPY_FIELD_ACTION: &str = "copy-connection-field";

/// One entry of a "Copy" menu, ready to render.
#[derive(Debug, Clone)]
pub(crate) struct CopyMenuEntry {
    /// The translated label shown in the menu.
    pub(crate) label: String,
    /// Target of [`COPY_FIELD_ACTION`]: `"<connection id>|<field key>"`.
    pub(crate) target: String,
    /// Whether this is a custom property, so a separator can set them apart.
    pub(crate) is_property: bool,
}

/// Builds the "Copy" menu entries for `connection`.
pub(crate) fn copy_menu_entries(connection: &rustconn_core::Connection) -> Vec<CopyMenuEntry> {
    copy_fields(connection)
        .into_iter()
        .map(|field| CopyMenuEntry {
            label: copy_field_label(connection, &field),
            target: format!("{}|{}", connection.id, field.key()),
            is_property: matches!(field, CopyField::Property(_)),
        })
        .collect()
}

/// The sidebar's "Copy" submenu items for `connection`, custom properties set
/// apart by a separator.
pub(crate) fn sidebar_copy_items(
    connection: &rustconn_core::Connection,
) -> Vec<crate::sidebar_ui::ContextMenuItem> {
    use crate::sidebar_ui::ContextMenuItem;

    let mut items = Vec::new();
    let mut in_properties = false;
    for entry in copy_menu_entries(connection) {
        if entry.is_property && !in_properties && !items.is_empty() {
            items.push(ContextMenuItem::Separator);
        }
        in_properties |= entry.is_property;
        items.push(ContextMenuItem::action_with_target(
            &entry.label,
            COPY_FIELD_ACTION,
            &entry.target.to_variant(),
        ));
    }
    items
}

/// The menu label of `field`. The port carries its value, so a list of
/// routers on different ports can be told apart without opening the editor;
/// nothing secret ever appears in a label.
fn copy_field_label(connection: &rustconn_core::Connection, field: &CopyField) -> String {
    match field {
        CopyField::Host => i18n("Host"),
        CopyField::Port => i18n_f("Port ({})", &[&connection.port.to_string()]),
        CopyField::Address => i18n("Address"),
        CopyField::Username => i18n("Username"),
        CopyField::Password => i18n("Password"),
        CopyField::SshCommand => i18n("SSH Command"),
        // A user-chosen name, shown as typed.
        CopyField::Property(name) => name.clone(),
    }
}

impl MainWindow {
    /// Registers `win.copy-connection-field` and `win.edit-connection-by-id`.
    pub(crate) fn setup_copy_field_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        let copy_action = gio::SimpleAction::new(COPY_FIELD_ACTION, Some(glib::VariantTy::STRING));
        let state_clone = state.clone();
        let window_weak = window.downgrade();
        let toast_clone = self.toast_overlay.clone();
        copy_action.connect_activate(move |_, param| {
            let Some(target) = param.and_then(glib::Variant::get::<String>) else {
                return;
            };
            let Some((id, key)) = target.split_once('|') else {
                return;
            };
            let (Ok(conn_id), Some(field)) = (Uuid::parse_str(id), CopyField::from_key(key)) else {
                tracing::warn!(%target, "Malformed copy-connection-field target");
                return;
            };
            if let Some(win) = window_weak.upgrade() {
                copy_connection_field(&win, &state_clone, &toast_clone, conn_id, &field);
            }
        });
        window.add_action(&copy_action);

        let edit_action =
            gio::SimpleAction::new("edit-connection-by-id", Some(glib::VariantTy::STRING));
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let notebook_clone = self.terminal_notebook.clone();
        let window_weak = window.downgrade();
        let key_setup_ctx = self.key_setup_context(window, state, sidebar);
        edit_action.connect_activate(move |_, param| {
            let Some(id) = param
                .and_then(glib::Variant::get::<String>)
                .and_then(|s| Uuid::parse_str(&s).ok())
            else {
                return;
            };
            if let Some(win) = window_weak.upgrade() {
                super::edit_dialogs::edit_connection_by_id(
                    win.upcast_ref(),
                    &state_clone,
                    &sidebar_clone,
                    &notebook_clone,
                    id,
                    &key_setup_ctx,
                );
            }
        });
        window.add_action(&edit_action);
    }
}

/// Copies `field` of the connection `conn_id` and confirms with a toast.
pub(crate) fn copy_connection_field(
    window: &adw::ApplicationWindow,
    state: &SharedAppState,
    toast: &SharedToastOverlay,
    conn_id: Uuid,
    field: &CopyField,
) {
    match field {
        CopyField::Username => return copy_username(window, state, toast, conn_id),
        CopyField::Password => return copy_password(window, state, toast, conn_id),
        _ => {}
    }

    let (text, sensitive) = {
        let Ok(state_ref) = state.try_borrow() else {
            return;
        };
        let Some(conn) = state_ref.get_connection(conn_id) else {
            return;
        };
        let proxy_jump = if *field == CopyField::SshCommand {
            resolve_proxy_jump_value(
                conn,
                &state_ref.list_connections_owned(),
                &state_ref.list_groups_owned(),
                &state_ref.settings().network,
            )
        } else {
            None
        };
        (
            copy_text(conn, field, proxy_jump.as_deref()).map(Zeroizing::new),
            field.is_sensitive(conn),
        )
    };
    let Some(text) = text else {
        return;
    };

    if sensitive {
        copy_secret(
            window,
            toast,
            text,
            &i18n_f(
                "Copied (auto-clears in {}s)",
                &[&SECRET_CLIPBOARD_TTL_SECS.to_string()],
            ),
        );
    } else {
        gtk4::prelude::WidgetExt::display(window)
            .clipboard()
            .set_text(&text);
        toast.show_success(&i18n("Copied"));
    }
}

/// Copies the user name: the one resolved at connect time when there is one,
/// otherwise the one stored on the connection, otherwise the one the secret
/// backend (vault, variable, script, parent group) supplies.
pub(crate) fn copy_username(
    window: &adw::ApplicationWindow,
    state: &SharedAppState,
    toast: &SharedToastOverlay,
    conn_id: Uuid,
) {
    {
        let Ok(state_ref) = state.try_borrow() else {
            return;
        };
        let Some(conn) = state_ref.get_connection(conn_id) else {
            return;
        };
        let known = state_ref
            .get_cached_credentials(conn_id)
            .map(|creds| creds.username.clone())
            .filter(|u| !u.trim().is_empty())
            .or_else(|| conn.username.clone().filter(|u| !u.trim().is_empty()));
        if let Some(username) = known {
            set_username_clipboard(window, toast, &username);
            return;
        }
    }

    // Nothing cached or stored: the field was offered because the password
    // source can supply a user name too, so ask it. The borrow above is
    // released first, because the resolver re-borrows the state.
    let window_weak = window.downgrade();
    let toast = toast.clone();
    crate::state::resolve_credentials_gtk(state, conn_id, move |result| {
        use rustconn_core::sync::CredentialResolutionResult;
        let resolved = match result {
            Ok(CredentialResolutionResult::Resolved(creds)) => {
                creds.username.filter(|u| !u.trim().is_empty())
            }
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to resolve credentials for username copy");
                None
            }
        };
        match (resolved, window_weak.upgrade()) {
            (Some(username), Some(win)) => set_username_clipboard(&win, &toast, &username),
            _ => toast.show_warning(&i18n("No username configured")),
        }
    });
}

/// Puts a user name on the clipboard and confirms with a toast.
fn set_username_clipboard(
    window: &adw::ApplicationWindow,
    toast: &SharedToastOverlay,
    username: &str,
) {
    gtk4::prelude::WidgetExt::display(window)
        .clipboard()
        .set_text(username);
    toast.show_success(&i18n("Username copied"));
}

/// Copies the password, clearing it from the clipboard after
/// [`SECRET_CLIPBOARD_TTL_SECS`]. Uses the credential cached at connect time
/// when there is one, and otherwise resolves it from the secret backend.
pub(crate) fn copy_password(
    window: &adw::ApplicationWindow,
    state: &SharedAppState,
    toast: &SharedToastOverlay,
    conn_id: Uuid,
) {
    let password_copied = i18n("Password copied (auto-clears in 30s)");
    {
        let Ok(state_ref) = state.try_borrow() else {
            return;
        };
        if state_ref.get_connection(conn_id).is_none() {
            return;
        }
        if let Some(creds) = state_ref.get_cached_credentials(conn_id) {
            let pw = creds.password.expose_secret();
            if pw.is_empty() {
                toast.show_warning(&i18n("Cached password is empty"));
            } else {
                copy_secret(
                    window,
                    toast,
                    Zeroizing::new(pw.to_string()),
                    &password_copied,
                );
            }
            return;
        }
    }

    // Not cached: resolve from the secret backend. The borrow above is
    // released first, because the resolver re-borrows the state.
    let window_weak = window.downgrade();
    let toast = toast.clone();
    crate::state::resolve_credentials_gtk(state, conn_id, move |result| {
        use rustconn_core::sync::CredentialResolutionResult;
        match result {
            Ok(CredentialResolutionResult::Resolved(creds)) => match creds.password {
                Some(ref password) if !password.expose_secret().is_empty() => {
                    if let Some(win) = window_weak.upgrade() {
                        copy_secret(
                            &win,
                            &toast,
                            Zeroizing::new(password.expose_secret().to_string()),
                            &password_copied,
                        );
                    }
                }
                Some(_) => toast.show_warning(&i18n("Password is empty")),
                None => toast.show_warning(&i18n("No password configured for this connection")),
            },
            Ok(_) => toast.show_warning(&i18n("No password configured for this connection")),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to resolve credentials for copy");
                toast.show_warning(&i18n("Could not retrieve password from secret backend"));
            }
        }
    });
}

/// Puts `secret` on the clipboard and removes it again after
/// [`SECRET_CLIPBOARD_TTL_SECS`], unless something else was copied meanwhile.
fn copy_secret(
    window: &adw::ApplicationWindow,
    toast: &SharedToastOverlay,
    secret: Zeroizing<String>,
    message: &str,
) {
    let clipboard = gtk4::prelude::WidgetExt::display(window).clipboard();
    clipboard.set_text(&secret);
    toast.show_success(message);
    let clipboard_weak = clipboard.downgrade();
    glib::timeout_add_seconds_local_once(SECRET_CLIPBOARD_TTL_SECS, move || {
        if let Some(cb) = clipboard_weak.upgrade() {
            cb.read_text_async(gio::Cancellable::NONE, move |result| {
                if let Ok(Some(current)) = result
                    && current.as_str() == secret.as_str()
                    && let Some(cb2) = clipboard_weak.upgrade()
                {
                    cb2.set_text("");
                }
            });
        }
    });
}
