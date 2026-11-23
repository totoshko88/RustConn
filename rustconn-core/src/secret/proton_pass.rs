//! Proton Pass CLI backend
//!
//! Stores credentials through the official Proton Pass CLI (`pass-cli`). Each
//! connection is one login item in a dedicated `RustConn` vault, titled by the
//! connection's lookup key (`rustconn/{name}`). The vault is created on the
//! first write.
//!
//! # Authentication
//!
//! `pass-cli` keeps its own session. The user signs in once with
//! `pass-cli login` (web login, or a personal access token) and every later
//! call reuses that session; `RustConn` never sees a Proton credential. A call
//! made without a session fails with "requires an authenticated client",
//! which this backend reports as [`SecretError::BackendUnavailable`].
//!
//! # Secrets and argv
//!
//! `pass-cli item update` only takes new values as `--field password=…` on the
//! command line, which would put the password in `/proc/PID/cmdline`. So a
//! write never updates an item in place: it creates a fresh item from a JSON
//! template piped over stdin (`item create login --from-template -`) and then
//! deletes the item it replaces. The new item is written before the old one is
//! removed, so a failure part-way leaves a duplicate rather than no credential.

use std::process::Stdio;

use async_trait::async_trait;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use zeroize::Zeroizing;

use super::backend::SecretBackend;
use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

/// The Proton Pass CLI program name.
const PASS_CLI: &str = "pass-cli";

/// Vault the backend writes to and reads from first.
const DEFAULT_VAULT: &str = "RustConn";

/// Ceiling on a single `pass-cli` invocation.
///
/// Every item command reaches the Proton API, so a slow or unreachable network
/// would otherwise stall the caller forever. Thirty seconds matches the
/// 1Password and Bitwarden backends.
const PP_INVOCATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Ceiling on one whole backend operation. A `store` runs up to seven
/// invocations in a row, so the per-invocation limit alone would allow minutes.
const PP_OPERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

/// Bounds a whole backend operation by [`PP_OPERATION_TIMEOUT`].
async fn bounded<T>(op: impl std::future::Future<Output = SecretResult<T>>) -> SecretResult<T> {
    tokio::time::timeout(PP_OPERATION_TIMEOUT, op)
        .await
        .map_err(|_| {
            SecretError::ConnectionFailed(format!(
                "Proton Pass operation timed out after {}s",
                PP_OPERATION_TIMEOUT.as_secs()
            ))
        })?
}

/// `pass-cli`'s wording when there is no usable session (not logged in, or the
/// session expired and was cleaned up).
const NOT_AUTHENTICATED: &str = "requires an authenticated client";

/// `pass-cli item view --field` wording for a field the item does not have.
/// Empty fields are not listed, so this is also how an empty username reads.
const FIELD_MISSING: &str = "Field does not exist";

/// Custom field the Windows domain is stored in.
const DOMAIN_FIELD: &str = "domain";

/// `pass-cli`'s switch between the kernel keyring and the Secret Service for
/// the key that encrypts its local session.
const KEYRING_ENV: &str = "PROTON_PASS_LINUX_KEYRING";

/// The environment variable `pass-cli` needs to keep a session in this process's
/// sandbox, if any.
///
/// `pass-cli` keeps its session key in the kernel keyring by default, and the
/// Flatpak seccomp filter refuses every `keyctl` call, so there it cannot even
/// report "not logged in". The manifest grants the Secret Service instead. A
/// value the user exported is left alone.
#[must_use]
pub fn pass_cli_sandbox_env() -> Option<(&'static str, &'static str)> {
    (crate::flatpak::is_flatpak() && std::env::var_os(KEYRING_ENV).is_none())
        .then_some((KEYRING_ENV, "dbus"))
}

/// Proton Pass CLI backend.
pub struct ProtonPassBackend {
    /// Vault for `RustConn` entries.
    vault_name: String,
    /// When true, every mutating operation is refused with
    /// [`SecretError::ReadOnly`] and the vault is left untouched.
    read_only: bool,
    /// When true, a miss in the `RustConn` vault widens the read to every vault
    /// the account can see, also matching items titled by host or name.
    root_search: bool,
    /// The `pass-cli` program to run; a stub in tests.
    program: String,
}

/// `pass-cli vault list --output json`.
#[derive(Debug, Deserialize)]
struct VaultList {
    #[serde(default)]
    vaults: Vec<VaultEntry>,
}

#[derive(Debug, Deserialize)]
struct VaultEntry {
    name: String,
    share_id: String,
}

/// `pass-cli item list --output json`. Holds no secret material: the CLI's
/// summary carries ids, title and type only.
#[derive(Debug, Deserialize)]
struct ItemList {
    #[serde(default)]
    items: Vec<ItemSummary>,
}

#[derive(Debug, Clone, Deserialize)]
struct ItemSummary {
    id: String,
    share_id: String,
    title: String,
}

/// The `item create login --from-template` JSON body. It holds the password,
/// so it is serialised straight into a `Zeroizing` buffer and never logged.
#[derive(Serialize)]
struct LoginTemplate<'a> {
    title: &'a str,
    username: Option<&'a str>,
    password: Option<&'a str>,
    urls: [&'a str; 0],
    extra_fields: Vec<ExtraField<'a>>,
}

#[derive(Serialize)]
struct ExtraField<'a> {
    kind: &'static str,
    title: &'static str,
    value: &'a str,
}

/// True for a value that is safe to pass as a positional `pass-cli` argument:
/// non-empty, not starting with `-` (it would be parsed as a flag) and with no
/// NUL or line break.
fn is_plain_arg(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.chars().any(|c| c == '\0' || c == '\n' || c == '\r')
}

impl ProtonPassBackend {
    /// Creates a backend for the default `RustConn` vault.
    #[must_use]
    pub fn new() -> Self {
        Self {
            vault_name: DEFAULT_VAULT.to_string(),
            read_only: false,
            root_search: false,
            program: PASS_CLI.to_string(),
        }
    }

    /// Creates a backend configured from secret settings, applying the
    /// `proton_pass_read_only` / `proton_pass_root_search` toggles. Every caller
    /// that builds this backend for a user operation goes through here.
    #[must_use]
    pub fn from_secret_settings(settings: &crate::config::SecretSettings) -> Self {
        Self::new()
            .with_read_only(settings.proton_pass_read_only)
            .with_root_search(settings.proton_pass_root_search)
    }

    /// Sets the vault name for `RustConn` entries.
    #[must_use]
    pub fn with_vault_name(mut self, name: impl Into<String>) -> Self {
        self.vault_name = name.into();
        self
    }

    /// Puts the backend in read-only mode, where `store` and `delete` are
    /// refused with [`SecretError::ReadOnly`] and the vault is never mutated.
    #[must_use]
    pub const fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Widens credential reads to every vault when the `RustConn` vault has no
    /// match. Writes still target the `RustConn` vault.
    #[must_use]
    pub const fn with_root_search(mut self, root_search: bool) -> Self {
        self.root_search = root_search;
        self
    }

    /// Runs `program` instead of `pass-cli`, so a test can drive the backend
    /// through a stub.
    #[cfg(test)]
    fn with_program(mut self, program: String) -> Self {
        self.program = program;
        self
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.env("PATH", crate::cli_download::get_extended_path());
        if let Some((key, value)) = pass_cli_sandbox_env() {
            cmd.env(key, value);
        }
        // The timeout drops the future; without this a stalled `pass-cli`
        // would outlive it.
        cmd.kill_on_drop(true);
        cmd.args(args);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd
    }

    /// Turns a failed invocation into an error. A missing session becomes
    /// [`SecretError::BackendUnavailable`] with the command that fixes it.
    fn failure(args: &[&str], stderr: &str) -> SecretError {
        if stderr.contains(NOT_AUTHENTICATED) {
            return SecretError::BackendUnavailable(
                "Not logged in to Proton Pass. Run 'pass-cli login' in a terminal".to_string(),
            );
        }
        // Only the subcommand is named: argv here never carries a secret, but
        // item titles are connection names and there is no need to repeat them.
        let sub = args.iter().take(2).copied().collect::<Vec<_>>().join(" ");
        SecretError::ConnectionFailed(format!("pass-cli {sub} failed: {}", stderr.trim()))
    }

    /// Runs `pass-cli` with `args` and an optional stdin payload. Returns the
    /// raw output so callers can tell a miss from a failure.
    async fn run_raw(
        &self,
        args: &[&str],
        stdin: Option<&[u8]>,
    ) -> SecretResult<std::process::Output> {
        let mut cmd = self.command(args);
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        let mut child = cmd
            .spawn()
            .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run pass-cli: {e}")))?;

        let stdin_pipe = child.stdin.take();
        let exchange = async {
            if let (Some(payload), Some(mut pipe)) = (stdin, stdin_pipe) {
                // A write error (broken pipe) means pass-cli exited early;
                // its stderr says why, so it is read below, not reported here.
                let _ = pipe.write_all(payload).await;
                // EOF ends the template.
                drop(pipe);
            }
            child.wait_with_output().await
        };
        tokio::time::timeout(PP_INVOCATION_TIMEOUT, exchange)
            .await
            .map_err(|_| {
                SecretError::ConnectionFailed(format!(
                    "pass-cli timed out after {}s",
                    PP_INVOCATION_TIMEOUT.as_secs()
                ))
            })?
            .map_err(|e| SecretError::ConnectionFailed(format!("Failed to wait for pass-cli: {e}")))
    }
    /// Runs `pass-cli` and returns stdout, or the mapped error on a non-zero exit.
    async fn run(&self, args: &[&str]) -> SecretResult<String> {
        let output = self.run_raw(args, None).await?;
        if !output.status.success() {
            return Err(Self::failure(
                args,
                &String::from_utf8_lossy(&output.stderr),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    async fn list_vaults(&self) -> SecretResult<Vec<VaultEntry>> {
        let out = self.run(&["vault", "list", "--output", "json"]).await?;
        let list: VaultList = serde_json::from_str(&out).map_err(|e| {
            SecretError::ConnectionFailed(format!("Failed to parse pass-cli vault list: {e}"))
        })?;
        // An id the CLI printed is passed back as an argument; one that looks
        // like a flag is dropped rather than trusted.
        Ok(list
            .vaults
            .into_iter()
            .filter(|v| is_plain_arg(&v.share_id))
            .collect())
    }

    /// Share id of the `RustConn` vault, or `None` when it does not exist yet.
    async fn find_vault(&self) -> SecretResult<Option<String>> {
        Ok(self
            .list_vaults()
            .await?
            .into_iter()
            .find(|v| v.name == self.vault_name)
            .map(|v| v.share_id))
    }

    /// Share id of the `RustConn` vault, creating it when missing.
    async fn get_or_create_vault(&self) -> SecretResult<String> {
        if let Some(share_id) = self.find_vault().await? {
            return Ok(share_id);
        }
        if !is_plain_arg(&self.vault_name) {
            return Err(SecretError::StoreFailed(
                "Invalid Proton Pass vault name".to_string(),
            ));
        }
        // `vault create` prints the new share id on its own line.
        let out = self
            .run(&["vault", "create", "--name", &self.vault_name])
            .await?;
        let share_id = out.trim();
        if !is_plain_arg(share_id) {
            return Err(SecretError::StoreFailed(
                "pass-cli vault create returned no valid share id".to_string(),
            ));
        }
        Ok(share_id.to_string())
    }

    /// Active login items in the vault `share_id`.
    async fn list_items(&self, share_id: &str) -> SecretResult<Vec<ItemSummary>> {
        let out = self
            .run(&[
                "item",
                "list",
                "--share-id",
                share_id,
                "--filter-type",
                "login",
                "--filter-state",
                "active",
                "--output",
                "json",
            ])
            .await?;
        // An item list carries titles and ids only, so the serde message (which
        // can quote input) cannot leak a credential.
        let list: ItemList = serde_json::from_str(&out).map_err(|e| {
            SecretError::ConnectionFailed(format!("Failed to parse pass-cli item list: {e}"))
        })?;
        Ok(list
            .items
            .into_iter()
            .filter(|i| is_plain_arg(&i.id) && is_plain_arg(&i.share_id))
            .collect())
    }

    /// `RustConn`'s own items for `key`: exact title in the `RustConn` vault.
    /// Never a root-search match — `store` and `delete` use this, and root
    /// search widens reads only.
    async fn own_items(&self, share_id: &str, key: &str) -> SecretResult<Vec<ItemSummary>> {
        Ok(self
            .list_items(share_id)
            .await?
            .into_iter()
            .filter(|item| item.title == key)
            .collect())
    }

    /// The bare titles a root-search read may match, in priority order: the
    /// key, then the connection's host/name aliases (issue #353). Blanks are
    /// dropped and duplicates removed case-insensitively.
    fn candidates<'c>(key: &'c str, aliases: &[&'c str]) -> Vec<&'c str> {
        let mut out: Vec<&str> = Vec::with_capacity(1 + aliases.len());
        for cand in std::iter::once(key).chain(aliases.iter().copied()) {
            let cand = cand.trim();
            if !cand.is_empty() && !out.iter().any(|c| c.eq_ignore_ascii_case(cand)) {
                out.push(cand);
            }
        }
        out
    }

    /// The first item in `items` matching a candidate. Candidate order wins
    /// over list order, so an item titled by the key beats one titled by host.
    fn pick<'i>(items: &'i [ItemSummary], candidates: &[&str]) -> Option<&'i ItemSummary> {
        candidates.iter().find_map(|cand| {
            items
                .iter()
                .find(|item| super::backend::root_alias_eq(&item.title, cand))
        })
    }

    /// Finds the item to read for `key`: the `RustConn` vault first, then —
    /// with root search on — every vault, matching the key or an alias.
    async fn find_for_read(
        &self,
        key: &str,
        aliases: &[&str],
    ) -> SecretResult<Option<ItemSummary>> {
        let vaults = self.list_vaults().await?;

        if let Some(own) = vaults.iter().find(|v| v.name == self.vault_name)
            && let Some(item) = self.own_items(&own.share_id, key).await?.into_iter().next()
        {
            return Ok(Some(item));
        }

        if !self.root_search {
            return Ok(None);
        }

        let candidates = Self::candidates(key, aliases);
        for vault in &vaults {
            let items = self.list_items(&vault.share_id).await?;
            if let Some(item) = Self::pick(&items, &candidates) {
                tracing::info!(
                    matched = %item.title,
                    vault = %vault.name,
                    "Proton Pass root search matched an item outside RustConn's own entry"
                );
                return Ok(Some(item.clone()));
            }
        }
        Ok(None)
    }

    /// Reads one field of an item. `None` when the item has no such field —
    /// which includes an empty one, since `pass-cli` does not list those.
    async fn read_field(
        &self,
        item: &ItemSummary,
        field: &str,
    ) -> SecretResult<Option<Zeroizing<String>>> {
        let args: [&str; 8] = [
            "item",
            "view",
            "--share-id",
            &item.share_id,
            "--item-id",
            &item.id,
            "--field",
            field,
        ];
        let output = self.run_raw(&args, None).await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains(FIELD_MISSING) {
                return Ok(None);
            }
            return Err(Self::failure(&args, &stderr));
        }
        let stdout = Zeroizing::new(output.stdout);
        let text = std::str::from_utf8(&stdout).map_err(|_| {
            SecretError::RetrieveFailed("pass-cli returned a non-UTF-8 field value".to_string())
        })?;
        // `pass-cli` adds one newline; a value's own whitespace is kept.
        let value = text
            .strip_suffix('\n')
            .map_or(text, |v| v.strip_suffix('\r').unwrap_or(v));
        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Zeroizing::new(value.to_string())))
        }
    }

    async fn read_credentials(&self, item: &ItemSummary) -> SecretResult<Credentials> {
        let username = self.read_field(item, "username").await?;
        let password = self.read_field(item, "password").await?;
        let domain = self.read_field(item, DOMAIN_FIELD).await?;
        Ok(Credentials {
            username: username.map(|u| u.as_str().to_owned()),
            password: password.map(|p| SecretString::from(p.as_str().to_owned())),
            domain: domain.map(|d| d.as_str().to_owned()),
        })
    }

    async fn retrieve_with_aliases(
        &self,
        key: &str,
        aliases: &[&str],
    ) -> SecretResult<Option<Credentials>> {
        let Some(item) = self.find_for_read(key, aliases).await? else {
            return Ok(None);
        };
        let creds = self.read_credentials(&item).await?;
        if creds.username.is_none() && creds.password.is_none() && creds.domain.is_none() {
            return Ok(None);
        }
        Ok(Some(creds))
    }

    /// Builds the login template JSON for `credentials` in a buffer that is
    /// wiped on drop.
    fn template_json(title: &str, credentials: &Credentials) -> SecretResult<Zeroizing<Vec<u8>>> {
        let mut extra_fields = Vec::new();
        if let Some(domain) = credentials.domain.as_deref().filter(|d| !d.is_empty()) {
            extra_fields.push(ExtraField {
                kind: "text",
                title: DOMAIN_FIELD,
                value: domain,
            });
        }
        let template = LoginTemplate {
            title,
            username: credentials.username.as_deref(),
            password: credentials.expose_password(),
            urls: [],
            extra_fields,
        };
        // Serialising plain strings cannot fail; the error path is mapped
        // without the serde message all the same. The buffer is sized for the
        // worst JSON escaping (6 bytes per char) so it never reallocates and
        // leaves a copy of the password in freed memory.
        let chars: usize = [
            title,
            credentials.username.as_deref().unwrap_or(""),
            credentials.domain.as_deref().unwrap_or(""),
            credentials.expose_password().unwrap_or(""),
        ]
        .iter()
        .map(|t| t.len())
        .sum();
        let mut buf = Zeroizing::new(Vec::with_capacity(chars * 6 + 256));
        serde_json::to_writer(&mut *buf, &template).map_err(|_| {
            SecretError::StoreFailed("Failed to build Proton Pass item".to_string())
        })?;
        Ok(buf)
    }

    async fn delete_item(&self, item: &ItemSummary) -> SecretResult<()> {
        self.run(&[
            "item",
            "delete",
            "--share-id",
            &item.share_id,
            "--item-id",
            &item.id,
        ])
        .await
        .map(drop)
        .map_err(|e| match e {
            SecretError::BackendUnavailable(_) => e,
            other => SecretError::DeleteFailed(other.to_string()),
        })
    }
}

impl Default for ProtonPassBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SecretBackend for ProtonPassBackend {
    async fn store(&self, connection_id: &str, credentials: &Credentials) -> SecretResult<()> {
        bounded(self.store_inner(connection_id, credentials)).await
    }
    async fn retrieve(&self, connection_id: &str) -> SecretResult<Option<Credentials>> {
        bounded(self.retrieve_with_aliases(connection_id, &[])).await
    }
    async fn retrieve_identity(
        &self,
        identity: super::backend::LookupIdentity<'_>,
    ) -> SecretResult<Option<Credentials>> {
        let aliases = identity.aliases();
        bounded(self.retrieve_with_aliases(identity.key, &aliases)).await
    }
    async fn delete(&self, connection_id: &str) -> SecretResult<()> {
        bounded(self.delete_inner(connection_id)).await
    }
    async fn is_available(&self) -> bool {
        // In-process PATH lookup: a session check would be an API round-trip
        // per secret operation, and a missing session is reported by the
        // operation itself.
        crate::which::is_available(PASS_CLI)
    }
    fn backend_id(&self) -> &'static str {
        "proton_pass"
    }
    fn display_name(&self) -> &'static str {
        "Proton Pass"
    }
    fn is_read_only(&self) -> bool {
        self.read_only
    }
    fn searches_from_root(&self) -> bool {
        self.root_search
    }
}

impl ProtonPassBackend {
    async fn store_inner(
        &self,
        connection_id: &str,
        credentials: &Credentials,
    ) -> SecretResult<()> {
        self.ensure_writable()?;
        let share_id = self.get_or_create_vault().await?;
        let existing = self.own_items(&share_id, connection_id).await?;

        // The item is recreated rather than edited, so a field the caller did
        // not supply would be lost. Carry it over from the item being replaced,
        // the same way a field-by-field backend leaves it in place.
        let mut merged = credentials.clone();
        if let Some(old) = existing.first()
            && (merged.username.is_none() || merged.password.is_none() || merged.domain.is_none())
        {
            let previous = self.read_credentials(old).await?;
            merged.username = merged.username.or(previous.username);
            merged.password = merged.password.or(previous.password);
            merged.domain = merged.domain.or(previous.domain);
        }

        let body = Self::template_json(connection_id, &merged)?;
        let args: [&str; 7] = [
            "item",
            "create",
            "login",
            "--share-id",
            &share_id,
            "--from-template",
            "-",
        ];
        let output = self.run_raw(&args, Some(body.as_slice())).await?;
        if !output.status.success() {
            // The CLI's stderr is not repeated here: a template parse error can
            // quote the offending value, which is the password.
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(if stderr.contains(NOT_AUTHENTICATED) {
                Self::failure(&args, &stderr)
            } else {
                SecretError::StoreFailed(format!("pass-cli item create failed ({})", output.status))
            });
        }

        // Only now that the new item exists are the ones it replaces removed.
        // Every old item is attempted even if one fails, so a duplicate left
        // behind is as small as possible; the next save removes any that stay.
        let mut failure = None;
        for old in &existing {
            if let Err(e) = self.delete_item(old).await {
                failure.get_or_insert(e);
            }
        }
        match failure {
            None => Ok(()),
            Some(e) => Err(SecretError::StoreFailed(format!(
                "Saved the new Proton Pass item but could not remove the old one: {e}"
            ))),
        }
    }

    async fn delete_inner(&self, connection_id: &str) -> SecretResult<()> {
        self.ensure_writable()?;
        let Some(share_id) = self.find_vault().await? else {
            return Ok(());
        };
        for item in self.own_items(&share_id, connection_id).await? {
            self.delete_item(&item).await?;
        }
        Ok(())
    }
}
impl std::fmt::Debug for ProtonPassBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProtonPassBackend")
            .field("vault_name", &self.vault_name)
            .field("read_only", &self.read_only)
            .field("root_search", &self.root_search)
            .field("program", &self.program)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, title: &str) -> ItemSummary {
        ItemSummary {
            id: id.to_string(),
            share_id: "S1".to_string(),
            title: title.to_string(),
        }
    }

    #[test]
    fn debug_does_not_leak_secret() {
        // The backend holds no secret; the sentinel guards a future field.
        let backend = ProtonPassBackend::new().with_vault_name("Ops");
        let rendered = format!("{backend:?}");
        assert!(rendered.contains("ProtonPassBackend"));
        assert!(rendered.contains("vault_name"));
    }

    #[test]
    fn read_only_and_root_search_toggles() {
        let b = ProtonPassBackend::new();
        assert!(!b.is_read_only());
        assert!(!b.searches_from_root());
        let both = ProtonPassBackend::new()
            .with_read_only(true)
            .with_root_search(true);
        assert!(both.is_read_only());
        assert!(both.searches_from_root());
    }

    #[test]
    fn from_secret_settings_applies_toggles() {
        let settings = crate::config::SecretSettings {
            proton_pass_read_only: true,
            proton_pass_root_search: true,
            ..Default::default()
        };
        let b = ProtonPassBackend::from_secret_settings(&settings);
        assert!(b.is_read_only());
        assert!(b.searches_from_root());
    }

    #[tokio::test]
    async fn read_only_refuses_writes_before_spawning() {
        // A program that cannot exist: reaching it would be a spawn error, not
        // ReadOnly.
        let b = ProtonPassBackend::new()
            .with_read_only(true)
            .with_program("/nonexistent/pass-cli".to_string());
        let err = b.store("k", &Credentials::default()).await.unwrap_err();
        assert!(matches!(err, SecretError::ReadOnly(name) if name == "Proton Pass"));
        let err = b.delete("k").await.unwrap_err();
        assert!(matches!(err, SecretError::ReadOnly(_)));
    }

    #[test]
    fn not_authenticated_maps_to_backend_unavailable() {
        let err = ProtonPassBackend::failure(
            &["item", "list"],
            "Error: This operation requires an authenticated client",
        );
        assert!(matches!(err, SecretError::BackendUnavailable(_)));
        let err = ProtonPassBackend::failure(&["item", "list"], "boom");
        assert!(matches!(err, SecretError::ConnectionFailed(m) if m.contains("item list")));
    }

    #[test]
    fn template_carries_fields_and_omits_empty_domain() {
        let creds = Credentials {
            username: Some("alice".to_string()),
            password: Some(SecretString::from("hunter2".to_string())),
            domain: Some("CORP".to_string()),
        };
        let json = ProtonPassBackend::template_json("rustconn/web", &creds).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(v["title"], "rustconn/web");
        assert_eq!(v["username"], "alice");
        assert_eq!(v["password"], "hunter2");
        assert_eq!(v["urls"], serde_json::json!([]));
        assert_eq!(
            v["extra_fields"],
            serde_json::json!([{"kind": "text", "title": "domain", "value": "CORP"}])
        );

        let bare = ProtonPassBackend::template_json("t", &Credentials::default()).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bare).unwrap();
        assert_eq!(v["extra_fields"], serde_json::json!([]));
        assert!(v["password"].is_null());
    }

    #[test]
    fn root_pick_prefers_candidate_order_and_matches_exactly() {
        let items = vec![
            item("1", "db.example.com"),
            item("2", "rustconn/Prod DB"),
            item("3", "Prod DB (old)"),
        ];
        let cands =
            ProtonPassBackend::candidates("rustconn/Prod DB", &["db.example.com", "Prod DB"]);
        // The key wins even though the host item is listed first.
        assert_eq!(ProtonPassBackend::pick(&items, &cands).unwrap().id, "2");
        // Host alias, case-insensitive.
        let cands = ProtonPassBackend::candidates("rustconn/x", &["DB.EXAMPLE.COM"]);
        assert_eq!(ProtonPassBackend::pick(&items, &cands).unwrap().id, "1");
        // Never a substring.
        let cands = ProtonPassBackend::candidates("rustconn/x", &["Prod"]);
        assert!(ProtonPassBackend::pick(&items, &cands).is_none());
    }

    #[test]
    fn candidates_dedupe_and_drop_blanks() {
        assert_eq!(
            ProtonPassBackend::candidates("k", &["", " K ", "host"]),
            vec!["k", "host"]
        );
    }

    #[test]
    fn parses_cli_json_shapes() {
        let vaults: VaultList = serde_json::from_str(
            r#"{"vaults":[{"name":"RustConn","vault_id":"v","share_id":"S1"}]}"#,
        )
        .unwrap();
        assert_eq!(vaults.vaults[0].share_id, "S1");
        let items: ItemList = serde_json::from_str(
            r#"{"items":[{"id":"I1","share_id":"S1","vault_id":"v","state":"Active",
                "flags":[],"create_time":"2026-01-01T00:00:00","modify_time":"2026-01-01T00:00:00",
                "last_use_time":null,"title":"rustconn/web","item_type":"login"}],"folders":[]}"#,
        )
        .unwrap();
        assert_eq!(items.items[0].title, "rustconn/web");
    }
}

#[cfg(test)]
#[cfg(unix)]
mod stub_tests {
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::Path;

    use super::*;

    const PASSWORD: &str = "argv-must-not-see-this";

    /// A plain-file stand-in for `pass-cli`. Every argv is appended to
    /// `argv.log`; vault and item lists come from files the test writes;
    /// `item create` saves its stdin; `item view --field F` prints `field_F`.
    fn stub(dir: &Path) -> String {
        let d = dir.display();
        format!(
            "#!/bin/sh\n\
             D='{d}'\n\
             echo \"$*\" >> \"$D/argv.log\"\n\
             case \"$1 $2\" in\n\
               'vault list') cat \"$D/vaults.json\" ;;\n\
               'vault create') echo S1 ;;\n\
               'item list') cat \"$D/items.json\" ;;\n\
               'item create') cat > \"$D/created.json\" ;;\n\
               'item delete') echo \"$6\" >> \"$D/deleted\" ;;\n\
               'item view') f=\"$D/field_$8\"; [ -f \"$f\" ] || {{ echo \"Error: Field does not exist: $8\" >&2; exit 1; }}; cat \"$f\"; echo ;;\n\
               *) exit 2 ;;\n\
             esac\n"
        )
    }

    fn setup() -> (tempfile::TempDir, ProtonPassBackend) {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("pass-cli");
        std::fs::write(&path, stub(dir.path())).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(
            dir.path().join("vaults.json"),
            r#"{"vaults":[{"name":"RustConn","vault_id":"v","share_id":"S1"}]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("items.json"),
            r#"{"items":[{"id":"OLD","share_id":"S1","title":"rustconn/web"}]}"#,
        )
        .unwrap();
        let backend = ProtonPassBackend::new().with_program(path.to_string_lossy().into_owned());
        (dir, backend)
    }

    /// Retries the ETXTBSY a just-written stub can hit under parallel tests.
    async fn retry<T, F, Fut>(mut f: F) -> SecretResult<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = SecretResult<T>>,
    {
        for _ in 0..10 {
            match f().await {
                Err(e) if e.to_string().contains("Text file busy") => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                other => return other,
            }
        }
        f().await
    }

    #[tokio::test]
    async fn store_pipes_the_password_and_replaces_the_old_item() {
        let (dir, backend) = setup();
        let creds = Credentials {
            username: Some("alice".to_string()),
            password: Some(SecretString::from(PASSWORD.to_string())),
            domain: None,
        };
        retry(|| backend.store("rustconn/web", &creds))
            .await
            .unwrap();

        let argv = std::fs::read_to_string(dir.path().join("argv.log")).unwrap();
        assert!(!argv.contains(PASSWORD), "password reached argv: {argv}");
        let created = std::fs::read_to_string(dir.path().join("created.json")).unwrap();
        assert!(created.contains(PASSWORD));
        assert!(created.contains("rustconn/web"));
        // The old item goes only after the new one is written.
        let create_at = argv.find("item create").unwrap();
        let delete_at = argv.find("item delete").unwrap();
        assert!(create_at < delete_at);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("deleted"))
                .unwrap()
                .trim(),
            "OLD"
        );
    }

    #[tokio::test]
    async fn store_keeps_a_field_the_caller_left_out() {
        let (dir, backend) = setup();
        std::fs::write(dir.path().join("field_password"), "kept-secret").unwrap();
        let creds = Credentials {
            username: Some("bob".to_string()),
            password: None,
            domain: None,
        };
        retry(|| backend.store("rustconn/web", &creds))
            .await
            .unwrap();
        let created = std::fs::read_to_string(dir.path().join("created.json")).unwrap();
        assert!(created.contains("kept-secret"));
        assert!(created.contains("bob"));
    }

    #[tokio::test]
    async fn retrieve_reads_fields_and_treats_missing_ones_as_none() {
        let (dir, backend) = setup();
        std::fs::write(dir.path().join("field_username"), "alice").unwrap();
        std::fs::write(dir.path().join("field_password"), " spaced pw ").unwrap();
        let creds = retry(|| backend.retrieve("rustconn/web"))
            .await
            .unwrap()
            .expect("found");
        assert_eq!(creds.username.as_deref(), Some("alice"));
        assert_eq!(creds.expose_password(), Some(" spaced pw "));
        assert_eq!(creds.domain, None);

        assert!(
            retry(|| backend.retrieve("rustconn/other"))
                .await
                .unwrap()
                .is_none()
        );
    }

    /// `pass-cli` resolves a duplicated name to the first match, and so does
    /// the backend: the first listed item is the one read.
    #[tokio::test]
    async fn retrieve_reads_the_first_of_duplicate_items() {
        let (dir, backend) = setup();
        std::fs::write(
            dir.path().join("items.json"),
            r#"{"items":[
                {"id":"FIRST","share_id":"S1","title":"rustconn/web"},
                {"id":"SECOND","share_id":"S1","title":"rustconn/web"}]}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("field_username"), "alice").unwrap();
        retry(|| backend.retrieve("rustconn/web"))
            .await
            .unwrap()
            .expect("found");
        let argv = std::fs::read_to_string(dir.path().join("argv.log")).unwrap();
        assert!(argv.contains("--item-id FIRST"), "{argv}");
        assert!(!argv.contains("SECOND"), "{argv}");
    }
    #[tokio::test]
    async fn failed_create_does_not_repeat_the_cli_stderr() {
        let (dir, backend) = setup();
        let path = dir.path().join("pass-cli");
        let script = stub(dir.path()).replace(
            "'item create') cat > \"$D/created.json\" ;;",
            &format!(
                "'item create') cat > /dev/null; echo \"bad value {PASSWORD}\" >&2; exit 1 ;;"
            ),
        );
        std::fs::write(&path, script).unwrap();
        let creds = Credentials {
            username: Some("alice".to_string()),
            password: Some(SecretString::from(PASSWORD.to_string())),
            domain: None,
        };
        let err = retry(|| backend.store("rustconn/web", &creds))
            .await
            .unwrap_err();
        assert!(!err.to_string().contains(PASSWORD), "leaked: {err}");
        assert!(
            !dir.path().join("deleted").exists(),
            "the old item must stay when create fails"
        );
    }
    #[tokio::test]
    async fn flag_like_vault_name_is_refused_before_spawning() {
        let (dir, backend) = setup();
        std::fs::write(dir.path().join("vaults.json"), r#"{"vaults":[]}"#).unwrap();
        let backend = backend.with_vault_name("--share-id");
        let creds = Credentials::default();
        let err = retry(|| backend.store("rustconn/web", &creds))
            .await
            .unwrap_err();
        assert!(matches!(err, SecretError::StoreFailed(_)));
        let argv = std::fs::read_to_string(dir.path().join("argv.log")).unwrap();
        assert!(!argv.contains("vault create"), "{argv}");
    }

    #[tokio::test]
    async fn flag_like_ids_from_the_cli_are_dropped() {
        let (dir, backend) = setup();
        std::fs::write(
            dir.path().join("items.json"),
            r#"{"items":[{"id":"--evil","share_id":"S1","title":"rustconn/web"}]}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("field_password"), "x").unwrap();
        assert!(
            retry(|| backend.retrieve("rustconn/web"))
                .await
                .unwrap()
                .is_none()
        );
        retry(|| backend.delete("rustconn/web")).await.unwrap();
        let argv = std::fs::read_to_string(dir.path().join("argv.log")).unwrap();
        assert!(!argv.contains("--evil"), "{argv}");
    }

    #[tokio::test]
    async fn delete_removes_only_exact_title_matches() {
        let (dir, backend) = setup();
        retry(|| backend.delete("rustconn/web")).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("deleted"))
                .unwrap()
                .trim(),
            "OLD"
        );
        std::fs::remove_file(dir.path().join("deleted")).unwrap();
        retry(|| backend.delete("rustconn/we")).await.unwrap();
        assert!(!dir.path().join("deleted").exists());
    }
}
