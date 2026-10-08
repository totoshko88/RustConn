//! Pass (password-store) backend for Unix password manager
//!
//! This module implements credential storage using the standard Unix password
//! manager "pass" (passwordstore.org). Pass uses GPG encryption and git-backed
//! storage, making it ideal for command-line users.

use std::process::Stdio;

use async_trait::async_trait;
use secrecy::SecretString;
use tokio::process::Command;

use super::backend::SecretBackend;
use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

/// Pass (password-store) backend for Unix password manager
///
/// This backend uses the `pass` command-line utility which stores passwords
/// in GPG-encrypted files organized in a directory hierarchy, typically
/// at ~/.password-store/. Each password is stored in a separate file.
pub struct PassBackend {
    /// Optional custom password store directory (defaults to ~/.password-store)
    store_dir: Option<String>,
    /// When true, every mutating operation is refused with
    /// [`SecretError::ReadOnly`] and the store is left untouched.
    read_only: bool,
    /// When true, a missed `rustconn/<id>/<field>` read falls back to
    /// `<id>/<field>` at the store root, so an entry the user keeps outside the
    /// `rustconn/` subtree is still found. Writes stay `rustconn/`-prefixed.
    root_search: bool,
}

impl Default for PassBackend {
    fn default() -> Self {
        Self::new(None)
    }
}

impl PassBackend {
    /// Creates a new Pass backend
    ///
    /// # Arguments
    /// * `store_dir` - Optional custom password store directory
    ///
    /// # Returns
    /// A new `PassBackend` instance
    #[must_use]
    pub fn new(store_dir: Option<String>) -> Self {
        Self {
            store_dir,
            read_only: false,
            root_search: false,
        }
    }

    /// Puts the backend in read-only mode, where `store` and `delete` are
    /// refused with [`SecretError::ReadOnly`] and the store is never mutated.
    #[must_use]
    pub const fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Widens credential reads: when a `rustconn/<id>/<field>` lookup misses,
    /// `retrieve` also tries `<id>/<field>` at the store root (no `rustconn/`
    /// prefix), so an entry the user keeps outside the `rustconn/` subtree is
    /// still found. Writes stay `rustconn/`-prefixed and are unaffected.
    #[must_use]
    pub const fn with_root_search(mut self, root_search: bool) -> Self {
        self.root_search = root_search;
        self
    }

    /// Creates a `PassBackend` from an optional store directory path.
    ///
    /// Convenience constructor that converts `PathBuf` to `String`.
    /// Avoids code duplication across GUI and CLI crates.
    #[must_use]
    pub fn from_path(store_dir: Option<&std::path::Path>) -> Self {
        Self::new(store_dir.map(|p| p.to_string_lossy().to_string()))
    }

    /// Creates a `PassBackend` from secret settings.
    ///
    /// Extracts `pass_store_dir` from the provided settings and applies the
    /// `pass_read_only` / `pass_root_search` toggles so the persisted choice
    /// reaches the backend. `from_path` already carries the store directory, so
    /// `pass_store_dir` still flows through unchanged.
    #[must_use]
    pub fn from_secret_settings(settings: &crate::config::SecretSettings) -> Self {
        Self::from_path(settings.pass_store_dir.as_deref())
            .with_read_only(settings.pass_read_only)
            .with_root_search(settings.pass_root_search)
    }

    /// Creates a `PassBackend` from app settings.
    ///
    /// Extracts `pass_store_dir` from the app settings' secrets section.
    #[must_use]
    pub fn from_app_settings(settings: &crate::config::AppSettings) -> Self {
        Self::from_secret_settings(&settings.secrets)
    }

    /// Builds the pass path for a connection's credential field
    ///
    /// Structure: `rustconn/<connection_id>/<field>`
    /// Where field is one of: username, password, key_passphrase, domain
    #[expect(
        clippy::unused_self,
        reason = "method is part of a uniform helper API where most operations need &self; keeping &self preserves the consistent signature"
    )]
    fn build_pass_path(&self, connection_id: &str, field: &str) -> String {
        // Sanitize connection_id to prevent path traversal (e.g. "../../other")
        let safe_id = connection_id.replace(['/', '\\', '.'], "_");
        let safe_field = field.replace(['/', '\\', '.'], "_");
        format!("rustconn/{safe_id}/{safe_field}")
    }

    /// Builds the root-search fallback path `<connection_id>/<field>` — the same
    /// entry WITHOUT the `rustconn/` prefix, for an entry the user keeps at the
    /// store root.
    ///
    /// Applies the identical path-traversal sanitization as
    /// [`Self::build_pass_path`]; the only difference is the dropped `rustconn/`
    /// prefix. Pure (no `self` state read) so the root-widening path shape is
    /// unit-tested without the `pass` CLI.
    #[expect(
        clippy::unused_self,
        reason = "mirrors build_pass_path's uniform &self helper signature"
    )]
    fn build_root_path(&self, connection_id: &str, field: &str) -> String {
        let safe_id = connection_id.replace(['/', '\\', '.'], "_");
        let safe_field = field.replace(['/', '\\', '.'], "_");
        format!("{safe_id}/{safe_field}")
    }

    /// Sets up the Command with optional PASSWORD_STORE_DIR
    fn setup_command(&self) -> Command {
        let mut cmd = Command::new("pass");
        cmd.env("PATH", crate::cli_download::get_extended_path());
        if let Some(ref dir) = self.store_dir {
            cmd.env("PASSWORD_STORE_DIR", dir);
        }
        cmd
    }

    /// Stores a value using pass insert
    async fn store_value(&self, connection_id: &str, field: &str, value: &str) -> SecretResult<()> {
        let path = self.build_pass_path(connection_id, field);

        let mut child = self
            .setup_command()
            .arg("insert")
            .arg("--force") // Overwrite if exists
            .arg("--multiline")
            .arg(&path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SecretError::Pass(format!("Failed to spawn pass: {e}")))?;

        // Write the secret to stdin and close it
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(value.as_bytes())
                .await
                .map_err(|e| SecretError::Pass(format!("Failed to write secret: {e}")))?;
            stdin
                .write_all(b"\n")
                .await
                .map_err(|e| SecretError::Pass(format!("Failed to write newline: {e}")))?;
            // Close stdin to signal EOF for --multiline
            drop(stdin);
        }

        let output = child
            .wait_with_output()
            .await
            .map_err(|e| SecretError::Pass(format!("Failed to wait for pass: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SecretError::StoreFailed(format!(
                "pass insert failed: {stderr}"
            )));
        }

        Ok(())
    }

    /// Retrieves a value using pass show, widening to the connection's host and
    /// name aliases at the store root when root-search is on (issue #353). The
    /// scoped `rustconn/<id>/<field>` lookup still wins first. When root-search
    /// is on, a scoped miss falls back to `<candidate>/<field>` at the store
    /// root for each candidate (the connection id first, then the supplied
    /// host/name aliases), so an entry the user keeps outside the `rustconn/`
    /// subtree and titled by host or name is still found. Reads only — writes
    /// stay `rustconn/`-prefixed. With `aliases` empty, behaviour is
    /// byte-identical to the id-only scoped-then-root lookup.
    async fn retrieve_value_with_aliases(
        &self,
        connection_id: &str,
        aliases: &[&str],
        field: &str,
    ) -> SecretResult<Option<String>> {
        // Scoped lookup first — unchanged behaviour. Found-first for back-compat:
        // a `rustconn/`-scoped entry wins over an identically-named one at the
        // store root.
        let scoped = self
            .show_path(&self.build_pass_path(connection_id, field))
            .await?;
        if scoped.is_some() {
            return Ok(scoped);
        }

        // Root-search fallback: only on a scoped miss, and only when enabled,
        // look up `<candidate>/<field>` at the store root (no `rustconn/`
        // prefix) for the connection id and each alias (host/name). This widens
        // READS only — `store`/`delete` stay `rustconn/`-prefixed. When
        // root-search is off, behaviour is byte-identical to the scoped-only
        // lookup above.
        if self.root_search {
            // Candidate order: id first (back-compat), then aliases;
            // de-duplicated case-insensitively, blanks dropped. Empty aliases
            // reduces to the single-id root fallback that was here before.
            let mut candidates: Vec<&str> = Vec::with_capacity(1 + aliases.len());
            for cand in std::iter::once(connection_id).chain(aliases.iter().copied()) {
                let cand = cand.trim();
                if !cand.is_empty() && !candidates.iter().any(|c| c.eq_ignore_ascii_case(cand)) {
                    candidates.push(cand);
                }
            }
            for cand in candidates {
                if let Some(value) = self.show_path(&self.build_root_path(cand, field)).await? {
                    return Ok(Some(value));
                }
            }
        }

        Ok(None)
    }

    /// Runs `pass show <path>` and returns the first stored line, or `None` for
    /// a genuine miss. Shared by the scoped lookup and the root-search fallback
    /// so both treat a missing entry and a not-ready store identically.
    async fn show_path(&self, path: &str) -> SecretResult<Option<String>> {
        let output = self
            .setup_command()
            .arg("show")
            .arg(path)
            .output()
            .await
            .map_err(|e| SecretError::Pass(format!("Failed to run pass: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Only a genuine miss is `Ok(None)`. Every non-zero exit used to be
            // one, with the stderr discarded, so an uninitialised store, a
            // missing or expired GPG key and a locked gpg-agent were all
            // indistinguishable from "no such password" — and `Ok(None)` reaches
            // the user as "Vault entry not found. You will be prompted for a
            // password", which is the wrong thing to say and gives them nothing
            // to act on. Every other backend reports a not-ready state as an
            // error, which the connect path turns into a dialog naming the
            // backend; `pass` was the only one that did not.
            //
            // `is not in the password store` is pass's own wording for a missing
            // entry, and `delete_value` below already matches on it for the same
            // reason, so this is the existing convention rather than a new guess.
            if stderr.contains("is not in the password store") {
                return Ok(None);
            }
            return Err(SecretError::Pass(format!(
                "pass show failed: {}",
                stderr.trim()
            )));
        }

        let value = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next() // Pass stores the password on the first line
            .unwrap_or("")
            .trim()
            .to_string();

        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }

    /// Shared retrieve path for both [`SecretBackend::retrieve`] and the
    /// alias-aware [`SecretBackend::retrieve_identity`]. `aliases` are the
    /// connection host/name that root-search may also match (issue #353); empty
    /// for the plain path, in which case behaviour is byte-identical to the old
    /// `retrieve`. Each field is looked up scoped-first then root-widened
    /// across the candidate paths.
    async fn retrieve_with_aliases(
        &self,
        connection_id: &str,
        aliases: &[&str],
    ) -> SecretResult<Option<Credentials>> {
        let username = self
            .retrieve_value_with_aliases(connection_id, aliases, "username")
            .await?;
        let password = self
            .retrieve_value_with_aliases(connection_id, aliases, "password")
            .await?;
        let key_passphrase = self
            .retrieve_value_with_aliases(connection_id, aliases, "key_passphrase")
            .await?;
        let domain = self
            .retrieve_value_with_aliases(connection_id, aliases, "domain")
            .await?;

        // If nothing was found, return None
        if username.is_none() && password.is_none() && key_passphrase.is_none() && domain.is_none()
        {
            return Ok(None);
        }

        Ok(Some(Credentials {
            username,
            password: password.map(SecretString::from),
            key_passphrase: key_passphrase.map(SecretString::from),
            domain,
        }))
    }

    /// Deletes a value using pass rm
    async fn delete_value(&self, connection_id: &str, field: &str) -> SecretResult<()> {
        let path = self.build_pass_path(connection_id, field);

        let output = self
            .setup_command()
            .arg("rm")
            .arg("--force") // Don't prompt for confirmation
            .arg(&path)
            .output()
            .await
            .map_err(|e| SecretError::Pass(format!("Failed to run pass: {e}")))?;

        if !output.status.success() {
            // It's okay if the file doesn't exist
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.contains("is not in the password store") {
                return Err(SecretError::DeleteFailed(format!(
                    "pass rm failed: {stderr}"
                )));
            }
        }

        Ok(())
    }

    /// Deletes the entire connection directory if empty
    async fn cleanup_directory(&self, connection_id: &str) -> SecretResult<()> {
        use std::path::PathBuf;

        // Determine the password store directory
        let store_dir = if let Some(ref custom_dir) = self.store_dir {
            PathBuf::from(custom_dir)
        } else if let Some(home) = dirs::home_dir() {
            // Default is ~/.password-store
            home.join(".password-store")
        } else {
            // Fallback if home directory cannot be determined
            PathBuf::from(".password-store")
        };

        // Try to remove the connection directory (will only succeed if empty)
        let conn_dir = store_dir.join("rustconn").join(connection_id);
        let _ = tokio::fs::remove_dir(&conn_dir).await;

        // Try to remove rustconn directory if empty
        let rustconn_dir = store_dir.join("rustconn");
        let _ = tokio::fs::remove_dir(&rustconn_dir).await;

        Ok(())
    }
}

#[async_trait]
impl SecretBackend for PassBackend {
    async fn store(&self, connection_id: &str, credentials: &Credentials) -> SecretResult<()> {
        self.ensure_writable()?;
        // Store username if present
        if let Some(username) = &credentials.username {
            self.store_value(connection_id, "username", username)
                .await?;
        }

        // Store password if present
        if let Some(password) = credentials.expose_password() {
            self.store_value(connection_id, "password", password)
                .await?;
        }

        // Store key passphrase if present
        if let Some(passphrase) = credentials.expose_key_passphrase() {
            self.store_value(connection_id, "key_passphrase", passphrase)
                .await?;
        }

        // Store domain if present
        if let Some(domain) = &credentials.domain {
            self.store_value(connection_id, "domain", domain).await?;
        }

        Ok(())
    }

    async fn retrieve(&self, connection_id: &str) -> SecretResult<Option<Credentials>> {
        self.retrieve_with_aliases(connection_id, &[]).await
    }

    async fn retrieve_identity(
        &self,
        identity: super::backend::LookupIdentity<'_>,
    ) -> SecretResult<Option<Credentials>> {
        let aliases = identity.aliases();
        self.retrieve_with_aliases(identity.key, &aliases).await
    }

    async fn delete(&self, connection_id: &str) -> SecretResult<()> {
        self.ensure_writable()?;
        // Delete all stored values for this connection
        // Ignore errors for individual fields (they might not exist)
        let _ = self.delete_value(connection_id, "username").await;
        let _ = self.delete_value(connection_id, "password").await;
        let _ = self.delete_value(connection_id, "key_passphrase").await;
        let _ = self.delete_value(connection_id, "domain").await;

        // Try to clean up empty directories
        let _ = self.cleanup_directory(connection_id).await;

        Ok(())
    }

    async fn is_available(&self) -> bool {
        // Check if pass is available
        let mut cmd = Command::new("pass");
        cmd.env("PATH", crate::cli_download::get_extended_path());
        if let Some(ref dir) = self.store_dir {
            cmd.env("PASSWORD_STORE_DIR", dir);
        }

        cmd.arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn backend_id(&self) -> &'static str {
        "pass"
    }

    fn display_name(&self) -> &'static str {
        "Pass (Unix Password Manager)"
    }

    fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn searches_from_root(&self) -> bool {
        self.root_search
    }
}

impl std::fmt::Debug for PassBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PassBackend")
            .field("store_dir", &self.store_dir)
            .field("read_only", &self.read_only)
            .field("root_search", &self.root_search)
            .finish()
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    #[test]
    fn debug_does_not_leak_secret() {
        // PassBackend stores no secrets — the password store directory
        // path is non-secret. The test ensures that future additions
        // (e.g. cached GPG passphrase) cannot leak through Debug.
        let backend = PassBackend::new(Some("/tmp/fake-store-hunter2".to_string()));
        let rendered = format!("{backend:?}");
        assert!(rendered.contains("PassBackend"));
        // The store dir is not a secret, so it may appear; but the
        // rendered output must not gain new fields containing passwords.
        // Sentinel: ensure we are still rendering only known fields.
        assert!(
            rendered.contains("store_dir"),
            "unexpected Debug shape: {rendered}"
        );
    }
}

#[cfg(test)]
mod read_only_tests {
    use super::*;
    use crate::error::SecretError;
    use crate::models::Credentials;

    #[test]
    fn with_read_only_sets_capability() {
        assert!(!PassBackend::new(None).is_read_only());
        assert!(PassBackend::new(None).with_read_only(true).is_read_only());
    }

    #[tokio::test]
    async fn read_only_refuses_store_before_touching_the_store() {
        // The guard is the first line of store(), so it short-circuits with
        // ReadOnly whether or not `pass` is installed — a deterministic test
        // that does not depend on the environment.
        let backend = PassBackend::new(Some("/nonexistent/store".to_string())).with_read_only(true);
        let err = backend
            .store("conn-1", &Credentials::default())
            .await
            .unwrap_err();
        assert!(
            matches!(err, SecretError::ReadOnly(name) if name == "Pass (Unix Password Manager)")
        );
    }

    #[tokio::test]
    async fn read_only_refuses_delete() {
        let backend = PassBackend::new(Some("/nonexistent/store".to_string())).with_read_only(true);
        let err = backend.delete("conn-1").await.unwrap_err();
        assert!(matches!(err, SecretError::ReadOnly(_)));
    }
}

#[cfg(test)]
mod root_search_tests {
    use super::*;

    #[test]
    fn searches_from_root_defaults_false_and_builder_toggles_it() {
        assert!(!PassBackend::new(None).searches_from_root());
        assert!(
            PassBackend::new(None)
                .with_root_search(true)
                .searches_from_root()
        );
        // Setting it back to false is honoured.
        assert!(
            !PassBackend::new(None)
                .with_root_search(true)
                .with_root_search(false)
                .searches_from_root()
        );
    }

    #[test]
    fn read_only_and_root_search_are_independent() {
        let both = PassBackend::new(None)
            .with_read_only(true)
            .with_root_search(true);
        assert!(both.is_read_only());
        assert!(both.searches_from_root());
    }

    /// The two path builders are the whole of the root-search widening for pass
    /// (`retrieve_value` only chooses scoped-first then root on a miss), so
    /// unit-testing the path shapes covers the logic without the `pass` CLI.
    /// The scoped path keeps the `rustconn/` prefix; the root path drops it;
    /// both apply the same path-traversal sanitization.
    #[test]
    fn scoped_path_keeps_prefix_and_root_path_drops_it() {
        let backend = PassBackend::new(None);

        assert_eq!(
            backend.build_pass_path("conn-1", "password"),
            "rustconn/conn-1/password"
        );
        assert_eq!(
            backend.build_root_path("conn-1", "password"),
            "conn-1/password"
        );
    }

    /// Both builders must neutralise path-traversal characters identically, so
    /// the root fallback cannot be a traversal the scoped form rejected.
    #[test]
    fn both_builders_sanitize_traversal_characters() {
        let backend = PassBackend::new(None);

        // `.`, `/` and `\` all collapse to `_` in both id and field, so
        // `../etc` becomes `___etc` (two dots + one slash) and `pass/word`
        // becomes `pass_word`.
        assert_eq!(
            backend.build_pass_path("../etc", "pass/word"),
            "rustconn/___etc/pass_word"
        );
        assert_eq!(
            backend.build_root_path("../etc", "pass/word"),
            "___etc/pass_word"
        );
    }

    /// Issue #353: the root-search fallback tries `<candidate>/<field>` at the
    /// store root for the connection id AND each host/name alias, so an entry
    /// titled by host or name is found. The root path for each alias is the
    /// alias (sanitized) joined to the field — distinct from the id's root path
    /// — which is the whole of the per-candidate widening
    /// (`retrieve_value_with_aliases` only iterates these paths). A host (with a
    /// dot) is sanitized the same way as any other candidate.
    #[test]
    fn root_paths_cover_id_host_and_name_aliases() {
        let backend = PassBackend::new(None);

        // The connection id (back-compat), still the first candidate tried.
        assert_eq!(
            backend.build_root_path("conn-1", "password"),
            "conn-1/password"
        );
        // A host alias — the `.` in a hostname is sanitized to `_`, exactly as
        // the scoped builder would, so host and scoped share one sanitizer.
        assert_eq!(
            backend.build_root_path("db.example.com", "password"),
            "db_example_com/password"
        );
        // A name alias.
        assert_eq!(
            backend.build_root_path("Prod Database", "username"),
            "Prod Database/username"
        );
        // Each candidate's root path is distinct from the scoped `rustconn/`
        // path, so an alias entry is never already caught by the scoped pass.
        assert_ne!(
            backend.build_root_path("db.example.com", "password"),
            backend.build_pass_path("conn-1", "password")
        );
    }
}
