//! Secrets read from the system keyring at startup, after the window is shown.
//!
//! `AppState::new` used to read them on the GTK thread before the window
//! existed, up to 5 s each when the Secret Service was locked or slow. Now it
//! only records which reads are needed ([`KeyringJobs`]); `app.rs` runs them on
//! a thread once the window is presented and hands the result to
//! `AppState::finish_startup_keyring`, which fills the settings, rebuilds the
//! secret manager and opens the [`StartupGate`].
//!
//! Until the gate opens, the keyring-backed secrets are `None` in the settings,
//! exactly as they are after a failed read. The connect path
//! (`resolve_credentials`) waits on the gate in its worker thread, so a connect
//! made right after start still resolves with the secrets. Other readers see the
//! `None` and use their existing "password not loaded" handling.

use std::fmt::Display;
use std::time::Duration;

use rustconn_core::config::{SecretBackendType, SecretSettings};
use rustconn_core::secret::SecretManager;
use rustconn_core::startup_gate::StartupGate;
use secrecy::SecretString;

/// Bound for one keyring read, as before.
const KEYRING_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a credential lookup waits for the reads. They run in parallel and
/// each is bounded by [`KEYRING_READ_TIMEOUT`], so this only guards a lost thread.
pub(super) const KEYRING_WAIT: Duration = Duration::from_secs(10);

/// The state published when the reads are over.
#[derive(Clone)]
pub struct StartupSnapshot {
    /// The secret settings with the loaded values filled in.
    pub secrets: SecretSettings,
    /// The secret manager rebuilt from [`Self::secrets`].
    pub manager: SecretManager,
}

/// Opens when the startup keyring reads are over; holds the state they produced.
pub type KeyringGate = StartupGate<Option<StartupSnapshot>>;

/// Which keyring reads startup still has to do.
#[allow(clippy::struct_excessive_bools)] // one independent flag per read
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyringJobs {
    kdbx: bool,
    onepassword: bool,
    passbolt: bool,
    portable: bool,
}

impl KeyringJobs {
    /// The reads needed for `secrets`, called after the encrypted local copies
    /// were decrypted: a secret already known is not read again.
    #[must_use]
    pub fn for_settings(secrets: &SecretSettings) -> Self {
        let backend = secrets.preferred_backend;
        Self {
            kdbx: secrets.kdbx_enabled
                && secrets.kdbx_password.is_none()
                && secrets.kdbx_save_to_keyring
                && secrets.kdbx_path.as_ref().is_some_and(|p| p.exists()),
            onepassword: backend == SecretBackendType::OnePassword
                && secrets.onepassword_service_account_token.is_none()
                && secrets.onepassword_save_to_keyring,
            passbolt: backend == SecretBackendType::Passbolt
                && secrets.passbolt_passphrase.is_none()
                && secrets.passbolt_save_to_keyring,
            portable: backend == SecretBackendType::PortableEncryptedFile
                && secrets.portable_passphrase.is_none()
                && secrets.portable_save_to_keyring,
        }
    }

    /// Whether any read is needed.
    #[must_use]
    pub const fn any(self) -> bool {
        self.kdbx || self.onepassword || self.passbolt || self.portable
    }
}

/// What the keyring reads returned.
#[derive(Default)]
pub struct KeyringSecrets {
    kdbx: Option<SecretString>,
    onepassword: Option<SecretString>,
    passbolt: Option<SecretString>,
    portable: Option<SecretString>,
    /// The KeePass read was asked for and did not produce a password.
    pub kdbx_failed: bool,
}

impl KeyringSecrets {
    /// Fills the secrets that are still `None`; one set meanwhile (by Settings
    /// or an unlock dialog) is kept. Returns whether anything was filled.
    ///
    /// Only the backends `secrets` configures now are filled, by the same
    /// rule as [`KeyringJobs::for_settings`]: a user who switched backend while
    /// the reads ran does not get the old backend's secret back in memory.
    pub fn apply(self, secrets: &mut SecretSettings) -> bool {
        let backend = secrets.preferred_backend;
        let kdbx_wanted = secrets.kdbx_enabled;
        let mut changed = false;
        for (wanted, slot, value) in [
            (kdbx_wanted, &mut secrets.kdbx_password, self.kdbx),
            (
                backend == SecretBackendType::OnePassword,
                &mut secrets.onepassword_service_account_token,
                self.onepassword,
            ),
            (
                backend == SecretBackendType::Passbolt,
                &mut secrets.passbolt_passphrase,
                self.passbolt,
            ),
            (
                backend == SecretBackendType::PortableEncryptedFile,
                &mut secrets.portable_passphrase,
                self.portable,
            ),
        ] {
            if wanted && slot.is_none() && value.is_some() {
                *slot = value;
                changed = true;
            }
        }
        changed
    }
}

/// Reads one secret with the startup bound; `None` when it is missing or failed.
async fn read<E: Display>(
    what: &str,
    bound: Duration,
    lookup: impl Future<Output = Result<Option<SecretString>, E>>,
) -> Option<SecretString> {
    match tokio::time::timeout(bound, lookup).await {
        Ok(Ok(Some(secret))) => {
            tracing::info!("{what} restored from system keyring");
            Some(secret)
        }
        Ok(Ok(None)) => {
            tracing::debug!("No {what} found in system keyring");
            None
        }
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "Failed to load {what} from system keyring");
            None
        }
        Err(_elapsed) => {
            tracing::warn!("Keyring query for {what} timed out");
            None
        }
    }
}

async fn load(jobs: KeyringJobs) -> KeyringSecrets {
    async fn when<F: Future<Output = Option<SecretString>>>(
        wanted: bool,
        fut: F,
    ) -> Option<SecretString> {
        if wanted { fut.await } else { None }
    }
    let (kdbx, onepassword, passbolt, portable) = tokio::join!(
        when(
            jobs.kdbx,
            read(
                "KeePass password",
                KEYRING_READ_TIMEOUT,
                rustconn_core::secret::get_kdbx_password_from_keyring()
            )
        ),
        when(
            jobs.onepassword,
            read(
                "1Password token",
                KEYRING_READ_TIMEOUT,
                rustconn_core::secret::get_token_from_keyring()
            )
        ),
        when(
            jobs.passbolt,
            read(
                "Passbolt passphrase",
                KEYRING_READ_TIMEOUT,
                rustconn_core::secret::get_passphrase_from_keyring()
            )
        ),
        when(
            jobs.portable,
            read(
                "portable file passphrase",
                KEYRING_READ_TIMEOUT,
                rustconn_core::secret::get_portable_passphrase_from_keyring()
            )
        ),
    );
    KeyringSecrets {
        kdbx_failed: jobs.kdbx && kdbx.is_none(),
        kdbx,
        onepassword,
        passbolt,
        portable,
    }
}

/// Runs the reads on their own thread and runtime, so neither the GTK thread
/// nor its runtime is involved. The receiver errors if the thread dies.
pub fn load_in_background(jobs: KeyringJobs) -> tokio::sync::oneshot::Receiver<KeyringSecrets> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("keyring-startup".into())
        .spawn(move || {
            let started = std::time::Instant::now();
            let loaded = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt.block_on(load(jobs)),
                Err(e) => {
                    tracing::warn!(%e, "Failed to create runtime for startup keyring reads");
                    KeyringSecrets {
                        kdbx_failed: jobs.kdbx,
                        ..KeyringSecrets::default()
                    }
                }
            };
            tracing::info!(
                elapsed_ms = started.elapsed().as_millis(),
                "startup keyring reads finished"
            );
            let _ = tx.send(loaded);
        });
    if let Err(e) = spawned {
        tracing::warn!(%e, "Failed to start the startup keyring thread");
    }
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyring_settings() -> SecretSettings {
        SecretSettings {
            preferred_backend: SecretBackendType::OnePassword,
            onepassword_save_to_keyring: true,
            ..SecretSettings::default()
        }
    }

    #[test]
    fn jobs_follow_the_preferred_backend() {
        let mut s = keyring_settings();
        assert!(KeyringJobs::for_settings(&s).onepassword);
        s.preferred_backend = SecretBackendType::Passbolt;
        let jobs = KeyringJobs::for_settings(&s);
        assert!(!jobs.onepassword && !jobs.passbolt && !jobs.any());
    }

    #[test]
    fn a_known_secret_is_not_read_again() {
        let mut s = keyring_settings();
        s.onepassword_service_account_token = Some(SecretString::from("t"));
        assert!(!KeyringJobs::for_settings(&s).any());
    }

    #[test]
    fn kdbx_needs_an_existing_file() {
        let mut s = SecretSettings {
            kdbx_enabled: true,
            kdbx_save_to_keyring: true,
            kdbx_path: Some("/nonexistent/db.kdbx".into()),
            ..SecretSettings::default()
        };
        assert!(!KeyringJobs::for_settings(&s).kdbx);
        s.kdbx_path = Some(std::env::current_exe().unwrap());
        assert!(KeyringJobs::for_settings(&s).kdbx);
    }

    #[test]
    fn apply_fills_only_empty_slots() {
        use secrecy::ExposeSecret;
        let mut s = SecretSettings {
            kdbx_enabled: true,
            preferred_backend: SecretBackendType::Passbolt,
            kdbx_password: Some(SecretString::from("set-meanwhile")),
            ..SecretSettings::default()
        };
        let loaded = KeyringSecrets {
            kdbx: Some(SecretString::from("from-keyring")),
            passbolt: Some(SecretString::from("pp")),
            ..KeyringSecrets::default()
        };
        assert!(loaded.apply(&mut s));
        assert_eq!(s.kdbx_password.unwrap().expose_secret(), "set-meanwhile");
        assert_eq!(s.passbolt_passphrase.unwrap().expose_secret(), "pp");
    }

    /// The user switched from 1Password to Passbolt while the reads ran: the
    /// 1Password token must not come back, and KeePass, now off, stays empty.
    #[test]
    fn apply_skips_backends_switched_away_from() {
        use secrecy::ExposeSecret;
        let mut s = SecretSettings {
            preferred_backend: SecretBackendType::Passbolt,
            kdbx_enabled: false,
            ..SecretSettings::default()
        };
        let loaded = KeyringSecrets {
            kdbx: Some(SecretString::from("kp")),
            onepassword: Some(SecretString::from("token")),
            passbolt: Some(SecretString::from("pp")),
            portable: Some(SecretString::from("port")),
            ..KeyringSecrets::default()
        };
        assert!(loaded.apply(&mut s));
        assert!(s.kdbx_password.is_none());
        assert!(s.onepassword_service_account_token.is_none());
        assert!(s.portable_passphrase.is_none());
        assert_eq!(s.passbolt_passphrase.unwrap().expose_secret(), "pp");
    }

    #[test]
    fn apply_reports_no_change_when_only_other_backends_loaded() {
        let mut s = SecretSettings {
            preferred_backend: SecretBackendType::Passbolt,
            ..SecretSettings::default()
        };
        let loaded = KeyringSecrets {
            onepassword: Some(SecretString::from("token")),
            ..KeyringSecrets::default()
        };
        assert!(!loaded.apply(&mut s));
        assert!(s.onepassword_service_account_token.is_none());
    }

    #[test]
    fn apply_with_nothing_loaded_changes_nothing() {
        let mut s = SecretSettings::default();
        assert!(!KeyringSecrets::default().apply(&mut s));
    }

    /// A hung keyring must cost the bound, not more, and report failure.
    #[test]
    fn a_hung_read_times_out_as_missing() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let got = rt.block_on(read::<String>(
            "x",
            Duration::from_millis(20),
            std::future::pending(),
        ));
        assert!(got.is_none());
    }
}
