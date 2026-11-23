//! Connection wizard and duplicate-via-wizard flow

use super::*;

impl MainWindow {
    /// Shows the Connection Wizard (simplified step-by-step flow)
    pub(super) fn show_connection_wizard(
        window: &adw::ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
        toast_overlay: SharedToastOverlay,
    ) {
        use crate::dialogs::connection_wizard::{ConnectionWizard, WizardResult};

        let wizard = ConnectionWizard::new(state.clone());

        let state_for_cb = state.clone();
        let sidebar_for_cb = sidebar.clone();
        let window_weak = window.downgrade();
        let toast_for_cb = toast_overlay;
        wizard.connect_complete(move |result| {
            match result {
                WizardResult::Save(conn, password) => {
                    let conn_name = conn.name.clone();
                    let conn_host = conn.host.clone();
                    let conn_username = conn.username.clone();
                    let password_source = conn.password_source.clone();
                    let protocol = conn.protocol;
                    let created = match state_for_cb.try_borrow_mut() {
                        Ok(mut state_mut) => state_mut.create_connection(conn).map(|conn_id| {
                            Self::persist_wizard_password(
                                &state_mut,
                                conn_id,
                                &password_source,
                                password.as_ref(),
                                &conn_name,
                                &conn_host,
                                conn_username.as_deref().unwrap_or(""),
                                protocol,
                            );
                        }),
                        Err(_) => Err(crate::i18n::i18n("the application is busy")),
                    };
                    created?;
                    let state_c = state_for_cb.clone();
                    let sidebar_c = sidebar_for_cb.clone();
                    let toast_c = toast_for_cb.clone();
                    let name = conn_name;
                    glib::idle_add_local_once(move || {
                        Self::reload_sidebar_preserving_state(&state_c, &sidebar_c);
                        toast_c.show_success(&crate::i18n::i18n_f(
                            "Connection “{}” created",
                            &[&name],
                        ));
                    });
                    Ok(())
                }
                WizardResult::SaveAndConnect(conn, password) => {
                    let conn_id_outer = conn.id;
                    let conn_name = conn.name.clone();
                    let conn_host = conn.host.clone();
                    let conn_username = conn.username.clone();
                    let conn_domain = conn.domain.clone();
                    let password_source = conn.password_source.clone();
                    let protocol = conn.protocol;
                    let created = match state_for_cb.try_borrow_mut() {
                        Ok(mut state_mut) => state_mut.create_connection(conn).map(|conn_id| {
                            Self::persist_wizard_password(
                                &state_mut,
                                conn_id,
                                &password_source,
                                password.as_ref(),
                                &conn_name,
                                &conn_host,
                                conn_username.as_deref().unwrap_or(""),
                                protocol,
                            );
                            Self::cache_wizard_password(
                                &mut state_mut,
                                conn_id,
                                conn_username.as_deref().unwrap_or(""),
                                conn_domain.as_deref().unwrap_or(""),
                                password.as_ref(),
                            );
                        }),
                        Err(_) => Err(crate::i18n::i18n("the application is busy")),
                    };
                    created?;
                    let state_c = state_for_cb.clone();
                    let sidebar_c = sidebar_for_cb.clone();
                    let window_w = window_weak.clone();
                    glib::idle_add_local_once(move || {
                        Self::reload_sidebar_preserving_state(&state_c, &sidebar_c);
                        // Connect after sidebar refresh
                        if let Some(win) = window_w.upgrade() {
                            let variant = conn_id_outer.to_string().to_variant();
                            gio::prelude::ActionGroupExt::activate_action(
                                &win,
                                "connect-to",
                                Some(&variant),
                            );
                        }
                    });
                    Ok(())
                }
                WizardResult::OpenAdvanced(partial) => {
                    // Open full dialog pre-filled with wizard data
                    if let Some(win) = window_weak.upgrade() {
                        let password = partial.storable_password();
                        let conn = partial.to_connection();
                        connection_dialogs::show_new_connection_dialog_prefilled(
                            win.upcast_ref(),
                            state_for_cb.clone(),
                            sidebar_for_cb.clone(),
                            conn,
                            password,
                        );
                    }
                    Ok(())
                }
            }
        });

        wizard.present(window);
    }

    /// Keeps a wizard-typed password for the connect that follows Save & Connect.
    ///
    /// The vault write runs on a worker thread, and the connect path reads the
    /// vault at once, so it could miss the entry and prompt for a password the
    /// user had just typed. The session cache is what that path checks first.
    pub(super) fn cache_wizard_password(
        state_mut: &mut crate::state::AppState,
        conn_id: uuid::Uuid,
        username: &str,
        domain: &str,
        password: Option<&secrecy::SecretString>,
    ) {
        use secrecy::ExposeSecret;
        if let Some(pwd) = password {
            state_mut.cache_credentials(conn_id, username, pwd.expose_secret(), domain);
        }
    }

    /// Persists a wizard-entered password to the configured vault.
    ///
    /// No-op unless `password_source` is `Vault` and a password is present.
    /// Mirrors the full connection dialog so connections created via the
    /// wizard resolve their credentials correctly (issue #188).
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors save_password_to_vault parameters 1:1; a struct would only restate them"
    )]
    pub(super) fn persist_wizard_password(
        state_mut: &crate::state::AppState,
        conn_id: uuid::Uuid,
        password_source: &rustconn_core::models::PasswordSource,
        password: Option<&secrecy::SecretString>,
        conn_name: &str,
        conn_host: &str,
        username: &str,
        protocol: rustconn_core::models::ProtocolType,
    ) {
        if *password_source != rustconn_core::models::PasswordSource::Vault {
            return;
        }
        let Some(pwd) = password else {
            return;
        };
        let settings = state_mut.settings().clone();
        let groups: Vec<_> = state_mut.list_groups_owned();
        let conn_for_path = state_mut.get_connection(conn_id).cloned();
        crate::state::save_password_to_vault(
            &settings,
            &groups,
            conn_for_path.as_ref(),
            conn_name,
            conn_host,
            protocol,
            username,
            pwd,
            conn_id,
        );
    }

    /// Opens the Connection Wizard pre-filled with data from the selected connection.
    ///
    /// Used for "Duplicate via Wizard…" — allows modifying fields before saving as new.
    pub(super) fn duplicate_via_wizard(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        toast_overlay: &SharedToastOverlay,
    ) {
        let Some(selected) = sidebar.get_selected_item() else {
            return;
        };
        if selected.is_group() {
            return;
        }
        let Ok(id) = uuid::Uuid::parse_str(&selected.id()) else {
            return;
        };
        Self::duplicate_via_wizard_by_id(window, state, sidebar, toast_overlay, id);
    }

    /// Same as [`Self::duplicate_via_wizard`] for a known connection id, so a tab
    /// can offer it without depending on the sidebar selection.
    pub(super) fn duplicate_via_wizard_by_id(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        toast_overlay: &SharedToastOverlay,
        id: uuid::Uuid,
    ) {
        use crate::dialogs::connection_wizard::{
            ConnectionWizard, PartialConnection, WizardResult,
        };

        let conn = {
            let Ok(state_ref) = state.try_borrow() else {
                return;
            };
            let Some(c) = state_ref.get_connection(id).cloned() else {
                return;
            };
            c
        };

        let mut partial = PartialConnection::from_connection(&conn);
        // A new name up front, so the wizard shows what will be saved instead of
        // the source's name (which the store would silently suffix).
        if let Ok(state_ref) = state.try_borrow() {
            partial.name =
                Some(state_ref.generate_unique_connection_name(
                    &format!("{} (copy)", conn.name),
                    conn.protocol,
                ));
        }
        // The wizard has no group page: the copy stays beside its source.
        let source_group = conn.group_id;
        let wizard = ConnectionWizard::new(state.clone());
        wizard.set_partial(&partial);

        let state_for_cb = state.clone();
        let sidebar_for_cb = sidebar.clone();
        let window_weak = window.downgrade();
        let toast_for_cb = toast_overlay.clone();
        wizard.connect_complete(move |result| match result {
            WizardResult::Save(mut new_conn, password) => {
                new_conn.group_id = source_group;
                let conn_name = new_conn.name.clone();
                let conn_host = new_conn.host.clone();
                let conn_username = new_conn.username.clone();
                let password_source = new_conn.password_source.clone();
                let protocol = new_conn.protocol;
                let created = match state_for_cb.try_borrow_mut() {
                    Ok(mut state_mut) => state_mut.create_connection(new_conn).map(|conn_id| {
                        Self::persist_wizard_password(
                            &state_mut,
                            conn_id,
                            &password_source,
                            password.as_ref(),
                            &conn_name,
                            &conn_host,
                            conn_username.as_deref().unwrap_or(""),
                            protocol,
                        );
                    }),
                    Err(_) => Err(crate::i18n::i18n("the application is busy")),
                };
                created?;
                let state_c = state_for_cb.clone();
                let sidebar_c = sidebar_for_cb.clone();
                let toast_c = toast_for_cb.clone();
                glib::idle_add_local_once(move || {
                    Self::reload_sidebar_preserving_state(&state_c, &sidebar_c);
                    toast_c.show_success(&crate::i18n::i18n_f(
                        "Connection “{}” created",
                        &[&conn_name],
                    ));
                });
                Ok(())
            }
            WizardResult::SaveAndConnect(mut new_conn, password) => {
                new_conn.group_id = source_group;
                let conn_id_outer = new_conn.id;
                let conn_name = new_conn.name.clone();
                let conn_host = new_conn.host.clone();
                let conn_username = new_conn.username.clone();
                let conn_domain = new_conn.domain.clone();
                let password_source = new_conn.password_source.clone();
                let protocol = new_conn.protocol;
                let created = match state_for_cb.try_borrow_mut() {
                    Ok(mut state_mut) => state_mut.create_connection(new_conn).map(|conn_id| {
                        Self::persist_wizard_password(
                            &state_mut,
                            conn_id,
                            &password_source,
                            password.as_ref(),
                            &conn_name,
                            &conn_host,
                            conn_username.as_deref().unwrap_or(""),
                            protocol,
                        );
                        Self::cache_wizard_password(
                            &mut state_mut,
                            conn_id,
                            conn_username.as_deref().unwrap_or(""),
                            conn_domain.as_deref().unwrap_or(""),
                            password.as_ref(),
                        );
                    }),
                    Err(_) => Err(crate::i18n::i18n("the application is busy")),
                };
                created?;
                let state_c = state_for_cb.clone();
                let sidebar_c = sidebar_for_cb.clone();
                let window_w = window_weak.clone();
                glib::idle_add_local_once(move || {
                    Self::reload_sidebar_preserving_state(&state_c, &sidebar_c);
                    if let Some(win) = window_w.upgrade() {
                        let variant = conn_id_outer.to_string().to_variant();
                        gio::prelude::ActionGroupExt::activate_action(
                            &win,
                            "connect-to",
                            Some(&variant),
                        );
                    }
                });
                Ok(())
            }
            WizardResult::OpenAdvanced(new_partial) => {
                if let Some(win) = window_weak.upgrade() {
                    let password = new_partial.storable_password();
                    let new_conn = new_partial.to_connection();
                    connection_dialogs::show_new_connection_dialog_prefilled(
                        win.upcast_ref(),
                        state_for_cb.clone(),
                        sidebar_for_cb.clone(),
                        new_conn,
                        password,
                    );
                }
                Ok(())
            }
        });

        wizard.present(window);
    }
}
