//! Settings dialog glue and post-save checks

use super::*;

impl MainWindow {
    #[expect(
        clippy::too_many_arguments,
        reason = "dialog launcher wiring independent window dependencies into the settings dialog and its live-apply closure; a struct would only restate them"
    )]
    pub(super) fn show_settings_dialog(
        window: &adw::ApplicationWindow,
        state: SharedAppState,
        notebook: SharedNotebook,
        monitoring: Rc<crate::monitoring::MonitoringCoordinator>,
        sidebar: SharedSidebar,
        overlay_split_view: adw::OverlaySplitView,
        session_split_bridges: SessionSplitBridges,
        header_title: adw::WindowTitle,
        open_secrets_page: bool,
    ) {
        let opened_at = std::time::Instant::now();
        tracing::debug!("settings action activated");
        let mut dialog = SettingsDialog::new(None);

        // Load current settings and connections
        {
            let state_ref = state.borrow();
            // Connections first: `set_settings` populates the startup-action and
            // global jump-host dropdowns from this list, so it has to be there
            // already. The old order left both showing nothing to pick.
            let connections: Vec<_> = state_ref.list_connections_owned();
            dialog.set_connections(connections);
            dialog.set_settings(state_ref.settings().clone());

            // Populate Cloud Sync sections
            let groups: Vec<_> = state_ref.list_groups_owned();
            dialog.populate_cloud_sync(&groups, state_ref.sync_manager(), &state);
        }
        // Outside the borrow above: the handler takes its own snapshot when the
        // button is pressed, and holding a `Ref` while wiring it would mean the
        // dialog owned one for as long as it was open.
        dialog.connect_credential_transfer(&state);
        dialog.connect_portable_passphrase_change(&state);
        dialog.connect_monitoring_override_reset(&state);
        if open_secrets_page {
            dialog.show_secrets_page();
        }
        tracing::debug!(
            elapsed_ms = opened_at.elapsed().as_millis() as u64,
            "settings dialog constructed and populated"
        );

        let window_clone = window.clone();
        let header_title_for_apply = header_title.clone();
        dialog.run(Some(window), move |result| {
            if let Some(settings) = result {
                // Capture backend and KeePass state for action update
                let backend = settings.secrets.preferred_backend;
                let keepass_enabled = settings.secrets.kdbx_enabled;
                let kdbx_path_exists = settings
                    .secrets
                    .kdbx_path
                    .as_ref()
                    .is_some_and(|p: &std::path::PathBuf| p.exists());

                // Apply terminal settings to existing terminals
                notebook.apply_settings(&settings.terminal);

                // Re-apply per-connection theme overrides that were wiped
                // by the global theme application above (fixes #99)
                {
                    let state_ref = state.borrow();
                    notebook.reapply_theme_overrides(
                        &settings.terminal.color_theme,
                        |connection_id| {
                            state_ref
                                .get_connection(connection_id)
                                .and_then(|c| c.theme_override.clone())
                        },
                    );
                    // Same story for the per-connection Backspace/Delete bytes:
                    // apply_settings reinstalled the global erase bindings over
                    // them (issue #271).
                    notebook.reapply_erase_modes(|connection_id| {
                        state_ref
                            .get_connection(connection_id)
                            .map(|c| c.protocol_config.erase_modes())
                    });
                    // Highlight rules: the global rules and the built-in toggle
                    // used to reach only sessions started afterwards, so turning
                    // the built-ins off changed nothing on screen (issue #343).
                    notebook.reapply_highlight_rules(
                        &settings.highlight_rules,
                        !settings.highlight_builtin_defaults_disabled,
                        |connection_id| {
                            state_ref
                                .get_connection(connection_id)
                                .map(|c| c.highlight_rules.clone())
                        },
                    );
                }

                // Apply reconnect history retention setting (#253)
                notebook.set_keep_history_on_reconnect(settings.terminal.keep_history_on_reconnect);
                notebook
                    .set_max_scrollback_on_reconnect(settings.terminal.max_scrollback_on_reconnect);

                // Apply smart folders visibility setting
                sidebar.set_smart_folders_visible(settings.ui.show_smart_folders);

                // Apply the tab menu's directional-close visibility setting
                notebook.set_show_directional_close(settings.ui.show_directional_tab_close);

                // Apply the auto-capture shortcuts setting for remote desktop sessions
                notebook.set_auto_capture_shortcuts(settings.ui.remote_desktop_auto_capture_shortcuts);

                // Apply the sidebar width (issue #3: one value, 180..=500) and show
                // or hide the drag handle to match the opt-in toggle.
                if let Some(w) = settings.ui.sidebar_width {
                    overlay_split_view
                        .set_max_sidebar_width(f64::from(rustconn_core::config::clamp_sidebar_width(w)));
                }
                sidebar_resize::apply_drag_resize_setting(
                    &overlay_split_view,
                    settings.ui.sidebar_drag_resize,
                );

                // Apply monitoring settings to active bars
                monitoring.apply_settings_to_all(&settings.monitoring);

                // Apply split-pane connection name labels to all active bridges (issue #277)
                {
                    let bridges = session_split_bridges.borrow();
                    for bridge in bridges.values() {
                        bridge.set_show_pane_labels(settings.ui.show_split_pane_labels);
                        // A split shown in a group window keeps its corner controls
                        // hidden: they act on the main window (see `detach_actions`).
                        let in_group_window = bridge
                            .owner_session()
                            .is_some_and(|owner| notebook.is_detached(owner));
                        bridge.set_hide_hover_controls(
                            in_group_window || settings.ui.hide_split_pane_hover_controls,
                        );
                    }
                }

                // Apply window-title-shows-connection setting live (issue #211)
                Self::update_window_title(
                    &window_clone,
                    &notebook,
                    settings.ui.window_title_shows_connection,
                );

                // Refresh the group-path subtitle live so toggling "Show
                // hierarchy path in header" updates without a tab switch.
                // Deferred to idle: it reads the setting from the state, and
                // the new settings only land there via `update_settings` below,
                // which also holds the state mutably while it runs.
                {
                    let header_title = header_title_for_apply.clone();
                    let notebook = notebook.clone();
                    let state = state.clone();
                    glib::idle_add_local_once(move || {
                        Self::refresh_header_subtitle(&header_title, &notebook, &state);
                    });
                }

                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    let simple_sync_was = state_mut.simple_sync_enabled();
                    let simple_sync_now = settings.sync.simple_sync_enabled;
                    // Clone secret settings for deferred keyring saves (done
                    // async to avoid blocking the GTK main loop on D-Bus). The
                    // settings in force before this save are needed too, so the
                    // deferred pass can tell which keyring entries the new
                    // choices have made stale.
                    let secrets_before_save = state_mut.settings().secrets.clone();
                    let secrets_for_keyring = settings.secrets.clone();
                    if let Err(e) = state_mut.update_settings(settings) {
                        tracing::error!(%e, "Failed to save settings");
                    } else {
                        // Non-blocking feedback that settings were persisted.
                        crate::toast::show_toast_on_window(
                            &window_clone,
                            &crate::i18n::i18n("Settings saved"),
                            crate::toast::ToastType::Success,
                        );

                        // Re-probe the selected secret backend now that the
                        // choice has changed, so the banner reports the backend
                        // in force rather than the one selected at launch. The
                        // Secrets page shows a readiness line while choosing;
                        // this is what makes a choice made anyway visible
                        // afterwards, instead of surfacing at the next start.
                        //
                        // Deferred to idle because the action's handler takes its
                        // own `try_borrow_mut` on the state this scope is already
                        // holding mutably.
                        let window_for_recheck = window_clone.clone();
                        glib::idle_add_local_once(move || {
                            let _ = gtk4::prelude::WidgetExt::activate_action(
                                &window_for_recheck,
                                "win.recheck-secret-backend",
                                None,
                            );
                        });

                        // A "remember this on the machine" request that could not
                        // be honoured. Same data-loss shape as the keyring failure
                        // below — the secret exists in memory only and is gone
                        // after a restart — so the same blocking dialog, not a
                        // toast that has already been replaced by "Settings
                        // saved".
                        let persistence_failures = state_mut.take_persistence_failures();
                        if !persistence_failures.is_empty() {
                            tracing::warn!(
                                backends = ?persistence_failures,
                                "Could not write the local encrypted copy of a secret"
                            );
                            crate::alert::show_error(
                                &window_clone,
                                &crate::i18n::i18n("Passphrase Not Saved on This Computer"),
                                &crate::i18n::i18n(
                                    "The passphrase could not be encrypted for local storage, so it is kept for this session only and you will be asked for it again after a restart. Choose \"System keyring\" instead, or check that RustConn can write to its data directory.",
                                ),
                            );
                        }

                        // Deferred keyring credential saves — run on a background
                        // thread so D-Bus round-trips don't block the UI.
                        let window_for_keyring = window_clone.clone();
                        let secrets_for_portable = secrets_for_keyring.clone();
                        let state_for_portable = state.clone();
                        glib::spawn_future_local(async move {
                            // Check the portable passphrase against the store it
                            // is supposed to open, before anything relies on it.
                            // Nothing used to: a typo here surfaced much later as
                            // a password prompt on the next connection, because
                            // `SecretManager::retrieve` reported a wrong
                            // passphrase as "no stored password".
                            verify_portable_passphrase_after_save(
                                &window_for_keyring,
                                &state_for_portable,
                                secrets_for_portable,
                            )
                            .await;

                            let outcome = gtk4::gio::spawn_blocking(move || {
                                crate::dialogs::settings::save_pending_keyring_credentials(
                                    &secrets_before_save,
                                    &secrets_for_keyring,
                                )
                            })
                            .await
                            .unwrap_or_default();
                            if outcome.write_failures > 0 {
                                tracing::warn!(
                                    write_failures = outcome.write_failures,
                                    "Some credentials failed to save to keyring"
                                );
                                // Data-loss risk: with the keyring selected there
                                // is no encrypted blob on disk, so a credential
                                // that never reached the keyring exists in memory
                                // only and is gone after a restart. The "Settings
                                // saved" toast has already fired, so this needs a
                                // blocking AlertDialog the user must acknowledge
                                // (GNOME HIG error-feedback rule).
                                crate::alert::show_error(
                                    &window_for_keyring,
                                    &crate::i18n::i18n("Credentials Not Saved to Keyring"),
                                    &crate::i18n::i18n(
                                        "The system keyring did not accept the credentials, so they are kept for this session only and will be gone after a restart. Check that your keyring service is running and unlocked, then save the settings again.",
                                    ),
                                );
                            }
                            if outcome.revoke_failures > 0 {
                                tracing::warn!(
                                    revoke_failures = outcome.revoke_failures,
                                    "Some stale keyring entries could not be removed"
                                );
                            }
                        });
                        // Simple Sync just turned on: publish the current store,
                        // then pull any existing remote data on idle so the
                        // device converges immediately.
                        if simple_sync_now && !simple_sync_was {
                            if let Err(e) = state_mut.simple_sync_export() {
                                tracing::warn!(%e, "Initial Simple Sync export failed");
                            }
                            let state_idle = state.clone();
                            let sidebar_idle = sidebar.clone();
                            glib::idle_add_local_once(move || {
                                let outcome = state_idle.try_borrow_mut().ok().and_then(|mut s| {
                                    let device_id = s.settings().sync.device_id;
                                    s.sync_manager()
                                        .should_import_simple_sync(device_id)
                                        .then(|| s.simple_sync_import_and_apply())
                                });
                                if let Some(Ok(report)) = outcome
                                    && !report.is_empty()
                                {
                                    Self::reload_sidebar_preserving_state(
                                        &state_idle,
                                        &sidebar_idle,
                                    );
                                }
                            });
                        }
                        // Update open-keepass action enabled state based on backend
                        if let Some(action) = window_clone.lookup_action("open-keepass")
                            && let Some(simple_action) = action.downcast_ref::<gio::SimpleAction>()
                        {
                            let action_enabled = match backend {
                                rustconn_core::config::SecretBackendType::LibSecret
                                | rustconn_core::config::SecretBackendType::MacOsKeychain
                                | rustconn_core::config::SecretBackendType::Bitwarden
                                | rustconn_core::config::SecretBackendType::OnePassword
                                | rustconn_core::config::SecretBackendType::Passbolt
                                | rustconn_core::config::SecretBackendType::Pass
                                | rustconn_core::config::SecretBackendType::ProtonPass
                                | rustconn_core::config::SecretBackendType::EncryptedFile
                                | rustconn_core::config::SecretBackendType::PortableEncryptedFile => true,
                                rustconn_core::config::SecretBackendType::KeePassXc
                                | rustconn_core::config::SecretBackendType::KdbxFile => {
                                    keepass_enabled && kdbx_path_exists
                                }
                            };
                            simple_action.set_enabled(action_enabled);
                        }
                    }
                } else {
                    tracing::error!("Failed to borrow state for settings update");
                }
            }
        });
    }
}

/// Checks a freshly saved portable passphrase against the store it must open.
///
/// Silent when the portable backend is not the preferred one, when no passphrase
/// was supplied, or when the store does not exist yet — the first write creates
/// it under whatever was typed, which is the intended first-machine flow.
///
/// The derivation runs on a blocking thread: it is Argon2id with the file's own
/// parameters, and the settings save happens on the GTK main thread.
#[expect(
    clippy::future_not_send,
    reason = "borrows a GTK window; awaited only from glib::spawn_future_local on the main thread"
)]
pub(super) async fn verify_portable_passphrase_after_save(
    window: &adw::ApplicationWindow,
    state: &SharedAppState,
    secrets: rustconn_core::config::SecretSettings,
) {
    use rustconn_core::config::SecretBackendType;

    if secrets.preferred_backend != SecretBackendType::PortableEncryptedFile {
        return;
    }
    let Some(passphrase) = secrets.portable_passphrase.clone() else {
        return;
    };
    let path =
        rustconn_core::secret::resolve_portable_store_path(secrets.portable_file_path.as_deref());
    if !path.exists() {
        return;
    }

    let outcome = gtk4::gio::spawn_blocking(move || {
        rustconn_core::secret::verify_portable_passphrase(&path, &passphrase)
    })
    .await;

    let Ok(Err(e)) = outcome else {
        // `Ok(Ok(()))` is the good path; a join error means the check itself did
        // not run, which is not evidence the passphrase is wrong and is not worth
        // a dialog the user cannot act on.
        return;
    };
    if !matches!(e, rustconn_core::error::SecretError::IncorrectPassphrase) {
        tracing::warn!(error = %e, "Could not check the portable store passphrase");
        return;
    }

    tracing::warn!("The saved portable passphrase does not open the portable credential file");

    // Drop the passphrase that was just proven wrong. Keeping it would leave the
    // backend "unlocked" with a key that opens nothing, and `retrieve` no longer
    // walks past `IncorrectPassphrase` — so every credential lookup would fail
    // for the rest of the session, including the ones the machine-bound store
    // could still answer. Locked is a state the rest of the app handles: the
    // backend reports itself unavailable, the fallback is reachable again, and a
    // connection that needs the file raises the unlock dialog.
    if let Ok(mut state_mut) = state.try_borrow_mut() {
        state_mut.lock_portable_store();
    }

    crate::alert::show_error(
        window,
        &crate::i18n::i18n("Passphrase Does Not Open the Portable File"),
        &crate::i18n::i18n(
            "The passphrase you saved does not decrypt the portable credential file, so none of the passwords in it can be read. Open Settings ▸ Secrets and enter the passphrase the file was created with.",
        ),
    );
}
