//! Password source, vault integration and visibility wiring
//!
//! Mechanically split out of `dialog.rs` (pure code motion).

#![allow(
    clippy::similar_names,
    reason = "module-wide override for legacy code; refactored case by case"
)]

use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Entry};

use super::ConnectionDialog;
use crate::alert;
use crate::i18n::{i18n, i18n_f};

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

    /// Connects password visibility toggle button
    pub fn connect_password_visibility_toggle(&self) {
        use std::cell::Cell;

        let password_entry = self.password_entry.clone();
        // Track visibility state - starts hidden (false)
        let is_visible = Rc::new(Cell::new(false));

        self.password_visibility_button.connect_clicked(move |btn| {
            let currently_visible = is_visible.get();
            let new_visible = !currently_visible;
            is_visible.set(new_visible);
            password_entry.set_visibility(new_visible);
            // Update icon
            if new_visible {
                btn.set_icon_name("view-conceal-symbolic");
            } else {
                btn.set_icon_name("view-reveal-symbolic");
            }
        });
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
        let name_entry = self.name_entry.clone();
        let host_entry = self.host_entry.clone();
        let protocol_dropdown = self.protocol_dropdown.clone();
        let group_dropdown = self.group_dropdown.clone();
        let groups_data = self.groups_data.clone();
        let window = self.dialog.clone();
        let kdbx_password = kdbx_password.cloned();

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

            // Flat lookup key — must match the format used by
            // `generate_store_key` so that store and retrieve are consistent.
            // LibSecret uses "{name} ({protocol})", while Bitwarden and other
            // backends use "rustconn/{name}".
            let flat_lookup_key = {
                let backend_type =
                    crate::state::select_backend_for_load(&secret_settings);
                crate::state::generate_store_key(
                    &conn_name,
                    &conn_host,
                    protocol_suffix,
                    backend_type,
                )
            };

            match selected {
                1 => {
                    // Vault — delegate to configured backend
                    let password_entry = password_entry.clone();
                    let window = window.clone();
                    let btn = btn.clone();
                    let kdbx_enabled = kdbx_enabled;
                    let kdbx_path = kdbx_path.clone();
                    let kdbx_password = kdbx_password.clone();
                    let kdbx_key_file = kdbx_key_file.clone();
                    let lookup_key = lookup_key.clone();
                    let flat_lookup_key = flat_lookup_key.clone();

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

                            spawn_blocking_with_callback(
                                move || {
                                    rustconn_core::secret::KeePassStatus
                                        ::get_password_from_kdbx_with_key(
                                            &kdbx_path,
                                            db_password.as_ref(),
                                            key_file.as_deref(),
                                            &lookup_key,
                                            None,
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
                        // Non-KeePass backend — dispatch based on
                        // preferred_backend
                        let secret_settings = secret_settings.clone();
                        spawn_blocking_with_callback(
                            move || {
                                use rustconn_core::config::SecretBackendType;
                                use rustconn_core::secret::SecretBackend;

                                let backend_type =
                                    crate::state::select_backend_for_load(&secret_settings);

                                match backend_type {
                                    SecretBackendType::Bitwarden => {
                                        crate::async_utils::with_runtime(|rt| {
                                            let backend = rt
                                                .block_on(rustconn_core::secret::auto_unlock(
                                                    &secret_settings,
                                                ))
                                                .map_err(|e| format!("{e}"))?;
                                            rt.block_on(backend.retrieve(&flat_lookup_key))
                                                .map_err(|e| format!("{e}"))
                                        })?
                                    }
                                    SecretBackendType::OnePassword => {
                                        let mut backend =
                                            rustconn_core::secret::OnePasswordBackend::new();
                                        if let Some(ref token) =
                                            secret_settings
                                                .onepassword_service_account_token
                                        {
                                            backend.set_service_account_token(token.clone());
                                        }
                                        crate::async_utils::with_runtime(|rt| {
                                            rt.block_on(backend.retrieve(&flat_lookup_key))
                                                .map_err(|e| format!("{e}"))
                                        })?
                                    }
                                    SecretBackendType::Passbolt => {
                                        let mut backend =
                                            rustconn_core::secret::PassboltBackend::new();
                                        if let Some(ref url) =
                                            secret_settings.passbolt_server_url
                                        {
                                            backend =
                                                backend.with_server_address(url.clone());
                                        }
                                        if let Some(ref passphrase) =
                                            secret_settings.passbolt_passphrase
                                        {
                                            backend =
                                                backend.with_user_password(passphrase.clone());
                                        }
                                        crate::async_utils::with_runtime(|rt| {
                                            rt.block_on(backend.retrieve(&flat_lookup_key))
                                                .map_err(|e| format!("{e}"))
                                        })?
                                    }
                                    SecretBackendType::Pass => {
                                        let backend =
                                            rustconn_core::secret::PassBackend::from_secret_settings(
                                                &secret_settings,
                                            );
                                        crate::async_utils::with_runtime(|rt| {
                                            rt.block_on(backend.retrieve(&flat_lookup_key))
                                                .map_err(|e| format!("{e}"))
                                        })?
                                    }
                                    SecretBackendType::LibSecret
                                    | SecretBackendType::MacOsKeychain
                                    | SecretBackendType::KeePassXc
                                    | SecretBackendType::KdbxFile => {
                                        // macOS uses the Keychain; LibSecretBackend
                                        // (oo7) is not compiled there (R10.1, R10.2).
                                        #[cfg(target_os = "macos")]
                                        let backend =
                                            rustconn_core::secret::MacOsKeychainBackend::new();
                                        #[cfg(not(target_os = "macos"))]
                                        let backend = rustconn_core::secret::LibSecretBackend::new(
                                            "rustconn",
                                        );
                                        crate::async_utils::with_runtime(|rt| {
                                            rt.block_on(backend.retrieve(&flat_lookup_key))
                                                .map_err(|e| format!("{e}"))
                                        })?
                                    }
                                    SecretBackendType::EncryptedFile => {
                                        // Application-managed encrypted file:
                                        // retrieve by the flat lookup key, same
                                        // key scheme as the other app-managed
                                        // backends.
                                        let backend =
                                            rustconn_core::secret::EncryptedFileBackend::new();
                                        crate::async_utils::with_runtime(|rt| {
                                            rt.block_on(backend.retrieve(&flat_lookup_key))
                                                .map_err(|e| format!("{e}"))
                                        })?
                                    }
                                }
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

            let flat_lookup_key = {
                let backend_type = crate::state::select_backend_for_load(&secret_settings);
                crate::state::generate_store_key(
                    &conn_name,
                    &conn_host,
                    protocol_suffix,
                    backend_type,
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
                if let Some(ref kdbx_path) = kdbx_path {
                    let kdbx_path = kdbx_path.clone();
                    let db_password = kdbx_password.clone();
                    let key_file = kdbx_key_file.clone();

                    spawn_blocking_with_callback(
                        move || {
                            rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
                                &kdbx_path,
                                db_password.as_ref(),
                                key_file.as_deref(),
                                &lookup_key,
                                None,
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
                            crate::state::VaultOp::Retrieve,
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
    pub const fn password_entry(&self) -> &Entry {
        &self.password_entry
    }

    /// Returns the password row widget for external access
    #[must_use]
    pub const fn password_row(&self) -> &GtkBox {
        &self.password_row
    }
}
