//! Password source, vault integration and visibility wiring
//!
//! Mechanically split out of `dialog.rs` (pure code motion).

#![allow(
    clippy::similar_names,
    reason = "module-wide override for legacy code; refactored case by case"
)]

use std::rc::Rc;

use gtk4::Box as GtkBox;
use gtk4::prelude::*;
use libadwaita as adw;

use super::ConnectionDialog;
use crate::alert;
use crate::i18n::{i18n, i18n_f};

/// The `.kdbx` file the Load and Test buttons read (issue #411): what a connect
/// will read once the dialog is saved — the dialog's own KeePass Database
/// field, else the selected group's override chain, else the global database.
fn dialog_kdbx_path(
    override_entry: &adw::EntryRow,
    group_id: Option<uuid::Uuid>,
    groups: &[rustconn_core::models::ConnectionGroup],
    global: Option<&std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    let text = override_entry.text();
    let own = text.trim();
    if !own.is_empty() {
        return Some(std::path::PathBuf::from(own));
    }
    rustconn_core::secret::resolve_group_kdbx_path_override(group_id, groups)
        .or_else(|| global.cloned())
}

impl ConnectionDialog {
    /// Updates password row visibility based on password source
    /// Shows for: Vault(1)
    /// Hides for: Prompt(0), Variable(2), Inherit(3), None(4)
    pub fn update_password_row_visibility(&self) {
        let selected = self.password_source_dropdown.selected();
        // Show password row for Vault(1) only
        self.password_row.set_visible(selected == 1);
        // Show variable row for Variable(2) only
        self.variable_row.set_visible(selected == 2);
        // Show script row for Script(5) only
        self.script_row.set_visible(selected == 5);
    }

    /// Pre-fills the password entry from a wizard-typed secret.
    ///
    /// Used when transferring partial data into the full Advanced dialog so the
    /// password typed in the wizard is not lost (issue #188).
    pub fn set_password(&self, password: &secrecy::SecretString) {
        use secrecy::ExposeSecret;
        self.password_entry.set_text(password.expose_secret());
    }

    /// Connects password source dropdown to update password row visibility
    pub fn connect_password_source_visibility(&self) {
        let password_row = self.password_row.clone();
        let variable_row = self.variable_row.clone();
        let script_row = self.script_row.clone();
        let ssh_auth_dropdown = self.ssh_auth_dropdown.clone();
        let protocol_dropdown = self.protocol_dropdown.clone();

        self.password_source_dropdown
            .connect_selected_notify(move |dropdown| {
                let selected = dropdown.selected();
                // Show password row for Vault(1) only
                password_row.set_visible(selected == 1);
                // Show variable row for Variable(2) only
                variable_row.set_visible(selected == 2);
                // Show script row for Script(5) only
                script_row.set_visible(selected == 5);

                // Sync: when password source is None(4) and protocol is SSH(0),
                // auto-switch SSH auth from Password(0) to Public Key(1)
                if selected == 4
                    && protocol_dropdown.selected() == 0
                    && ssh_auth_dropdown.selected() == 0
                {
                    ssh_auth_dropdown.set_selected(1);
                }
            });

        // Reverse sync: when SSH auth changes to Password(0) while
        // password source is None(4), auto-switch password source to Prompt(0)
        let password_source_dropdown = self.password_source_dropdown.clone();
        let protocol_dropdown2 = self.protocol_dropdown.clone();
        self.ssh_auth_dropdown
            .connect_selected_notify(move |dropdown| {
                let is_ssh = protocol_dropdown2.selected() == 0;
                if is_ssh && dropdown.selected() == 0 && password_source_dropdown.selected() == 4 {
                    password_source_dropdown.set_selected(0); // Prompt
                }
            });
    }

    /// Connects password load button to load password from vault (KeePass or Keyring)
    ///
    /// This method sets up the click handler for the password load button.
    /// Connects password load button with group hierarchy support
    ///
    /// This method sets up the click handler for the password load button.
    /// When clicked, it loads the password from the appropriate backend based on
    /// the selected password source (KeePass or Keyring).
    ///
    /// # Arguments
    /// * `kdbx_enabled` - Whether KeePass is enabled
    /// * `kdbx_path` - Path to the KeePass database
    /// * `kdbx_password` - Password for the KeePass database
    /// * `kdbx_key_file` - Key file for the KeePass database
    /// * `groups` - List of connection groups for building hierarchical paths
    /// * `secret_settings` - Secret backend settings for backend dispatch
    pub fn connect_password_load_button_with_groups(
        &self,
        kdbx_enabled: bool,
        kdbx_path: Option<std::path::PathBuf>,
        kdbx_password: Option<&secrecy::SecretString>,
        kdbx_key_file: Option<std::path::PathBuf>,
        groups: Vec<rustconn_core::models::ConnectionGroup>,
        secret_settings: rustconn_core::config::SecretSettings,
    ) {
        use crate::utils::spawn_blocking_with_callback;

        let password_source_dropdown = self.password_source_dropdown.clone();
        let password_entry = self.password_entry.clone();
        let username_entry = self.username_entry.clone();
        let domain_entry = self.domain_entry.clone();
        let name_entry = self.name_entry.clone();
        let host_entry = self.host_entry.clone();
        let protocol_dropdown = self.protocol_dropdown.clone();
        let group_dropdown = self.group_dropdown.clone();
        let groups_data = self.groups_data.clone();
        let window = self.dialog.clone();
        let kdbx_password = kdbx_password.cloned();
        let state = self.state.clone();
        let editing_id = self.editing_id.clone();
        let kdbx_override_entry = self.kdbx_override_entry.clone();

        // The KeePass Database row (issue #411) means something only with the
        // KeePass backend; with any other backend it stays hidden. Wired here
        // because this is the one setup call every connection-dialog opener
        // makes with the secret settings at hand.
        let keepass_active = kdbx_enabled
            && matches!(
                secret_settings.preferred_backend,
                rustconn_core::config::SecretBackendType::KeePassXc
                    | rustconn_core::config::SecretBackendType::KdbxFile
            );
        if keepass_active {
            self.password_row
                .bind_property("visible", &self.kdbx_override_entry, "visible")
                .sync_create()
                .build();
        } else {
            self.kdbx_override_entry.set_visible(false);
        }

        // Clone groups for use in closure
        let groups = Rc::new(groups);

        self.password_load_button.connect_clicked(move |btn| {
            let selected = password_source_dropdown.selected();

            // Get connection name for lookup key
            let conn_name = name_entry.text().to_string();
            let conn_host = host_entry.text().to_string();
            let protocol_index = protocol_dropdown.selected();

            // Build lookup key with protocol for uniqueness
            let base_name = if conn_name.trim().is_empty() {
                conn_host.clone()
            } else {
                conn_name.clone()
            };

            if base_name.trim().is_empty() {
                alert::show_error(
                    &window,
                    &i18n("Cannot Load Password"),
                    &i18n("Enter a connection name or host first."),
                );
                return;
            }

            let protocol_suffix = match protocol_index {
                0 => "ssh",
                1 => "rdp",
                2 => "vnc",
                3 => "spice",
                4 => "zerotrust",
                5 => "telnet",
                6 => "serial",
                7 => "sftp",
                8 => "kubernetes",
                9 => "mosh",
                10 => "web",
                11 => "winrm",
                _ => "ssh",
            };

            // Build hierarchical lookup key for KeePass
            let lookup_key = if groups.is_empty() {
                // Legacy behavior: sanitize name and use flat path
                let sanitized_name = base_name.replace('/', "-");
                format!("{sanitized_name} ({protocol_suffix})")
            } else {
                // Build hierarchical path using selected group
                let selected_group_idx = group_dropdown.selected() as usize;
                let groups_data_ref = groups_data.borrow();
                let group_id = if selected_group_idx < groups_data_ref.len() {
                    groups_data_ref[selected_group_idx].0
                } else {
                    None
                };
                drop(groups_data_ref);

                // Build path from group hierarchy
                let group_path = if let Some(gid) = group_id {
                    rustconn_core::secret::KeePassHierarchy::resolve_group_path(gid, &groups)
                } else {
                    Vec::new()
                };

                if group_path.is_empty() {
                    format!("{base_name} ({protocol_suffix})")
                } else {
                    let path = group_path.join("/");
                    format!("{path}/{base_name} ({protocol_suffix})")
                }
            };

            // The lookup key the save path writes. LibSecret and the macOS Keychain
            // use "RustConn/{group}/{name} ({protocol})" — with no group segment
            // when the connection is in no group, which is the part this used to
            // get wrong: it passed `None` for an ungrouped connection, and `None`
            // asks for the flat pre-0.19.18 key instead (issue #316). Bitwarden and
            // the rest use "rustconn/{name}".
            let flat_lookup_key = {
                let backend_type =
                    crate::state::select_backend_for_load(&secret_settings);
                let selected_group_idx = group_dropdown.selected() as usize;
                let groups_data_ref = groups_data.borrow();
                let group_id = if selected_group_idx < groups_data_ref.len() {
                    groups_data_ref[selected_group_idx].0
                } else {
                    None
                };
                drop(groups_data_ref);
                crate::state::generate_store_key_for_connection(
                    &conn_name,
                    &conn_host,
                    protocol_suffix,
                    backend_type,
                    group_id,
                    &groups,
                )
            };

            match selected {
                1 => {
                    // A saved connection whose lookup fields are untouched asks
                    // the vault exactly what its connect asks, so the session
                    // cache (keyed by connection) answers a repeat Load, and a
                    // vault answer is kept for the next Load, Copy or Connect.
                    // Any edit, or a new connection, goes to the vault.
                    let cache_target = {
                        let selected_group_idx = group_dropdown.selected() as usize;
                        let group_id = groups_data
                            .borrow()
                            .get(selected_group_idx)
                            .and_then(|(id, _)| *id);
                        let username = username_entry.text();
                        let domain = domain_entry.text();
                        let fields = LoadLookup {
                            name: &conn_name,
                            host: &conn_host,
                            protocol: protocol_suffix,
                            group_id,
                            username: username.as_str(),
                            domain: domain.as_str(),
                        };
                        (*editing_id.borrow()).and_then(|id| {
                            let state_ref = state.try_borrow().ok()?;
                            let saved = state_ref.get_connection(id)?;
                            load_matches_saved(saved, &fields).then(|| CacheTarget {
                                connection_id: id,
                                protocol: saved.protocol_config.protocol_type(),
                                username: fields.username.to_string(),
                                domain: fields.domain.to_string(),
                            })
                        })
                    };
                    if let Some(target) = &cache_target
                        && fill_from_cache(
                            &state,
                            target.connection_id,
                            &password_entry,
                            &username_entry,
                            &domain_entry,
                        )
                    {
                        return;
                    }

                    // Vault — delegate to configured backend
                    let password_entry = password_entry.clone();
                    let window = window.clone();
                    let btn = btn.clone();
                    let state = state.clone();
                    let kdbx_enabled = kdbx_enabled;
                    let selected_group_id = groups_data
                        .borrow()
                        .get(group_dropdown.selected() as usize)
                        .and_then(|(id, _)| *id);
                    let kdbx_path = dialog_kdbx_path(
                        &kdbx_override_entry,
                        selected_group_id,
                        groups.as_slice(),
                        kdbx_path.as_ref(),
                    );
                    let kdbx_password = kdbx_password.clone();
                    let kdbx_key_file = kdbx_key_file.clone();
                    let kdbx_yubikey_slot = secret_settings.kdbx_yubikey_slot.clone();
                    let kdbx_root_search = secret_settings.kdbx_root_search;
                    let lookup_key = lookup_key.clone();
                    let flat_lookup_key = flat_lookup_key.clone();
                    let conn_name = conn_name.clone();
                    let conn_host = conn_host.clone();

                    btn.set_sensitive(false);
                    btn.set_icon_name("content-loading-symbolic");

                    // Try KeePass first if enabled, then fall back to
                    // Keyring/Bitwarden/etc.
                    if kdbx_enabled
                        && matches!(
                            secret_settings.preferred_backend,
                            rustconn_core::config::SecretBackendType::KeePassXc
                                | rustconn_core::config::SecretBackendType::KdbxFile
                        )
                    {
                        if let Some(ref kdbx_path) = kdbx_path {
                            let kdbx_path = kdbx_path.clone();
                            let db_password = kdbx_password.clone();
                            let key_file = kdbx_key_file.clone();
                            let yubikey_slot = kdbx_yubikey_slot.clone();

                            spawn_blocking_with_callback(
                                move || {
                                    // Same scoped-then-root read as a connect, so
                                    // this button finds what connecting finds
                                    // (issue #353).
                                    rustconn_core::secret::KeePassStatus
                                        ::get_password_from_kdbx_scoped_then_root(
                                            &kdbx_path,
                                            db_password.as_ref(),
                                            key_file.as_deref(),
                                            &lookup_key,
                                            &[conn_host.as_str(), conn_name.as_str()],
                                            kdbx_root_search,
                                            yubikey_slot.as_deref(),
                                        )
                                },
                                move |result: rustconn_core::error::SecretResult<
                                    Option<secrecy::SecretString>,
                                >| {
                                    btn.set_sensitive(true);
                                    btn.set_icon_name("document-open-symbolic");

                                    match result {
                                        Ok(Some(password)) => {
                                            use secrecy::ExposeSecret;
                                            password_entry.set_text(password.expose_secret());
                                            // A KeePass entry supplies the password
                                            // alone; a connect pairs it with the
                                            // connection's own user name and domain.
                                            if let Some(target) = cache_target {
                                                let non_empty = |s: String| {
                                                    Some(s).filter(|s| !s.trim().is_empty())
                                                };
                                                crate::state::cache_resolved_credentials(
                                                    &state,
                                                    target.connection_id,
                                                    target.protocol,
                                                    &rustconn_core::models::Credentials {
                                                        username: non_empty(target.username),
                                                        password: Some(password),
                                                        domain: non_empty(target.domain),
                                                    },
                                                );
                                            }
                                        }
                                        Ok(None) => {
                                            alert::show_error(
                                                &window,
                                                &i18n("Password Not Found"),
                                                &i18n("No password found in vault for this connection."),
                                            );
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "Failed to load password from vault: {e}"
                                            );
                                            alert::show_error(
                                                &window,
                                                &i18n("Failed to Load Password"),
                                                &i18n("Could not load password from vault."),
                                            );
                                        }
                                    }
                                },
                            );
                        } else {
                            btn.set_sensitive(true);
                            btn.set_icon_name("document-open-symbolic");
                            alert::show_error(
                                &window,
                                &i18n("Vault Not Configured"),
                                &i18n("Configure a secret backend in Settings → Secrets."),
                            );
                        }
                    } else {
                        // Every other backend goes through the shared dispatcher,
                        // the one a connect uses. This used to build each backend
                        // inline and call a plain `retrieve`, which handed a
                        // root-searching backend no name or host to match, and
                        // ran a Bitwarden unlock outside the vault lock (#353).
                        let secret_settings = secret_settings.clone();
                        let username_entry = username_entry.clone();
                        let domain_entry = domain_entry.clone();
                        spawn_blocking_with_callback(
                            move || {
                                crate::state::dispatch_vault_op(
                                    &secret_settings,
                                    &flat_lookup_key,
                                    crate::state::VaultOp::RetrieveFor {
                                        name: &conn_name,
                                        host: &conn_host,
                                    },
                                )
                            },
                            move |result: Result<
                                Option<rustconn_core::models::Credentials>,
                                String,
                            >| {
                                btn.set_sensitive(true);
                                btn.set_icon_name("document-open-symbolic");

                                match result {
                                    Ok(Some(creds)) => {
                                        if let Some(password) = creds.expose_password() {
                                            password_entry.set_text(password);
                                            // Same rule a connect applies: a
                                            // non-empty vault user name or
                                            // domain replaces what is typed
                                            // here, an empty one keeps it.
                                            let identity =
                                                rustconn_core::secret::merge_vault_identity(
                                                    Some(username_entry.text().as_str()),
                                                    Some(domain_entry.text().as_str()),
                                                    creds.username.as_deref(),
                                                    creds.domain.as_deref(),
                                                );
                                            if let Some(username) = &identity.username {
                                                username_entry.set_text(username);
                                            }
                                            if let Some(domain) = &identity.domain {
                                                domain_entry.set_text(domain);
                                            }
                                            // The fields were the saved ones, so
                                            // this merge is the one a connect
                                            // makes; keep its answer.
                                            if let Some(target) = &cache_target {
                                                crate::state::cache_resolved_credentials(
                                                    &state,
                                                    target.connection_id,
                                                    target.protocol,
                                                    &rustconn_core::models::Credentials {
                                                        username: identity.username,
                                                        password: creds.password.clone(),
                                                        domain: identity.domain,
                                                    },
                                                );
                                            }
                                        } else {
                                            alert::show_error(
                                                &window,
                                                &i18n("Password Not Found"),
                                                &i18n("No password found in vault for this connection."),
                                            );
                                        }
                                    }
                                    Ok(None) => {
                                        alert::show_error(
                                            &window,
                                            &i18n("Password Not Found"),
                                            &i18n("No password found in vault for this connection."),
                                        );
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to load password from vault: {e}");
                                        alert::show_error(
                                            &window,
                                            &i18n("Failed to Load Password"),
                                            &i18n("Could not load password from vault."),
                                        );
                                    }
                                }
                            },
                        );
                    }
                }
                _ => {
                    // Prompt(0), Variable(2), Inherit(3), None(4)
                    alert::show_error(
                        &window,
                        &i18n("Cannot Load Password"),
                        &i18n("Password loading is only available for Vault source."),
                    );
                }
            }
        });
    }

    /// Wires up the "Test credential resolution" button.
    ///
    /// When clicked, performs a vault lookup using the current connection name,
    /// host, protocol, and group — then shows a success/failure dialog with
    /// the lookup key used. Helps users verify their vault configuration
    /// before connecting.
    pub fn connect_vault_test_button(
        &self,
        kdbx_enabled: bool,
        kdbx_path: Option<std::path::PathBuf>,
        kdbx_password: Option<&secrecy::SecretString>,
        kdbx_key_file: Option<std::path::PathBuf>,
        groups: Vec<rustconn_core::models::ConnectionGroup>,
        secret_settings: rustconn_core::config::SecretSettings,
    ) {
        use crate::utils::spawn_blocking_with_callback;

        let password_source_dropdown = self.password_source_dropdown.clone();
        let name_entry = self.name_entry.clone();
        let host_entry = self.host_entry.clone();
        let protocol_dropdown = self.protocol_dropdown.clone();
        let group_dropdown = self.group_dropdown.clone();
        let groups_data = self.groups_data.clone();
        let window = self.dialog.clone();
        let kdbx_password = kdbx_password.cloned();
        let kdbx_override_entry = self.kdbx_override_entry.clone();
        let groups = Rc::new(groups);

        self.vault_test_button.connect_clicked(move |btn| {
            let selected = password_source_dropdown.selected();
            if selected != 1 {
                // Only test for Vault source
                alert::show_error(
                    &window,
                    &i18n("Test Not Available"),
                    &i18n("Credential test is only available when Password Source is set to Vault."),
                );
                return;
            }

            let conn_name = name_entry.text().to_string();
            let conn_host = host_entry.text().to_string();
            let protocol_index = protocol_dropdown.selected();

            let base_name = if conn_name.trim().is_empty() {
                conn_host.clone()
            } else {
                conn_name.clone()
            };

            if base_name.trim().is_empty() {
                alert::show_error(
                    &window,
                    &i18n("Cannot Test"),
                    &i18n("Enter a connection name or host first."),
                );
                return;
            }

            let protocol_suffix = match protocol_index {
                0 => "ssh",
                1 => "rdp",
                2 => "vnc",
                3 => "spice",
                4 => "zerotrust",
                5 => "telnet",
                6 => "serial",
                7 => "sftp",
                8 => "kubernetes",
                9 => "mosh",
                10 => "web",
                11 => "winrm",
                _ => "ssh",
            };

            // Build hierarchical lookup key for KeePass
            let lookup_key = if groups.is_empty() {
                let sanitized_name = base_name.replace('/', "-");
                format!("{sanitized_name} ({protocol_suffix})")
            } else {
                let selected_group_idx = group_dropdown.selected() as usize;
                let groups_data_ref = groups_data.borrow();
                let group_id = if selected_group_idx < groups_data_ref.len() {
                    groups_data_ref[selected_group_idx].0
                } else {
                    None
                };
                drop(groups_data_ref);

                let group_path = if let Some(gid) = group_id {
                    rustconn_core::secret::KeePassHierarchy::resolve_group_path(gid, &groups)
                } else {
                    Vec::new()
                };

                if group_path.is_empty() {
                    format!("{base_name} ({protocol_suffix})")
                } else {
                    let path = group_path.join("/");
                    format!("{path}/{base_name} ({protocol_suffix})")
                }
            };

            // Same key as the 📂 load button above, and the same issue #316 fix:
            // an ungrouped connection is `RustConn/{name} ({protocol})`, not the
            // bare legacy key its `None` group path used to ask for.
            let flat_lookup_key = {
                let backend_type = crate::state::select_backend_for_load(&secret_settings);
                let selected_group_idx = group_dropdown.selected() as usize;
                let groups_data_ref = groups_data.borrow();
                let group_id = if selected_group_idx < groups_data_ref.len() {
                    groups_data_ref[selected_group_idx].0
                } else {
                    None
                };
                drop(groups_data_ref);
                crate::state::generate_store_key_for_connection(
                    &conn_name,
                    &conn_host,
                    protocol_suffix,
                    backend_type,
                    group_id,
                    &groups,
                )
            };

            btn.set_sensitive(false);
            btn.set_icon_name("content-loading-symbolic");

            let btn_clone = btn.clone();
            let window_clone = window.clone();
            let lookup_key_display = lookup_key.clone();
            let flat_key_display = flat_lookup_key.clone();

            if kdbx_enabled
                && matches!(
                    secret_settings.preferred_backend,
                    rustconn_core::config::SecretBackendType::KeePassXc
                        | rustconn_core::config::SecretBackendType::KdbxFile
                )
            {
                let selected_group_id = groups_data
                    .borrow()
                    .get(group_dropdown.selected() as usize)
                    .and_then(|(id, _)| *id);
                let effective_kdbx_path = dialog_kdbx_path(
                    &kdbx_override_entry,
                    selected_group_id,
                    groups.as_slice(),
                    kdbx_path.as_ref(),
                );
                if let Some(kdbx_path) = effective_kdbx_path {
                    let db_password = kdbx_password.clone();
                    let key_file = kdbx_key_file.clone();
                    let yubikey_slot = secret_settings.kdbx_yubikey_slot.clone();
                    let root_search = secret_settings.kdbx_root_search;

                    spawn_blocking_with_callback(
                        move || {
                            // The read a connect does, root search included —
                            // a test that skips it fails for an entry the
                            // connect then finds (issue #353).
                            rustconn_core::secret::KeePassStatus::get_password_from_kdbx_scoped_then_root(
                                &kdbx_path,
                                db_password.as_ref(),
                                key_file.as_deref(),
                                &lookup_key,
                                &[conn_host.as_str(), conn_name.as_str()],
                                root_search,
                                yubikey_slot.as_deref(),
                            )
                        },
                        move |result: rustconn_core::error::SecretResult<
                            Option<secrecy::SecretString>,
                        >| {
                            btn_clone.set_sensitive(true);
                            btn_clone.set_icon_name("object-select-symbolic");

                            match result {
                                Ok(Some(_)) => {
                                    alert::show_success(
                                        &window_clone,
                                        &i18n("Credential Test Passed"),
                                        &i18n_f(
                                            "Password found in vault.\nLookup key: {}",
                                            &[&lookup_key_display],
                                        ),
                                    );
                                }
                                Ok(None) => {
                                    alert::show_error(
                                        &window_clone,
                                        &i18n("Credential Test Failed"),
                                        &i18n_f(
                                            "No password found in vault.\nLookup key: {}",
                                            &[&lookup_key_display],
                                        ),
                                    );
                                }
                                Err(e) => {
                                    tracing::error!("Vault test failed: {e}");
                                    alert::show_error(
                                        &window_clone,
                                        &i18n("Credential Test Failed"),
                                        &i18n_f(
                                            "Vault error. Check your KeePass configuration.\nLookup key: {}",
                                            &[&lookup_key_display],
                                        ),
                                    );
                                }
                            }
                        },
                    );
                } else {
                    btn.set_sensitive(true);
                    btn.set_icon_name("object-select-symbolic");
                    alert::show_error(
                        &window,
                        &i18n("Vault Not Configured"),
                        &i18n("Configure a secret backend in Settings → Secrets."),
                    );
                }
            } else {
                // Non-KeePass backend
                let secret_settings = secret_settings.clone();
                spawn_blocking_with_callback(
                    move || {
                        crate::state::dispatch_vault_op(
                            &secret_settings,
                            &flat_lookup_key,
                            crate::state::VaultOp::RetrieveFor {
                                name: &conn_name,
                                host: &conn_host,
                            },
                        )
                    },
                    move |result: Result<
                        Option<rustconn_core::models::Credentials>,
                        String,
                    >| {
                        btn_clone.set_sensitive(true);
                        btn_clone.set_icon_name("object-select-symbolic");

                        match result {
                            Ok(Some(creds)) => {
                                let has_pw = creds.expose_password().is_some();
                                if has_pw {
                                    alert::show_success(
                                        &window_clone,
                                        &i18n("Credential Test Passed"),
                                        &i18n_f(
                                            "Password found in vault.\nLookup key: {}",
                                            &[&flat_key_display],
                                        ),
                                    );
                                } else {
                                    alert::show_error(
                                        &window_clone,
                                        &i18n("Credential Test Failed"),
                                        &i18n_f(
                                            "Entry found but contains no password.\nLookup key: {}",
                                            &[&flat_key_display],
                                        ),
                                    );
                                }
                            }
                            Ok(None) => {
                                alert::show_error(
                                    &window_clone,
                                    &i18n("Credential Test Failed"),
                                    &i18n_f(
                                        "No entry found in vault.\nLookup key: {}",
                                        &[&flat_key_display],
                                    ),
                                );
                            }
                            Err(e) => {
                                tracing::error!("Vault test failed: {e}");
                                alert::show_error(
                                    &window_clone,
                                    &i18n("Credential Test Failed"),
                                    &i18n_f(
                                        "Backend error. Check your vault configuration.\nLookup key: {}",
                                        &[&flat_key_display],
                                    ),
                                );
                            }
                        }
                    },
                );
            }
        });
    }

    /// Returns the password entry widget for external access
    #[must_use]
    pub const fn password_entry(&self) -> &adw::PasswordEntryRow {
        &self.password_entry
    }

    /// Returns the password row widget for external access
    #[must_use]
    pub const fn password_row(&self) -> &GtkBox {
        &self.password_row
    }
}

/// The editor fields "Load password from vault" builds its lookup from.
struct LoadLookup<'a> {
    name: &'a str,
    host: &'a str,
    /// Lowercase protocol id, as in [`rustconn_core::models::ProtocolType::as_str`].
    protocol: &'a str,
    group_id: Option<uuid::Uuid>,
    username: &'a str,
    domain: &'a str,
}

/// The saved connection whose session-cache entry a Load may read and write.
struct CacheTarget {
    connection_id: uuid::Uuid,
    protocol: rustconn_core::models::ProtocolType,
    /// The connection's user name and domain as the editor shows them, which
    /// [`load_matches_saved`] proved equal to the saved ones.
    username: String,
    domain: String,
}

/// Whether a Load for `fields` asks the vault the question a connect of `saved`
/// asks, so the session cache, keyed by connection, holds the same answer.
///
/// Every input of the lookup has to be untouched: name and host (the keys and
/// the root-search aliases), protocol and group (the key), and the user name and
/// domain the vault identity is merged into. The saved source has to be Vault,
/// because a cache entry for a Variable or Script connection holds that
/// source's secret, not the vault's.
fn load_matches_saved(saved: &rustconn_core::models::Connection, fields: &LoadLookup<'_>) -> bool {
    saved.password_source == rustconn_core::models::PasswordSource::Vault
        && saved.name == fields.name
        && saved.host == fields.host
        && saved.protocol_config.protocol_type().as_str() == fields.protocol
        && saved.group_id == fields.group_id
        && saved.username.as_deref().unwrap_or_default() == fields.username
        && saved.domain.as_deref().unwrap_or_default() == fields.domain
}

/// Fills the editor from the session cache entry for `connection_id`.
///
/// Returns `false`, leaving the fields alone, when there is no live entry with
/// a non-empty password; the caller then asks the vault. User name and domain
/// follow the merge rule the vault answer gets.
///
/// The entry does not record where it came from. When the vault had no entry
/// and the user typed the password at the connect prompt, that typed password
/// is what Load shows here, for the rest of the cache TTL. Accepted: it is the
/// password Connect and Copy would use for this connection right now, so Load
/// shows what the next connect sends rather than a different answer.
// ponytail: no source tag on cache entries; add one and skip prompt-sourced
// entries here if Load must show only what the vault holds.
fn fill_from_cache(
    state: &crate::state::SharedAppState,
    connection_id: uuid::Uuid,
    password_entry: &adw::PasswordEntryRow,
    username_entry: &gtk4::Entry,
    domain_entry: &gtk4::Entry,
) -> bool {
    use secrecy::ExposeSecret;

    // Copied out so no state borrow is held while the entries emit `changed`.
    let Some((password, cached_username, cached_domain)) =
        state.try_borrow().ok().and_then(|state_ref| {
            state_ref
                .get_cached_credentials(connection_id)
                .filter(|c| !c.password.expose_secret().is_empty())
                .map(|c| {
                    (
                        zeroize::Zeroizing::new(c.password.expose_secret().to_string()),
                        c.username.clone(),
                        c.domain.clone(),
                    )
                })
        })
    else {
        return false;
    };
    password_entry.set_text(&password);
    let identity = rustconn_core::secret::merge_vault_identity(
        Some(username_entry.text().as_str()),
        Some(domain_entry.text().as_str()),
        Some(cached_username.as_str()),
        Some(cached_domain.as_str()),
    );
    if let Some(username) = identity.username {
        username_entry.set_text(&username);
    }
    if let Some(domain) = identity.domain {
        domain_entry.set_text(&domain);
    }
    tracing::debug!(%connection_id, "Editor password loaded from the session cache");
    true
}

#[cfg(test)]
mod load_cache_tests {
    use rustconn_core::models::{Connection, PasswordSource};

    use super::{LoadLookup, load_matches_saved};

    fn saved() -> Connection {
        let mut conn = Connection::new_rdp("krdp".to_string(), "192.168.0.122".to_string(), 3389);
        conn.password_source = PasswordSource::Vault;
        conn.username = Some("admin".to_string());
        conn
    }

    fn unedited(conn: &Connection) -> LoadLookup<'_> {
        LoadLookup {
            name: &conn.name,
            host: &conn.host,
            protocol: "rdp",
            group_id: conn.group_id,
            username: conn.username.as_deref().unwrap_or_default(),
            domain: conn.domain.as_deref().unwrap_or_default(),
        }
    }

    #[test]
    fn an_unedited_vault_connection_may_use_the_cache() {
        let conn = saved();
        assert!(load_matches_saved(&conn, &unedited(&conn)));
    }

    /// Each input of the lookup, edited alone, sends the Load to the vault.
    #[test]
    fn any_edited_lookup_field_goes_to_the_vault() {
        let conn = saved();
        let group = Some(uuid::Uuid::new_v4());
        let edits: [&dyn Fn(&mut LoadLookup<'_>); 6] = [
            &|f| f.name = "other",
            &|f| f.host = "10.0.0.1",
            &|f| f.protocol = "ssh",
            &|f| f.group_id = group,
            &|f| f.username = "root",
            &|f| f.domain = "CORP",
        ];
        for (i, edit) in edits.iter().enumerate() {
            let mut fields = unedited(&conn);
            edit(&mut fields);
            assert!(!load_matches_saved(&conn, &fields), "edit #{i}");
        }
    }

    /// A cache entry of a connection saved with another source holds that
    /// source's secret, not the vault's.
    #[test]
    fn a_connection_saved_with_another_source_goes_to_the_vault() {
        for source in [
            PasswordSource::Prompt,
            PasswordSource::None,
            PasswordSource::Inherit,
            PasswordSource::Variable("SECRET".to_string()),
            PasswordSource::Script("op read".to_string()),
        ] {
            let mut conn = saved();
            conn.password_source = source.clone();
            assert!(!load_matches_saved(&conn, &unedited(&conn)), "{source:?}");
        }
    }
}
