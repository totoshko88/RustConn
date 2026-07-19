//! SSH agent keys and global/inherited variables
//!
//! Mechanically split out of `dialog.rs` (pure code motion).

#![allow(
    clippy::similar_names,
    reason = "module-wide override for legacy code; refactored case by case"
)]

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{StringList, glib};
use libadwaita as adw;
use rustconn_core::variables::Variable;

use super::ConnectionDialog;
use crate::i18n::i18n;

impl ConnectionDialog {
    /// Refreshes the SSH agent key list asynchronously.
    ///
    /// Spawns the agent probe on a background thread with a 5-second timeout so
    /// the GTK main thread is never blocked — even if the system ssh-agent is
    /// broken or launchd-throttled (common on macOS).
    ///
    /// This should be called before showing the dialog to populate the agent key dropdown
    /// with the currently loaded keys from the SSH agent.
    pub fn refresh_agent_keys(&self) {
        use rustconn_core::ssh_agent::SshAgentManager;

        // 5-second timeout — enough for a healthy agent, prevents indefinite hang.
        const AGENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

        // Show a placeholder while loading
        let loading_items: Vec<String> = vec![i18n("Loading agent keys…")];
        let loading_list =
            StringList::new(&loading_items.iter().map(String::as_str).collect::<Vec<_>>());
        self.ssh_agent_key_dropdown.set_model(Some(&loading_list));
        self.ssh_agent_key_dropdown.set_selected(0);
        self.ssh_agent_key_dropdown.set_sensitive(false);

        // Clone the Rc fields needed to update UI after async completion
        let ssh_agent_keys = self.ssh_agent_keys.clone();
        let ssh_agent_key_dropdown = self.ssh_agent_key_dropdown.clone();
        let ssh_key_source_dropdown = self.ssh_key_source_dropdown.clone();
        let pending_agent_selection = self.pending_agent_selection.clone();

        // Read the socket path from environment now (cheap, no blocking)
        let manager = SshAgentManager::from_env();

        glib::spawn_future_local(async move {
            // Run the blocking agent probe on a background thread
            let keys = gtk4::gio::spawn_blocking(move || {
                match manager.get_status_timeout(AGENT_TIMEOUT) {
                    Ok(status) if status.running => status.keys,
                    _ => Vec::new(),
                }
            })
            .await
            .unwrap_or_default();

            // Back on GTK main thread — update the UI
            *ssh_agent_keys.borrow_mut() = keys.clone();

            let items: Vec<String> = if keys.is_empty() {
                vec![i18n("No keys loaded")]
            } else {
                keys.iter().map(Self::format_agent_key_short).collect()
            };

            let string_list =
                StringList::new(&items.iter().map(String::as_str).collect::<Vec<_>>());
            ssh_agent_key_dropdown.set_model(Some(&string_list));
            ssh_agent_key_dropdown.set_selected(0);

            // Update sensitivity based on whether keys are available
            let has_keys = !keys.is_empty();
            if ssh_key_source_dropdown.selected() == 2 {
                // Agent source is selected
                ssh_agent_key_dropdown.set_sensitive(has_keys);
            }

            // Restore pending agent key selection (set by set_ssh_config before keys were loaded)
            if let Some((ref fingerprint, ref comment)) = *pending_agent_selection.borrow() {
                let keys_ref = ssh_agent_keys.borrow();
                for (idx, key) in keys_ref.iter().enumerate() {
                    if key.fingerprint == *fingerprint || key.comment == *comment {
                        #[expect(
                            clippy::cast_possible_truncation,
                            reason = "agent key count always fits u32"
                        )]
                        ssh_agent_key_dropdown.set_selected(idx as u32);
                        break;
                    }
                }
            }
        });
    }

    /// Formats an agent key for short display in dropdown button
    /// Shows: "comment_start...comment_end (TYPE)"
    pub(super) fn format_agent_key_short(key: &rustconn_core::ssh_agent::AgentKey) -> String {
        let comment = &key.comment;
        let max_comment_len = 24;

        let short_comment = if comment.len() > max_comment_len {
            // Show first 10 and last 10 chars with ellipsis
            let start = &comment[..10];
            let end = &comment[comment.len() - 10..];
            format!("{start}…{end}")
        } else {
            comment.clone()
        };

        format!("{short_comment} ({})", key.key_type)
    }

    /// Sets the global variables to display as inherited in the Variables tab
    ///
    /// This should be called before `set_connection` to properly show
    /// which variables are inherited from global scope.
    pub fn set_global_variables(&self, variables: &[Variable]) {
        *self.global_variables.borrow_mut() = variables.to_vec();

        // Populate variable dropdown with secret global variables
        if let Some(model) = self.variable_dropdown.model()
            && let Some(sl) = model.downcast_ref::<gtk4::StringList>()
        {
            // Clear existing items
            sl.splice(0, sl.n_items(), &[] as &[&str]);
            // Add secret variables only
            let mut has_secrets = false;
            for var in variables {
                if var.is_secret {
                    sl.append(&var.name);
                    has_secrets = true;
                }
            }
            // Show placeholder when no secret variables are defined
            if has_secrets {
                self.variable_dropdown.set_sensitive(true);
            } else {
                sl.append(&i18n("(no secret variables)"));
                self.variable_dropdown.set_sensitive(false);
            }
        }
    }

    /// Populates the Variables tab with inherited global variables
    ///
    /// Call this after `set_global_variables` to show global variables
    /// that can be overridden locally.
    pub fn populate_inherited_variables(&self) {
        // Clear existing rows first
        while let Some(row) = self.variables_list.row_at_index(0) {
            self.variables_list.remove(&row);
        }
        self.variables_rows.borrow_mut().clear();

        // Add rows for each global variable (as inherited, read-only name)
        let global_vars = self.global_variables.borrow();
        for var in global_vars.iter() {
            // Create a row showing the global variable with empty value
            // (user can fill in to override)
            let mut display_var = var.clone();
            display_var.value = String::new(); // Clear value so user can override
            self.add_local_variable_row(Some(&display_var), true);
        }
    }

    /// Sets the preferred secret backend and updates the password source dropdown
    ///
    /// This method sets the default selection in the password source dropdown
    /// based on the configured secret backend in Settings:
    /// - All backends map to Vault (index 1)
    ///
    /// The dropdown shows: Prompt(0), Vault(1), Variable(2), Inherit(3), None(4)
    pub fn set_preferred_backend(&self, _backend: rustconn_core::config::SecretBackendType) {
        // All backends map to Vault (index 1)
        self.password_source_dropdown.set_selected(1);

        // Update password row visibility based on new selection
        self.update_password_row_visibility();
    }

    /// Configures the dialog for Import group mode.
    ///
    /// Synced fields become read-only (insensitive) with a tooltip explaining
    /// they are managed by cloud sync. Local-only fields remain editable.
    ///
    /// This follows the GNOME HIG pattern of using insensitive widgets with
    /// descriptive tooltips rather than replacing the entire dialog layout.
    pub fn configure_import_group_mode(&self) {
        use crate::i18n::i18n;

        let managed_tooltip = i18n("Managed by cloud sync");

        // Synced fields → read-only (insensitive + tooltip)
        // Name, host, port, protocol, username, domain, tags, description
        self.name_entry.set_sensitive(false);
        self.name_entry.set_tooltip_text(Some(&managed_tooltip));

        self.host_entry.set_sensitive(false);
        self.host_entry.set_tooltip_text(Some(&managed_tooltip));

        self.port_spin.set_sensitive(false);
        self.port_spin.set_tooltip_text(Some(&managed_tooltip));

        self.protocol_dropdown.set_sensitive(false);
        self.protocol_dropdown
            .set_tooltip_text(Some(&managed_tooltip));

        self.username_entry.set_sensitive(false);
        self.username_entry.set_tooltip_text(Some(&managed_tooltip));

        self.domain_entry.set_sensitive(false);
        self.domain_entry.set_tooltip_text(Some(&managed_tooltip));

        self.tags_entry.set_sensitive(false);
        self.tags_entry.set_tooltip_text(Some(&managed_tooltip));

        self.description_view.set_sensitive(false);
        self.description_view
            .set_tooltip_text(Some(&managed_tooltip));

        self.icon_entry.set_sensitive(false);
        self.icon_entry.set_tooltip_text(Some(&managed_tooltip));

        // Group dropdown → read-only (can't move connection between groups)
        self.group_dropdown.set_sensitive(false);
        self.group_dropdown.set_tooltip_text(Some(&managed_tooltip));

        // Local-only fields remain editable:
        // password_source_dropdown, ssh_key_entry, ssh_key_source_dropdown
        // (these are already editable by default — no changes needed)

        // Update window title to indicate Import group mode
        self.dialog.set_title(&i18n("Edit Connection (Synced)"));
    }
}
