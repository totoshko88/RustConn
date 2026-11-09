//! Bitwarden CLI backend for password management
//!
//! This module implements credential storage using the Bitwarden CLI (`bw`).
//! It supports both cloud and self-hosted Bitwarden instances.
//!
//! # Authentication Methods
//!
//! The Bitwarden CLI supports several authentication methods:
//!
//! 1. **Email and Password** (interactive) - `bw login`
//! 2. **API Key** (automated) - Using `BW_CLIENTID` and `BW_CLIENTSECRET` environment variables
//! 3. **SSO** - `bw login --sso`
//!
//! After login, the vault must be unlocked with `bw unlock` to access credentials.
//! The unlock command returns a session key that must be passed to subsequent commands.
//!
//! # Session Management
//!
//! Session keys are valid until:
//! - `bw lock` is called
//! - `bw logout` is called
//! - A new terminal session is started (keys don't persist)
//!
//! # Usage Example
//!
//! ```ignore
//! use rustconn_core::secret::{BitwardenBackend, unlock_vault};
//! use secrecy::SecretString;
//!
//! // Unlock vault with master password
//! let password = SecretString::from("master_password");
//! let session_key = unlock_vault(&password).await?;
//!
//! // Create backend with session
//! let backend = BitwardenBackend::with_session(session_key);
//!
//! // Store credentials
//! backend.store("my-server", &credentials).await?;
//! ```

use std::process::Stdio;
use std::sync::RwLock;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use super::backend::SecretBackend;
use super::serde_helpers::serde_error_kind;
use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

/// Thread-safe in-process storage for the Bitwarden session key.
///
/// Replaces `std::env::set_var("BW_SESSION", ...)` which is unsafe in
/// multi-threaded contexts (Rust 1.66+, hard error in edition 2024).
/// The session key is passed to `bw` commands via `--session` CLI arg
/// in [`BitwardenBackend::build_command`], so child processes do not
/// need the environment variable.
static BW_SESSION_STORE: RwLock<Option<SecretString>> = RwLock::new(None);

/// Thread-safe in-process storage for the resolved `bw` CLI command path.
///
/// In Flatpak sandboxes, `bw` is not on the default PATH. The UI layer
/// detects the correct binary path (e.g.
/// `~/.var/app/io.github.totoshko88.RustConn/cli/bitwarden/bw`) and
/// stores it here so that all backend functions use the same resolved path.
///
/// Falls back to `"bw"` when no custom path has been stored.
static BW_CMD_STORE: RwLock<Option<String>> = RwLock::new(None);

/// Timestamp of the last successful vault unlock/status verification.
///
/// Used to skip redundant `bw status` calls when the session key is
/// already present and was verified recently. The threshold is
/// [`UNLOCK_VERIFY_TTL_SECS`].
static BW_LAST_VERIFIED: RwLock<Option<std::time::Instant>> = RwLock::new(None);

/// How long (in seconds) a successful `bw status` result is trusted
/// before re-checking. Keeps reconnect fast while still detecting
/// vault locks within a reasonable window.
const UNLOCK_VERIFY_TTL_SECS: u64 = 120;

/// Timestamp of the last successful `bw sync`.
///
/// Kept apart from [`BW_LAST_VERIFIED`] so that re-trusting a stored session
/// after the short verify TTL does not cost a `bw sync` (seconds) every time;
/// see [`SYNC_TTL_SECS`].
///
/// Wall-clock `SystemTime`, not `Instant`: on Linux `Instant` stops while the
/// machine is suspended, so after an overnight sleep a sync taken just before
/// it would still look recent and the first connects would skip `bw sync`.
static BW_LAST_SYNCED: RwLock<Option<std::time::SystemTime>> = RwLock::new(None);

/// How long (in seconds) after a successful `bw sync` an [`auto_unlock`] that
/// re-trusts a stored session skips the next one.
///
/// The cost: an item edited in another Bitwarden client is seen up to this much
/// later. A refused sign-in still forces a sync at once, because
/// [`sync_on_next_unlock`] clears this mark too (issue #351), and a fresh unlock
/// always syncs.
const SYNC_TTL_SECS: u64 = 600;

/// One Bitwarden unlock at a time, process-wide.
///
/// [`auto_unlock`] holds it across its `bw` calls, and
/// [`unlock_vault_blocking`] — the Settings → Secrets entry — takes it too, so
/// the page's unlock no longer runs alongside a connect's (issue #353 logs show
/// two or three `bw unlock` at once). A tokio mutex because `auto_unlock`
/// holds the guard across awaits; it is not tied to a runtime, which matters
/// because each caller brings its own, and its `blocking_lock` serves the
/// blocking entry.
static BW_UNLOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// One Bitwarden `bw sync` at a time, process-wide.
///
/// The unlock path no longer syncs (#353), so the first retrieve/store after an
/// unlock — and the startup thread's background warm-up — each run a
/// `sync_due()`-gated sync. Without a lock a connect made the instant the
/// background sync starts sees `sync_due()` still true (neither has called
/// `mark_synced()` yet) and fires a *second* `bw sync` alongside the first,
/// which the #353-follow-up trace showed costing a retrieve the full ~13 s.
/// [`BitwardenBackend::sync_if_due`] takes this guard and re-checks `sync_due()`
/// under it, so the second caller waits, then finds the vault freshly synced and
/// does nothing. Unconditional [`BitwardenBackend::sync`] (explicit user sync,
/// the #351 refused-retry, the store out-of-date retry) does not take it — those
/// must sync regardless of the TTL.
static BW_SYNC: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The persistent `bw serve` transport (feature `bitwarden-serve`, W2 P3).
///
/// Holds the running serve child once started, and a `disabled` latch set when
/// the transport failed and the session fell back to one-shot spawns. Behind a
/// tokio mutex because [`BitwardenBackend::run_command_inner`] holds it across
/// the HTTP await, and because one serve child serves the whole process (the
/// GUI already serializes vault ops under `VAULT_OP_LOCK`, and `bw serve` itself
/// races under concurrent writes — bitwarden/clients#20951).
#[cfg(feature = "bitwarden-serve")]
static BW_SERVE: tokio::sync::Mutex<ServeManager> =
    tokio::sync::Mutex::const_new(ServeManager::new());

/// Whether the serve transport is PREFERRED. Off by default: the backend keeps
/// spawning one `bw` per call until the maintainer opts in (Settings → Secrets,
/// `bitwarden_persistent_cli`). Separate from [`BW_SERVE`] so the preference can
/// be read without taking the child's lock.
#[cfg(feature = "bitwarden-serve")]
static BW_SERVE_PREFERRED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Set whenever the in-process session key changes out of band (a spawn-path
/// `bw unlock`, or a clear on lock/logout). A running serve child came up with
/// the *previous* session — or none — and keeps its own lock state in the Node
/// process's memory, which does not pick up a `BW_SESSION` that appears after it
/// started. So an unlock done outside serve leaves the serve child still locked,
/// and its next data op fails with "Vault is locked" (seen in a real run before
/// the one-shot respawn recovered it). This flag makes the next serve op respawn
/// the child with the current session *before* the op, turning that recovered
/// failure into a clean start. `AtomicBool` because [`set_session_key`] /
/// [`clear_session_key`] are sync and cannot take the async [`BW_SERVE`] lock.
#[cfg(feature = "bitwarden-serve")]
static BW_SERVE_STALE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Marks the running serve child stale so the next serve op respawns it with the
/// current session. A no-op without the feature.
#[cfg(feature = "bitwarden-serve")]
fn mark_serve_session_stale() {
    BW_SERVE_STALE.store(true, std::sync::atomic::Ordering::Relaxed);
}
#[cfg(not(feature = "bitwarden-serve"))]
const fn mark_serve_session_stale() {}

/// The process-wide serve child plus the session fallback latch.
#[cfg(feature = "bitwarden-serve")]
struct ServeManager {
    process: Option<super::bitwarden_serve::ServeProcess>,
    /// Set once a serve failure made the session fall back to spawns; stays set
    /// until a lock/logout/rebuild clears it, so a dead CLI is not retried on
    /// every call.
    disabled: bool,
}

#[cfg(feature = "bitwarden-serve")]
impl ServeManager {
    const fn new() -> Self {
        Self {
            process: None,
            disabled: false,
        }
    }
}

/// Turns the preferred serve transport on or off (Settings → Secrets toggle).
///
/// Turning it off also shuts any running child down on the next lock/rebuild;
/// turning it on lets the next vault op start one lazily.
#[cfg(feature = "bitwarden-serve")]
pub fn set_serve_preferred(preferred: bool) {
    BW_SERVE_PREFERRED.store(preferred, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the serve transport is currently preferred.
#[cfg(feature = "bitwarden-serve")]
#[must_use]
pub fn is_serve_preferred() -> bool {
    BW_SERVE_PREFERRED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Shuts down any running serve child and clears the session fallback latch.
///
/// Called on lock, logout, backend rebuild and settings changes: the next vault
/// op then starts a fresh child (if still preferred) with the current session.
#[cfg(feature = "bitwarden-serve")]
pub async fn shutdown_serve() {
    let mut guard = BW_SERVE.lock().await;
    guard.disabled = false;
    BW_SERVE_STALE.store(false, std::sync::atomic::Ordering::Relaxed);
    // This stops the child itself, which is all a pending lock-event stop asks.
    BW_SERVE_STOP_PENDING.store(false, std::sync::atomic::Ordering::SeqCst);
    if let Some(process) = guard.process.take() {
        process.shutdown().await;
    }
}

/// Set by [`stop_serve_on_lock`] when a vault op held the serve child at that
/// moment; the op stops the child as soon as it has finished.
#[cfg(feature = "bitwarden-serve")]
static BW_SERVE_STOP_PENDING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Stops the persistent `bw serve` child on screen lock or suspend.
///
/// The decrypted vault then does not stay in a long-lived process's memory
/// while the user is away. Meant for a [`crate::lock_events`] handler.
///
/// Only the child goes. The session key stays in this process, as it does with
/// the spawn-per-call transport, so the next vault op starts a fresh child that
/// comes up unlocked and nothing asks for the master password again. A child
/// busy with a vault op at this moment is stopped by that op once it returns.
///
/// Never blocks and needs no runtime. Returns whether a child was stopped now.
#[cfg(feature = "bitwarden-serve")]
#[must_use]
pub fn stop_serve_on_lock() -> bool {
    stop_serve_now_or_after_op(&BW_SERVE, &BW_SERVE_STOP_PENDING)
}

/// [`stop_serve_on_lock`] with the serve slot and the pending flag passed in, so
/// tests do not share the process-wide child.
///
/// The flag is raised *before* the `try_lock`: an op that holds the slot reads
/// the flag only after releasing it ([`finish_pending_serve_stop`]), so either
/// the `try_lock` here succeeds or that op sees the flag. No interleaving leaves
/// the child running.
#[cfg(feature = "bitwarden-serve")]
fn stop_serve_now_or_after_op(
    serve: &tokio::sync::Mutex<ServeManager>,
    pending: &std::sync::atomic::AtomicBool,
) -> bool {
    pending.store(true, std::sync::atomic::Ordering::SeqCst);
    let Ok(mut manager) = serve.try_lock() else {
        return false;
    };
    pending.store(false, std::sync::atomic::Ordering::SeqCst);
    manager
        .process
        .take()
        .map(super::bitwarden_serve::ServeProcess::kill)
        .is_some()
}

/// Stops the serve child if a lock event found it busy. Called by every serve op
/// after it has released the slot.
#[cfg(feature = "bitwarden-serve")]
async fn finish_pending_serve_stop(
    serve: &tokio::sync::Mutex<ServeManager>,
    pending: &std::sync::atomic::AtomicBool,
) -> bool {
    if !pending.swap(false, std::sync::atomic::Ordering::SeqCst) {
        return false;
    }
    let stopped = serve.lock().await.process.take();
    match stopped {
        Some(process) => {
            process.shutdown().await;
            true
        }
        None => false,
    }
}

/// No-op stand-ins so call sites do not need `#[cfg]` around every use.
#[cfg(not(feature = "bitwarden-serve"))]
pub fn set_serve_preferred(_preferred: bool) {}
/// Nothing to stop without the feature.
#[cfg(not(feature = "bitwarden-serve"))]
#[must_use]
pub const fn stop_serve_on_lock() -> bool {
    false
}
/// Always false without the feature.
#[cfg(not(feature = "bitwarden-serve"))]
#[must_use]
pub const fn is_serve_preferred() -> bool {
    false
}
/// Nothing to shut down without the feature.
#[cfg(not(feature = "bitwarden-serve"))]
pub async fn shutdown_serve() {}

/// Ceiling on a single `bw` invocation.
///
/// One number for "one child process", so the guarantee is the same wherever a
/// `bw` is spawned. [`BitwardenBackend::run_command`] had it as a literal 30 and
/// was the only place that honoured it; [`unlock_vault_sync`] spawns up to three
/// children and bounded none of them, which is what let an unlock sit
/// indefinitely on a stalled network sync while the GUI's status row waited for
/// an answer that was never coming. A `gio::spawn_blocking` worker is cheap but
/// not free, and there are only a handful of them.
///
/// Thirty seconds is generous for a call that normally costs a few — measured at
/// about 4.5 s for a real unlock in issue
/// [#312](https://github.com/totoshko88/RustConn/issues/312) — and the point is
/// to bound a hang, not to race a slow vault server.
pub(crate) const BW_INVOCATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Records that the vault was just verified as unlocked.
fn mark_verified() {
    if let Ok(mut guard) = BW_LAST_VERIFIED.write() {
        *guard = Some(std::time::Instant::now());
    }
}

/// Returns `true` if the vault was verified as unlocked within the TTL.
fn is_recently_verified() -> bool {
    BW_LAST_VERIFIED
        .read()
        .ok()
        .and_then(|guard| *guard)
        .is_some_and(|t| t.elapsed().as_secs() < UNLOCK_VERIFY_TTL_SECS)
}

/// Makes the next [`auto_unlock`] check the session again and run `bw sync`.
///
/// For [`UNLOCK_VERIFY_TTL_SECS`] after a check, `auto_unlock` hands back the
/// stored session without running anything, so `bw` keeps answering from its
/// local copy of the vault, which does not see an item edited in another
/// Bitwarden client until the next sync. Call this when credentials read from
/// the vault were just refused, so the retry reads the item fresh (issue #351).
pub fn sync_on_next_unlock() {
    clear_verified();
    if let Ok(mut guard) = BW_LAST_SYNCED.write() {
        *guard = None;
    }
}

/// Records that `bw sync` just succeeded.
fn mark_synced() {
    if let Ok(mut guard) = BW_LAST_SYNCED.write() {
        *guard = Some(std::time::SystemTime::now());
    }
}

/// Returns `true` if the vault should be synced: never synced in this process,
/// or the last sync is older than [`SYNC_TTL_SECS`].
fn sync_due() -> bool {
    is_sync_due(BW_LAST_SYNCED.read().ok().and_then(|guard| *guard))
}

/// [`sync_due`] for a given last-sync time, so the TTL is testable without
/// waiting it out. A mark in the future (the clock was set back) counts as due.
fn is_sync_due(last_synced: Option<std::time::SystemTime>) -> bool {
    last_synced.is_none_or(|t| !t.elapsed().is_ok_and(|age| age.as_secs() < SYNC_TTL_SECS))
}

/// Clears the verification timestamp (e.g. on lock/logout).
fn clear_verified() {
    if let Ok(mut guard) = BW_LAST_VERIFIED.write() {
        *guard = None;
    }
}

/// Stores the Bitwarden session key in thread-safe in-process storage.
///
/// Call this instead of `std::env::set_var("BW_SESSION", ...)`.
/// The key is used by [`get_session_key`] and passed to `bw` commands
/// via `--session` argument.
pub fn set_session_key(key: SecretString) {
    if let Ok(mut guard) = BW_SESSION_STORE.write() {
        *guard = Some(key);
    }
    // A serve child started before this unlock is still locked (it does not see
    // a BW_SESSION that appears after it spawned); respawn it on the next op.
    mark_serve_session_stale();
}

/// Retrieves the Bitwarden session key from thread-safe in-process storage.
///
/// Returns `None` if no session key has been stored or if the lock is poisoned.
#[must_use]
pub fn get_session_key() -> Option<SecretString> {
    BW_SESSION_STORE.read().ok().and_then(|guard| guard.clone())
}

/// Returns `true` if a Bitwarden session key is stored in-process.
///
/// The same answer as `get_session_key().is_some()`, without copying the key.
/// A stored key means an unlock succeeded in this process; it is trusted the
/// same way [`BitwardenBackend::is_unlocked_fast`] trusts it, because
/// `bw status` on CLI v2026.4+ reports `locked` even with a valid session.
#[must_use]
pub fn has_session_key() -> bool {
    BW_SESSION_STORE.read().is_ok_and(|guard| guard.is_some())
}

/// Clears the stored Bitwarden session key.
pub fn clear_session_key() {
    if let Ok(mut guard) = BW_SESSION_STORE.write() {
        *guard = None;
    }
    clear_verified();
    // The serve child now holds a session we no longer trust; respawn it on the
    // next op (lock_vault/logout also shut it down outright).
    mark_serve_session_stale();
}

/// Clears every process-global Bitwarden unlock mark: the session key, the
/// verify timestamp and the last-sync timestamp.
///
/// Call this when the user switches their preferred secret backend away from
/// Bitwarden. [`SecretManager::rebuild_from_settings`](crate::secret::SecretManager::rebuild_from_settings)
/// rebuilds the backend chain but leaves these process-global marks in place,
/// so without this a stale session key would keep answering `is_unlocked_fast`
/// and `sync_due` from the previous backend. Unlike [`sync_on_next_unlock`] —
/// which clears the verify and sync marks but keeps the session for a fresh
/// re-check — this also drops the session key, because the backend it unlocked
/// is no longer the preferred one.
pub fn clear_bitwarden_session_state() {
    clear_session_key();
    if let Ok(mut guard) = BW_LAST_SYNCED.write() {
        *guard = None;
    }
}

/// Stores the resolved `bw` CLI command path.
///
/// Call this from the UI layer after detecting the correct binary path.
/// All backend functions will use this path instead of bare `"bw"`.
pub fn set_bw_cmd(cmd: &str) {
    if let Ok(mut guard) = BW_CMD_STORE.write() {
        *guard = Some(cmd.to_string());
        // This only caches the resolved `bw` binary path from backend detection
        // (the Secrets settings page probes every backend to fill its Status
        // rows). It is NOT a vault lookup — a KeePass user seeing this line in a
        // credential-resolution log was alarmed into thinking Bitwarden was
        // being queried for their secret, which it is not (issue #353). The
        // wording now says so.
        tracing::debug!(bw_cmd = %cmd, "Bitwarden: cached CLI binary path from backend detection (not a vault lookup)");
    }
}

/// Returns the stored `bw` CLI command path, or `"bw"` as default.
#[must_use]
pub fn get_bw_cmd() -> String {
    BW_CMD_STORE
        .read()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_else(|| "bw".to_string())
}

/// Creates a [`Command`] for the resolved `bw` binary with extended PATH.
///
/// On macOS, GUI apps launched via `.app` bundle have minimal PATH.
/// This helper ensures `bw` (and any tools it invokes) are discoverable.
fn bw_command(bw_cmd: &str) -> Command {
    let mut cmd = Command::new(bw_cmd);
    cmd.env("PATH", crate::cli_download::get_extended_path());
    // Every call here is wrapped in a timeout, and a timeout drops the `output()`
    // future rather than signalling the child. `tokio::process` does not kill on
    // drop by default, so without this a `bw` that hung on an unreachable server
    // outlived the timeout as an orphan — still holding a vault session.
    cmd.kill_on_drop(true);
    cmd
}

/// Resolves the `bw` CLI command path by probing known locations.
///
/// Checks (in order):
/// 1. Previously stored path via [`set_bw_cmd`]
/// 2. Flatpak CLI install directory (`~/.var/app/<app-id>/cli/bitwarden/bw`)
/// 3. Bare `"bw"` on PATH
///
/// Stores and returns the first working path.
#[must_use]
pub fn resolve_bw_cmd() -> String {
    // If already resolved, return cached value
    if let Ok(guard) = BW_CMD_STORE.read()
        && let Some(ref cmd) = *guard
    {
        return cmd.clone();
    }

    let mut candidates: Vec<String> = vec![];

    // Flatpak CLI directory
    if let Some(cli_dir) = crate::cli_download::get_cli_install_dir() {
        let flatpak_bw = cli_dir.join("bitwarden").join("bw");
        if flatpak_bw.exists() {
            candidates.push(flatpak_bw.to_string_lossy().to_string());
        }
    }

    // Non-Flatpak system paths
    if !crate::flatpak::is_flatpak() {
        candidates.extend(["/snap/bin/bw".to_string(), "/usr/local/bin/bw".to_string()]);
    }

    // Default bare command (relies on PATH)
    candidates.push("bw".to_string());

    for candidate in &candidates {
        if std::process::Command::new(candidate)
            .env("PATH", crate::cli_download::get_extended_path())
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            set_bw_cmd(candidate);
            return candidate.clone();
        }
    }

    // Nothing found — return "bw" as fallback (will produce clear error on use)
    "bw".to_string()
}

/// Bitwarden CLI backend
///
/// This backend uses the `bw` command-line utility to interact with
/// Bitwarden vaults. Requires the user to be logged in and have an
/// active session.
pub struct BitwardenBackend {
    /// Session key for authenticated operations
    session_key: Option<SecretString>,
    /// Custom server URL (for self-hosted instances)
    server_url: Option<String>,
    /// Organization ID (optional, for org vaults)
    organization_id: Option<String>,
    /// Folder name for RustConn entries
    folder_name: String,
    /// When true, every mutating operation is refused with
    /// [`SecretError::ReadOnly`] and the vault is left untouched.
    read_only: bool,
    /// When true, credential reads also match an entry titled by the bare
    /// connection id, not only the `RustConn: {id}` convention, so entries
    /// outside the RustConn folder are found.
    root_search: bool,
    /// Resolved path to the `bw` CLI binary
    bw_cmd: String,
}

/// Bitwarden item structure for JSON parsing
#[derive(Debug, Deserialize)]
struct BitwardenItem {
    id: String,
    name: String,
    login: Option<BitwardenLogin>,
    notes: Option<String>,
    /// Custom fields. A field named `domain` (any case) supplies the Windows
    /// domain for an item the user keeps outside RustConn.
    #[serde(default)]
    fields: Option<Vec<BitwardenField>>,
}

/// One custom field as `bw` returns it (`{"name": …, "value": …, "type": …}`).
#[derive(Debug, Deserialize)]
struct BitwardenField {
    name: Option<String>,
    value: Option<String>,
}

/// Bitwarden login structure
#[derive(Debug, Deserialize)]
struct BitwardenLogin {
    username: Option<String>,
    #[serde(
        default,
        deserialize_with = "super::serde_helpers::deserialize_optional_secret"
    )]
    password: Option<SecretString>,
    /// Login URIs. `bw list items --search` matches them too, so root search
    /// can find an item through its URI rather than its title (issue #353).
    #[serde(default)]
    uris: Option<Vec<BitwardenLoginUri>>,
}

/// One login URI as `bw` returns it (`{"match": …, "uri": …}`).
#[derive(Debug, Deserialize)]
struct BitwardenLoginUri {
    uri: Option<String>,
}

/// How a root-search item matched, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RootMatch {
    /// The title equals a candidate.
    Title,
    /// A login URI's host equals a candidate.
    UriHost,
    /// The first DNS label of the title or of a URI host equals a dotless
    /// candidate (`hanfile03.corp.example` for `hanfile03`), or the title is
    /// the first label of a dotted candidate. Accepted only when unambiguous.
    ShortName,
}

/// Which title match ended a root search early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TermHit {
    /// RustConn's own `RustConn: {id}` item, found in the first term's
    /// results — what the scoped search this replaces would have returned.
    Own,
    /// An item titled by a candidate (or, in a later term, RustConn's own
    /// item: a scoped search can miss it while a `store` of the same
    /// connection is still creating it).
    Title,
}

/// The root-search choice over each term's `bw --search` results, without the
/// CLI, so the order, the early exit and the pool are unit-tested.
///
/// Feed the results term by term, in candidate order, to [`Self::offer`]; stop
/// at its first `Some`. Otherwise hand [`Self::into_pool`] to
/// [`BitwardenBackend::pick_root_match`].
struct RootSelection<'a> {
    candidates: &'a [&'a str],
    own_name: &'a str,
    first_term: bool,
    pool: Vec<BitwardenItem>,
}

impl<'a> RootSelection<'a> {
    const fn new(candidates: &'a [&'a str], own_name: &'a str) -> Self {
        Self {
            candidates,
            own_name,
            first_term: true,
            pool: Vec::new(),
        }
    }

    /// Takes one term's results. Returns the item that ends the search: in
    /// the first term RustConn's own item before anything else (the scoped
    /// search used to run first and win), then the first exact title match.
    /// Otherwise pools the items, de-duplicated by id, and returns `None`.
    fn offer(&mut self, items: Vec<BitwardenItem>) -> Option<(BitwardenItem, TermHit)> {
        let first_term = std::mem::replace(&mut self.first_term, false);
        let own = first_term
            .then(|| items.iter().position(|item| item.name == self.own_name))
            .flatten()
            .map(|pos| (pos, TermHit::Own));
        let hit = own.or_else(|| {
            items
                .iter()
                .position(|item| {
                    item.name == self.own_name
                        || BitwardenBackend::root_search_matches(&item.name, self.candidates)
                })
                .map(|pos| (pos, TermHit::Title))
        });
        if let Some((pos, how)) = hit {
            return items.into_iter().nth(pos).map(|item| (item, how));
        }
        for item in items {
            if !self.pool.iter().any(|p| p.id == item.id) {
                self.pool.push(item);
            }
        }
        None
    }

    /// Everything the terms returned without a title match, first-seen order.
    fn into_pool(self) -> Vec<BitwardenItem> {
        self.pool
    }
}

/// Bitwarden item template for creation
#[derive(Debug, Serialize)]
struct BitwardenItemTemplate {
    #[serde(rename = "type")]
    item_type: u8,
    name: String,
    notes: Option<String>,
    login: BitwardenLoginTemplate,
    #[serde(rename = "folderId", skip_serializing_if = "Option::is_none")]
    folder_id: Option<String>,
}

/// Bitwarden login template for creation
#[derive(Debug, Serialize)]
struct BitwardenLoginTemplate {
    username: Option<String>,
    // Plain String: serde `Serialize` cannot derive over `Zeroizing`/`SecretString`
    // (the workspace `zeroize` has no `serde` feature). It is still wiped: the
    // `Drop` impl below zeroizes it, and `Drop` rules out moving it out first.
    // The serialized JSON is wrapped in `Zeroizing` at the call site and the
    // template is dropped right after.
    password: Option<String>,
    uris: Vec<BitwardenUri>,
}

impl Drop for BitwardenLoginTemplate {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.password.zeroize();
    }
}

/// Bitwarden URI structure
#[derive(Debug, Serialize)]
struct BitwardenUri {
    uri: String,
    #[serde(rename = "match")]
    match_type: Option<u8>,
}

/// Bitwarden folder structure
#[derive(Debug, Deserialize)]
struct BitwardenFolder {
    id: Option<String>,
    name: String,
}

/// Bitwarden status response
#[derive(Debug, Deserialize)]
pub struct BitwardenStatus {
    status: String,
    #[serde(rename = "userEmail")]
    #[expect(
        dead_code,
        reason = "Deserialized from `bw status` JSON but not used directly"
    )]
    user_email: Option<String>,
}

impl BitwardenBackend {
    /// Creates a new Bitwarden backend
    #[must_use]
    pub fn new() -> Self {
        Self {
            session_key: None,
            server_url: None,
            organization_id: None,
            folder_name: "RustConn".to_string(),
            read_only: false,
            root_search: false,
            bw_cmd: get_bw_cmd(),
        }
    }

    /// Creates a new Bitwarden backend with a session key
    #[must_use]
    pub fn with_session(session_key: SecretString) -> Self {
        Self {
            session_key: Some(session_key),
            server_url: None,
            organization_id: None,
            folder_name: "RustConn".to_string(),
            read_only: false,
            root_search: false,
            bw_cmd: get_bw_cmd(),
        }
    }

    /// Sets the server URL for self-hosted instances
    #[must_use]
    pub fn with_server_url(mut self, url: impl Into<String>) -> Self {
        self.server_url = Some(url.into());
        self
    }

    /// Sets the organization ID for org vault access
    #[must_use]
    pub fn with_organization(mut self, org_id: impl Into<String>) -> Self {
        self.organization_id = Some(org_id.into());
        self
    }

    /// Sets the folder name for storing RustConn entries
    #[must_use]
    pub fn with_folder_name(mut self, name: impl Into<String>) -> Self {
        self.folder_name = name.into();
        self
    }

    /// Puts the backend in read-only mode, where `store` and `delete` are
    /// refused with [`SecretError::ReadOnly`] and the vault is never mutated.
    #[must_use]
    pub const fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Widens credential reads to also match an entry titled by the bare
    /// connection id (not only `RustConn: {id}`), so an entry the user keeps
    /// outside the RustConn folder is found. Writes are unaffected.
    #[must_use]
    pub const fn with_root_search(mut self, root_search: bool) -> Self {
        self.root_search = root_search;
        self
    }

    /// Applies the persisted `bitwarden_read_only` / `bitwarden_root_search`
    /// toggles from secret settings.
    ///
    /// [`auto_unlock`] applies this to whatever backend it returns, so every
    /// unlocked backend honours the user's choice without each call site having
    /// to remember both builders.
    #[must_use]
    pub const fn with_settings_toggles(self, settings: &crate::config::SecretSettings) -> Self {
        self.with_read_only(settings.bitwarden_read_only)
            .with_root_search(settings.bitwarden_root_search)
    }

    /// Sets the session key
    pub fn set_session_key(&mut self, key: SecretString) {
        self.session_key = Some(key);
    }

    /// Clears the session key
    pub fn clear_session(&mut self) {
        self.session_key = None;
    }

    /// Builds command with session key if available.
    ///
    /// Checks the instance-level session key first, then falls back to the
    /// global in-process session store set by [`set_session_key`] (module-level).
    /// This ensures that backends created via `BitwardenBackend::new()` (which
    /// have no instance session key) still pick up a session established by
    /// [`auto_unlock`].
    ///
    /// The session key is passed via `BW_SESSION` environment variable instead
    /// of `--session` CLI argument to avoid exposure in `/proc/PID/cmdline`.
    fn build_command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.bw_cmd);
        cmd.env("PATH", crate::cli_download::get_extended_path());
        // See `bw_command`: a dropped timeout future does not signal the child
        // unless this is set.
        cmd.kill_on_drop(true);
        // --nointeraction prevents CLI from prompting for input or performing
        // implicit network operations that can hang in sandboxed environments.
        cmd.arg("--nointeraction");
        cmd.args(args);

        if let Some(ref session) = self.session_key {
            cmd.env("BW_SESSION", session.expose_secret());
        } else if let Some(ref global_session) = get_session_key() {
            cmd.env("BW_SESSION", global_session.expose_secret());
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd
    }

    /// Runs a bw command and returns stdout
    async fn run_command(&self, args: &[&str]) -> SecretResult<String> {
        self.run_command_inner(args, None).await
    }

    /// Runs a bw command with `input` written to its stdin, then closed.
    ///
    /// For `bw create item` / `bw edit item`: given no encoded-JSON argument,
    /// `bw` reads it from stdin (`CliUtils.readStdin` in the CLI). The item
    /// carries the plaintext password, and as an argument it sat in
    /// `/proc/<pid>/cmdline`, which every user on the machine can read.
    async fn run_command_with_stdin(&self, args: &[&str], input: &[u8]) -> SecretResult<String> {
        self.run_command_inner(args, Some(input)).await
    }

    /// [`Self::run_command`] and [`Self::run_command_with_stdin`].
    async fn run_command_inner(&self, args: &[&str], input: Option<&[u8]>) -> SecretResult<String> {
        // Only the verb reaches the log, never the whole argv.
        //
        // `bw create item` and `bw edit item` take the item as a base64 argument,
        // and that item contains the plaintext password (see `store`). Logging
        // `args` therefore wrote every saved password to the log at debug level,
        // wearing base64 as a disguise. The bulk credential transfer turned that
        // from an occasional line into one per stored password.
        let verb = args.iter().take(2).copied().collect::<Vec<_>>().join(" ");
        tracing::debug!(
            %verb,
            has_session = self.session_key.is_some(),
            has_global_session = get_session_key().is_some(),
            "Bitwarden run_command"
        );

        // Serve transport first, when preferred. On any serve-side failure the
        // call falls through to the one-shot spawn path below, so a wedged or
        // too-old CLI never blocks an operation — it only loses the speed-up.
        #[cfg(feature = "bitwarden-serve")]
        if is_serve_preferred()
            && let Some(result) = self.try_serve(args, input, &verb).await
        {
            return result;
        }

        // Timeout prevents hanging when Bitwarden CLI stalls on network
        // requests (e.g. expired session triggers a sync attempt that blocks).
        let mut cmd = self.build_command(args);
        let run = async {
            let Some(input) = input else {
                return cmd.output().await;
            };
            cmd.stdin(Stdio::piped());
            let mut child = cmd.spawn()?;
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                // A `bw` that exits before reading everything breaks the pipe;
                // its exit status and stderr below say why, so that is what
                // gets reported, not the write error.
                if let Err(e) = stdin.write_all(input).await {
                    tracing::debug!(%verb, error = %e, "Bitwarden run_command: stdin write failed");
                }
                // Dropping the handle closes the pipe: `bw` reads to EOF.
            }
            child.wait_with_output().await
        };
        let output = tokio::time::timeout(BW_INVOCATION_TIMEOUT, run)
            .await
            .map_err(|_| {
                tracing::warn!(%verb, "Bitwarden run_command: timed out after 30s");
                // The argv is not in this message for the same reason it is not in the
                // log line above: it carries the credential, and this error is shown to
                // the user and logged by `vault_ops`.
                SecretError::ConnectionFailed("bw command timed out after 30s".to_string())
            })?
            .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run bw: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::debug!(
                %verb,
                stderr = %stderr,
                "Bitwarden run_command: failed"
            );
            return Err(SecretError::ConnectionFailed(format!(
                "bw command failed: {stderr}"
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        tracing::debug!(
            %verb,
            output_len = stdout.len(),
            "Bitwarden run_command: success"
        );
        Ok(stdout)
    }

    /// Runs `args` over the persistent `bw serve` transport when it is preferred
    /// and healthy.
    ///
    /// Returns:
    /// - `None` — the caller must fall back to a one-shot spawn: the verb is not
    ///   one the transport maps, the transport is disabled for this session, a
    ///   serve child could not be started, or the operation failed even after a
    ///   one-shot respawn. The fallback keeps the operation working; it only
    ///   loses the speed-up.
    /// - `Some(Ok(stdout))` / `Some(Err(_))` — the transport handled the verb
    ///   and this is its result, shaped like the spawn path's stdout.
    ///
    /// The child is started lazily with the current session key so it comes up
    /// unlocked. One serve process serves the whole backend; a transport error
    /// triggers exactly one respawn-and-retry before the session latches back to
    /// spawns, so a wedged CLI is not retried on every call
    /// (`bw serve` can race under load — bitwarden/clients#20951).
    #[cfg(feature = "bitwarden-serve")]
    async fn try_serve(
        &self,
        args: &[&str],
        input: Option<&[u8]>,
        verb: &str,
    ) -> Option<SecretResult<String>> {
        // Don't even reserve the lock for a verb the transport cannot map.
        if !super::bitwarden_serve::is_supported_verb(args) {
            return None;
        }

        let result = {
            let mut manager = BW_SERVE.lock().await;
            self.try_serve_locked(&mut manager, args, input, verb).await
        };
        // The slot is released: stop the child if a screen lock or suspend found
        // it busy with this op (see `stop_serve_on_lock`).
        if finish_pending_serve_stop(&BW_SERVE, &BW_SERVE_STOP_PENDING).await {
            tracing::info!(%verb, "Bitwarden serve: stopped after the op, on screen lock or suspend");
        }
        result
    }

    /// The body of [`Self::try_serve`], run while it holds the serve slot.
    #[cfg(feature = "bitwarden-serve")]
    async fn try_serve_locked(
        &self,
        manager: &mut ServeManager,
        args: &[&str],
        input: Option<&[u8]>,
        verb: &str,
    ) -> Option<SecretResult<String>> {
        if manager.disabled {
            return None;
        }

        let session = self.session_key.clone().or_else(get_session_key);
        let bw_cmd = self.bw_cmd.clone();

        // If the session changed out of band since the child started (a
        // spawn-path unlock, or a clear), the running child is still on the old
        // lock state: drop it so the lazy start below brings up a fresh one with
        // the current session. Clearing the flag first means a concurrent change
        // still re-trips it.
        if BW_SERVE_STALE.swap(false, std::sync::atomic::Ordering::Relaxed)
            && let Some(stale) = manager.process.take()
        {
            tracing::debug!(%verb, "Bitwarden serve: session changed, respawning with the current session");
            stale.shutdown().await;
        }

        // Ensure a child exists (lazy start with the current session).
        if manager.process.is_none() {
            match super::bitwarden_serve::ServeProcess::spawn(&bw_cmd, session.as_ref()).await {
                Ok(process) => manager.process = Some(process),
                Err(e) => {
                    tracing::warn!(%verb, error = %e, "Bitwarden serve: spawn failed, falling back to spawn-per-call");
                    manager.disabled = true;
                    return None;
                }
            }
        }

        // First attempt.
        let first = {
            let process = manager.process.as_ref()?;
            process.run_cli(args, input).await
        };
        match first {
            Ok(out) => {
                tracing::debug!(%verb, transport = "serve", output_len = out.len(), "Bitwarden serve: ok");
                return Some(Ok(out));
            }
            Err(e) => {
                tracing::warn!(%verb, error = %e, "Bitwarden serve: request failed, respawning once");
            }
        }

        // One-shot respawn, then retry the same op.
        if let Some(dead) = manager.process.take() {
            dead.shutdown().await;
        }
        match super::bitwarden_serve::ServeProcess::spawn(&bw_cmd, session.as_ref()).await {
            Ok(process) => {
                let retry = process.run_cli(args, input).await;
                manager.process = Some(process);
                match retry {
                    Ok(out) => Some(Ok(out)),
                    Err(e) => {
                        tracing::warn!(%verb, error = %e, "Bitwarden serve: retry failed, falling back to spawn for the rest of the session");
                        manager.disabled = true;
                        None
                    }
                }
            }
            Err(e) => {
                tracing::warn!(%verb, error = %e, "Bitwarden serve: respawn failed, falling back to spawn");
                manager.disabled = true;
                None
            }
        }
    }

    /// Gets the vault status
    ///
    /// # Errors
    /// Returns `SecretError` if the command fails or output cannot be parsed
    pub async fn get_status(&self) -> SecretResult<BitwardenStatus> {
        let output = self.run_command(&["status"]).await?;
        serde_json::from_str(&output)
            .map_err(|e| SecretError::ConnectionFailed(format!("Failed to parse status: {e}")))
    }

    /// Checks if the vault is unlocked
    pub async fn is_unlocked(&self) -> bool {
        let unlocked = self
            .get_status()
            .await
            .map(|s| s.status == "unlocked")
            .unwrap_or(false);
        if unlocked {
            mark_verified();
        }
        unlocked
    }

    /// Fast check: returns `true` if the vault was recently verified as
    /// unlocked and a session key is available, without spawning `bw status`.
    /// Falls back to the full [`Self::is_unlocked`] check when the cached result
    /// has expired.
    ///
    /// # Bitwarden CLI v2026.4+ compatibility
    ///
    /// `bw status` ignores the `BW_SESSION` environment variable and always
    /// reports "locked" unless the vault was unlocked in the same process.
    /// When a session key is present, we trust it and skip the unreliable
    /// `bw status` check. If the session is actually expired, the next
    /// data command (`bw list items`) will fail and trigger a re-unlock.
    pub async fn is_unlocked_fast(&self) -> bool {
        let has_session = self.session_key.is_some() || get_session_key().is_some();
        if has_session {
            // Trust the session key — bw status is unreliable with BW_SESSION
            if is_recently_verified() {
                return true;
            }
            // Session key exists but not recently verified — re-mark and trust.
            // The alternative (calling bw status) always returns "locked" on
            // Bitwarden CLI v2026.4+ even with a valid session key.
            mark_verified();
            return true;
        }
        self.is_unlocked().await
    }

    /// Syncs the vault with the server.
    ///
    /// A success restarts the ten-minute window in which [`auto_unlock`] skips
    /// the sync when it re-trusts a stored session.
    ///
    /// # Errors
    /// Returns `SecretError` if the sync command fails
    pub async fn sync(&self) -> SecretResult<()> {
        self.run_command(&["sync"]).await?;
        mark_synced();
        Ok(())
    }

    /// Syncs the vault only if one is due, serialized process-wide.
    ///
    /// Takes [`BW_SYNC`] and re-checks [`sync_due`] under it, so two callers that
    /// both saw a sync as due — a connect racing the startup background warm-up —
    /// run exactly one `bw sync` between them: the first syncs, the second waits
    /// on the lock and then finds nothing to do. A no-op when a sync ran within
    /// [`SYNC_TTL_SECS`]. Use this on the optimisation paths (retrieve, store,
    /// startup warm-up); use [`sync`](Self::sync) when a sync must happen
    /// regardless of the TTL.
    ///
    /// # Errors
    /// Returns `SecretError` if the sync command fails.
    pub async fn sync_if_due(&self) -> SecretResult<()> {
        let _one_at_a_time = BW_SYNC.lock().await;
        if !sync_due() {
            tracing::debug!("Bitwarden: skipping sync (another caller just synced)");
            return Ok(());
        }
        self.run_command(&["sync"]).await?;
        mark_synced();
        Ok(())
    }

    /// Gets or creates the RustConn folder
    async fn get_or_create_folder(&self) -> SecretResult<Option<String>> {
        // List folders
        let output = self.run_command(&["list", "folders"]).await?;
        let folders: Vec<BitwardenFolder> = serde_json::from_str(&output)
            .map_err(|e| SecretError::ConnectionFailed(format!("Failed to parse folders: {e}")))?;

        // Find existing folder (skip folders with null id)
        if let Some(folder) = folders
            .iter()
            .find(|f| f.name == self.folder_name && f.id.is_some())
        {
            return Ok(folder.id.clone());
        }

        // Create folder
        let folder_json = serde_json::json!({ "name": self.folder_name });
        let encoded = base64_encode(folder_json.to_string().as_bytes());

        let output = self.run_command(&["create", "folder", &encoded]).await?;
        let folder: BitwardenFolder = serde_json::from_str(&output)
            .map_err(|e| SecretError::StoreFailed(format!("Failed to create folder: {e}")))?;

        Ok(folder.id)
    }

    /// Generates a unique name for a connection entry
    fn entry_name(connection_id: &str) -> String {
        format!("RustConn: {connection_id}")
    }

    /// Generates a URI for a connection (used for searching)
    fn connection_uri(connection_id: &str) -> String {
        format!("rustconn://{connection_id}")
    }

    /// Finds RustConn's own item for `connection_id`: the one titled
    /// `RustConn: {id}`, never a root-search match.
    ///
    /// `store` and `delete` use this, and must. They used to go through the
    /// root-widened [`Self::find_item_with_aliases`], so with root search on a
    /// write could land on an entry the user keeps outside RustConn — `store`
    /// overwrote and renamed it, `delete` removed it. Root search widens reads
    /// only.
    async fn find_item(&self, connection_id: &str) -> SecretResult<Option<BitwardenItem>> {
        let search_term = Self::entry_name(connection_id);
        tracing::debug!(
            search_term = %search_term,
            connection_id = %connection_id,
            "Bitwarden find_item: searching vault"
        );

        let output = self
            .run_command(&["list", "items", "--search", &search_term])
            .await?;

        // Position, not the serde `Display`. `bw list items` includes
        // `login.password`, and serde reports a type mismatch by quoting the value
        // it choked on — a numeric password comes back as `invalid type: integer
        // \`1234\`, expected a string`, which `vault_ops` then logs and shows.
        let items: Vec<BitwardenItem> = serde_json::from_str(&output).map_err(|e| {
            SecretError::RetrieveFailed(format!(
                "Failed to parse items: {} error at line {}, column {}",
                serde_error_kind(&e),
                e.line(),
                e.column()
            ))
        })?;

        tracing::debug!(
            items_count = items.len(),
            search_term = %search_term,
            "Bitwarden find_item: parsed results"
        );

        let result = items.into_iter().find(|item| item.name == search_term);
        if result.is_none() {
            tracing::debug!(
                search_term = %search_term,
                "Bitwarden find_item: no exact match found"
            );
        }
        Ok(result)
    }

    /// The bare candidate titles a root-search fallback may match, in priority
    /// order: the connection id first (back-compat), then the supplied aliases
    /// (connection host/name — issue #353). De-duplicated case-insensitively
    /// with blanks dropped. When `aliases` is empty this is just `[id]`, so the
    /// fallback behaves byte-identically to the id-only form it replaced.
    fn bare_candidates<'c>(connection_id: &'c str, aliases: &[&'c str]) -> Vec<&'c str> {
        let mut candidates: Vec<&str> = Vec::with_capacity(2 + aliases.len());
        let mut push = |cand: &'c str| {
            let cand = cand.trim();
            if !cand.is_empty() && !candidates.iter().any(|c| c.eq_ignore_ascii_case(cand)) {
                candidates.push(cand);
            }
        };
        push(connection_id);
        for alias in aliases {
            push(alias);
            // A host given as a URL (`https://sso.example.com/start/#/`, a Web
            // or Zero Trust connection) is not what anyone titles a vault entry,
            // and `bw --search` with the whole URL finds nothing. Its host is
            // the useful candidate, right after the URL itself.
            if alias.contains("://")
                && let Some(host) = Self::uri_host(alias)
            {
                push(host);
            }
        }
        candidates
    }

    /// The root-search match predicate: does `item_name` equal ANY of the bare
    /// `candidates` (the connection id, host or name) under case-insensitive,
    /// whitespace-trimmed EXACT matching ([`super::backend::root_alias_eq`])?
    ///
    /// Pure (no CLI, no `self`) so the issue #353 widening is unit-tested
    /// deterministically: it must accept the UUID, the name and the host, do so
    /// case-insensitively, and reject a mere substring.
    fn root_search_matches(item_name: &str, candidates: &[&str]) -> bool {
        candidates
            .iter()
            .any(|cand| super::backend::root_alias_eq(item_name, cand))
    }

    /// The host of a login URI: `rdp://user@hanfile03.corp:3389/x` ->
    /// `hanfile03.corp`. A bare `hanfile03` (no scheme) is its own host.
    fn uri_host(uri: &str) -> Option<&str> {
        let uri = uri.trim();
        let rest = uri.split_once("://").map_or(uri, |(_, r)| r);
        let authority = rest.split(['/', '?', '#']).next()?;
        let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let host = if let Some(v6) = host_port.strip_prefix('[') {
            v6.split(']').next()?
        } else {
            host_port.split(':').next()?
        };
        (!host.is_empty()).then_some(host)
    }

    /// The first DNS label of `name`, when `name` is a dotted host name (not
    /// an IP address): `hanfile03.corp.example` -> `hanfile03`.
    fn first_label(name: &str) -> Option<&str> {
        let name = name.trim();
        if name.parse::<std::net::IpAddr>().is_ok() {
            return None;
        }
        let (label, rest) = name.split_once('.')?;
        (!label.is_empty() && !rest.is_empty()).then_some(label)
    }

    /// Classifies how `item` matches the root-search `candidates`, if at all.
    ///
    /// Pure, so every tier is unit-tested without the `bw` CLI.
    fn root_match(item: &BitwardenItem, candidates: &[&str]) -> Option<RootMatch> {
        use super::backend::root_alias_eq;

        if Self::root_search_matches(&item.name, candidates) {
            return Some(RootMatch::Title);
        }
        let hosts: Vec<&str> = item
            .login
            .iter()
            .flat_map(|login| login.uris.iter().flatten())
            .filter_map(|u| u.uri.as_deref())
            .filter_map(Self::uri_host)
            .collect();
        if hosts
            .iter()
            .any(|host| candidates.iter().any(|cand| root_alias_eq(host, cand)))
        {
            return Some(RootMatch::UriHost);
        }
        let short_match = candidates.iter().any(|cand| {
            if cand.contains('.') {
                // A dotted candidate (an FQDN host): the title is its short name.
                Self::first_label(cand).is_some_and(|label| root_alias_eq(&item.name, label))
            } else {
                // A dotless candidate: the title or a URI host is its FQDN.
                std::iter::once(item.name.as_str())
                    .chain(hosts.iter().copied())
                    .filter_map(Self::first_label)
                    .any(|label| root_alias_eq(label, cand))
            }
        });
        short_match.then_some(RootMatch::ShortName)
    }

    /// Picks the item root search should use from everything the candidate
    /// searches returned: the strongest tier wins, and a tier weaker than
    /// [`RootMatch::Title`] must be unambiguous — two items that both only
    /// match by URI or short name would hand one of them a password chosen
    /// by list order, so neither is used.
    fn pick_root_match(
        items: &[BitwardenItem],
        candidates: &[&str],
    ) -> Result<Option<(usize, RootMatch)>, RootMatch> {
        let matches: Vec<(usize, RootMatch)> = items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| Self::root_match(item, candidates).map(|m| (i, m)))
            .collect();
        let Some(best) = matches.iter().map(|(_, m)| *m).min() else {
            return Ok(None);
        };
        let mut at_best = matches.iter().filter(|(_, m)| *m == best);
        let first = at_best.next().copied();
        if best != RootMatch::Title && at_best.next().is_some() {
            return Err(best);
        }
        Ok(first)
    }

    /// Alias-aware form of [`Self::find_item`] (issue #353). The scoped
    /// `RustConn: {id}` match still wins first. When root-search is on, the
    /// whole-vault fallback accepts an entry titled by the bare connection id
    /// OR any alias (connection host/name), matched case-insensitively and
    /// exactly via [`Self::root_search_matches`]. When no title matches, an
    /// item whose login URI host is a candidate, or whose title/URI host is
    /// the candidate's FQDN or short name, is used — but only if exactly one
    /// item matches that way ([`Self::pick_root_match`]). Reads only — stores
    /// still use the `RustConn: {id}` name and RustConn folder.
    async fn find_item_with_aliases(
        &self,
        connection_id: &str,
        aliases: &[&str],
    ) -> SecretResult<Option<BitwardenItem>> {
        // Root-search off: the scoped search is the whole lookup, one `bw` call.
        if !self.root_search {
            return self.find_item(connection_id).await;
        }

        // Root-search fallback: an entry the user keeps outside RustConn's
        // naming convention is titled by the bare connection id — or, for issue
        // #353, by the connection's host or name. Search for each candidate and
        // match it exactly via `root_search_matches`. Reads only — stores still
        // use the `RustConn: {id}` name and RustConn folder.
        //
        // `bw --search` is a substring search over title, username, URIs and
        // notes, so it returns items whose title is not a candidate. An exact
        // title match from any search wins at once (the pre-#353 behaviour);
        // everything else is pooled, and only after every search came back
        // empty-handed is the pool tried for a URI-host or short-name match.
        let candidates = Self::bare_candidates(connection_id, aliases);
        let own_name = Self::entry_name(connection_id);
        // The first candidate is the connection id, and a search for it returns
        // everything the scoped `RustConn: {id}` search does (each word of the
        // id is in that name), so `RootSelection` looks for the own item in its
        // results first and the scoped call is skipped — one `bw` call fewer
        // (seconds each). Only an id that `bare_candidates` trimmed or dropped
        // breaks that, and keeps the scoped call.
        if candidates.first() != Some(&connection_id)
            && let Some(item) = self.find_item(connection_id).await?
        {
            return Ok(Some(item));
        }
        let mut selection = RootSelection::new(&candidates, &own_name);
        for term in &candidates {
            let bare_output = self
                .run_command(&["list", "items", "--search", term])
                .await?;
            let bare_items: Vec<BitwardenItem> =
                serde_json::from_str(&bare_output).map_err(|e| {
                    SecretError::RetrieveFailed(format!(
                        "Failed to parse items: {} error at line {}, column {}",
                        serde_error_kind(&e),
                        e.line(),
                        e.column()
                    ))
                })?;
            // Titles and URI hosts only — never a password or a username.
            // Without these, a "no item found" after a search that returned
            // items cannot be told apart from an empty vault (issue #353).
            tracing::debug!(
                %term,
                items_count = bare_items.len(),
                titles = ?bare_items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
                "Bitwarden root search: search returned"
            );
            match selection.offer(bare_items) {
                Some((item, TermHit::Own)) => return Ok(Some(item)),
                Some((item, TermHit::Title)) => {
                    Self::log_root_match(&item, term, RootMatch::Title, connection_id);
                    return Ok(Some(item));
                }
                None => {}
            }
        }

        let pool = selection.into_pool();
        match Self::pick_root_match(&pool, &candidates) {
            Ok(Some((pos, how))) => {
                let item = pool.into_iter().nth(pos);
                if let Some(item) = &item {
                    Self::log_root_match(item, "-", how, connection_id);
                }
                Ok(item)
            }
            Ok(None) => {
                tracing::debug!(
                    ?candidates,
                    rejected = ?pool.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
                    "Bitwarden root search: no title, URI host or short name matched"
                );
                Ok(None)
            }
            Err(how) => {
                tracing::warn!(
                    ?how,
                    ?candidates,
                    ambiguous = ?pool
                        .iter()
                        .filter(|i| Self::root_match(i, &candidates) == Some(how))
                        .map(|i| i.name.as_str())
                        .collect::<Vec<_>>(),
                    "Bitwarden root search: several items match equally well; using none"
                );
                Ok(None)
            }
        }
    }

    /// A root-search hit hands a password RustConn did not store to a
    /// connection; say which item and why, so a wrong match can be traced.
    fn log_root_match(item: &BitwardenItem, term: &str, how: RootMatch, connection_id: &str) {
        tracing::info!(
            matched = %item.name,
            via = %term,
            ?how,
            %connection_id,
            "Bitwarden root search matched an item outside RustConn"
        );
    }

    /// Shared retrieve path for both [`SecretBackend::retrieve`] and the
    /// alias-aware [`SecretBackend::retrieve_identity`]. `aliases` are the
    /// connection host/name that root-search may also match (issue #353); empty
    /// for the plain path, in which case behaviour is byte-identical to the old
    /// `retrieve`.
    async fn retrieve_with_aliases(
        &self,
        connection_id: &str,
        aliases: &[&str],
    ) -> SecretResult<Option<Credentials>> {
        tracing::debug!(
            connection_id = %connection_id,
            "Bitwarden retrieve: starting"
        );

        // Fast unlock check — skips `bw status` if recently verified
        if !self.is_unlocked_fast().await {
            return Err(SecretError::BackendUnavailable(
                "Bitwarden vault is locked. Please unlock with 'bw unlock'".to_string(),
            ));
        }

        // `bw sync` runs here at most once per `SYNC_TTL_SECS`, gated by
        // `sync_due()` — NOT on every lookup. `auto_unlock` no longer syncs on
        // the unlock path (it cost ~6 s on the critical path to a ready vault,
        // #353), so the freshness it used to guarantee is picked up here on the
        // first retrieve after an unlock: `BW_LAST_SYNCED` is still unset/stale,
        // so this one call syncs, `mark_synced()` restarts the window, and every
        // reconnect and batch lookup within it skips the ~0.5-2 s round-trip.
        // A refused sign-in (#351) clears the mark via `sync_on_next_unlock()`,
        // so the retry still reads a fresh item.
        let _ = self.sync_if_due().await;

        let item = if let Some(item) = self.find_item_with_aliases(connection_id, aliases).await? {
            tracing::debug!(
                item_id = %item.id,
                item_name = %item.name,
                "Bitwarden retrieve: item found"
            );
            item
        } else {
            tracing::debug!(
                connection_id = %connection_id,
                "Bitwarden retrieve: no item found"
            );
            return Ok(None);
        };

        let domain = Self::item_domain(&item, connection_id);
        let login = match item.login {
            Some(login) => login,
            None => return Ok(None),
        };

        Ok(Some(Credentials {
            username: login.username,
            password: login.password,
            domain,
        }))
    }

    /// The Windows domain an item supplies.
    ///
    /// A custom field named `domain` (any case) is read from any item. The
    /// notes field is read only from RustConn's own `RustConn: {id}` item,
    /// because that is where `store` has always written the domain. An item
    /// found by root search is the user's own entry, whose notes are free
    /// text: handing them to RDP as `/d:` would put prose on the command line.
    /// The value is checked by [`super::sanitize_domain`] later either way.
    fn item_domain(item: &BitwardenItem, connection_id: &str) -> Option<String> {
        let from_field = item
            .fields
            .iter()
            .flatten()
            .find(|f| {
                f.name
                    .as_deref()
                    .is_some_and(|n| n.trim().eq_ignore_ascii_case("domain"))
            })
            .and_then(|f| f.value.clone())
            .filter(|v| !v.trim().is_empty());
        from_field.or_else(|| {
            (item.name == Self::entry_name(connection_id))
                .then(|| item.notes.clone())
                .flatten()
        })
    }

    /// Finds an item by exact vault entry name (without `RustConn:` prefix)
    /// and returns its password.
    ///
    /// This allows reusing existing Bitwarden entries that were not created
    /// by RustConn. The search matches the item name exactly as stored.
    ///
    /// # Errors
    /// Returns `SecretError` if the vault is locked or CLI fails.
    pub async fn find_password_by_exact_name(
        &self,
        entry_name: &str,
    ) -> SecretResult<Option<SecretString>> {
        tracing::debug!(
            entry_name = %entry_name,
            "Bitwarden find_password_by_exact_name: searching vault"
        );

        // One-shot sync — the same freshness the unlock path used to guarantee,
        // now paid on the first lookup after an unlock. See the longer note in
        // `retrieve_with_aliases`.
        let _ = self.sync_if_due().await;

        let output = self
            .run_command(&["list", "items", "--search", entry_name])
            .await?;

        // Position, not the serde `Display`. `bw list items` includes
        // `login.password`, and serde reports a type mismatch by quoting the value
        // it choked on — a numeric password comes back as `invalid type: integer
        // \`1234\`, expected a string`, which `vault_ops` then logs and shows.
        let items: Vec<BitwardenItem> = serde_json::from_str(&output).map_err(|e| {
            SecretError::RetrieveFailed(format!(
                "Failed to parse items: {} error at line {}, column {}",
                serde_error_kind(&e),
                e.line(),
                e.column()
            ))
        })?;

        tracing::debug!(
            items_count = items.len(),
            entry_name = %entry_name,
            "Bitwarden find_password_by_exact_name: parsed results"
        );

        // Find exact match by name (case-sensitive)
        let result = items.into_iter().find(|item| item.name == entry_name);

        if result.is_none() {
            tracing::debug!(
                entry_name = %entry_name,
                "Bitwarden find_password_by_exact_name: no exact match found"
            );
        }

        Ok(result.and_then(|item| item.login).and_then(|l| l.password))
    }
}

impl Default for BitwardenBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SecretBackend for BitwardenBackend {
    async fn store(&self, connection_id: &str, credentials: &Credentials) -> SecretResult<()> {
        self.ensure_writable()?;
        let entry_name = Self::entry_name(connection_id);
        tracing::debug!(
            connection_id = %connection_id,
            entry_name = %entry_name,
            has_session = self.session_key.is_some(),
            has_global_session = get_session_key().is_some(),
            "Bitwarden store: starting"
        );

        // Check if vault is unlocked
        if !self.is_unlocked_fast().await {
            tracing::error!("Bitwarden store: vault is locked");
            return Err(SecretError::BackendUnavailable(
                "Bitwarden vault is locked. Please unlock with 'bw unlock'".to_string(),
            ));
        }

        // One-shot sync before the find-then-edit-or-create below, so a fresh
        // unlock sees items added in another client and edits them instead of
        // creating a duplicate. See the note in `retrieve_with_aliases`.
        let _ = self.sync_if_due().await;

        // Get or create folder
        let folder_id = self.get_or_create_folder().await?;

        // Check if item already exists
        if let Some(existing) = self.find_item(connection_id).await? {
            // Update existing item
            let item_template = BitwardenItemTemplate {
                item_type: 1, // Login
                name: Self::entry_name(connection_id),
                notes: credentials.domain.clone(),
                login: BitwardenLoginTemplate {
                    username: credentials.username.clone(),
                    password: credentials.expose_password().map(String::from),
                    uris: vec![BitwardenUri {
                        uri: Self::connection_uri(connection_id),
                        match_type: Some(3), // Exact match
                    }],
                },
                folder_id,
            };

            // Wrap the serialized item in Zeroizing: the JSON buffer holds the
            // plaintext password and is wiped on drop instead of lingering.
            let json = zeroize::Zeroizing::new(
                serde_json::to_string(&item_template)
                    .map_err(|e| SecretError::StoreFailed(format!("Failed to serialize: {e}")))?,
            );
            let encoded = base64_encode_secret(json.as_bytes());

            // The item goes to `bw` on stdin, never as an argument: it holds
            // the password, and argv is world-readable in /proc/<pid>/cmdline.
            let edit_result = self
                .run_command_with_stdin(&["edit", "item", &existing.id], encoded.as_bytes())
                .await;

            if let Err(ref e) = edit_result {
                let err_msg = format!("{e}");
                if err_msg.contains("out of date") || err_msg.contains("out-of-date") {
                    tracing::info!("Bitwarden cipher out of date, syncing and retrying...");
                    // `sync()`, not a bare `run_command`: it records the sync,
                    // so the next `auto_unlock` does not repeat it.
                    let _ = self.sync().await;

                    // Re-fetch the item to get updated revision
                    if let Some(refreshed) = self.find_item(connection_id).await? {
                        self.run_command_with_stdin(
                            &["edit", "item", &refreshed.id],
                            encoded.as_bytes(),
                        )
                        .await?;
                    } else {
                        // Item was deleted remotely — create instead
                        self.run_command_with_stdin(&["create", "item"], encoded.as_bytes())
                            .await?;
                    }
                } else {
                    edit_result?;
                }
            }
        } else {
            // Create new item
            let item_template = BitwardenItemTemplate {
                item_type: 1, // Login
                name: Self::entry_name(connection_id),
                notes: credentials.domain.clone(),
                login: BitwardenLoginTemplate {
                    username: credentials.username.clone(),
                    password: credentials.expose_password().map(String::from),
                    uris: vec![BitwardenUri {
                        uri: Self::connection_uri(connection_id),
                        match_type: Some(3), // Exact match
                    }],
                },
                folder_id,
            };

            // Wrap the serialized item in Zeroizing: the JSON buffer holds the
            // plaintext password and is wiped on drop instead of lingering.
            let json = zeroize::Zeroizing::new(
                serde_json::to_string(&item_template)
                    .map_err(|e| SecretError::StoreFailed(format!("Failed to serialize: {e}")))?,
            );
            let encoded = base64_encode_secret(json.as_bytes());

            // On stdin, not argv — see the edit branch above.
            self.run_command_with_stdin(&["create", "item"], encoded.as_bytes())
                .await?;
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
        // Check if vault is unlocked
        if !self.is_unlocked_fast().await {
            return Err(SecretError::BackendUnavailable(
                "Bitwarden vault is locked. Please unlock with 'bw unlock'".to_string(),
            ));
        }

        let item = match self.find_item(connection_id).await? {
            Some(item) => item,
            None => return Ok(()), // Already deleted
        };

        self.run_command(&["delete", "item", &item.id]).await?;
        Ok(())
    }

    async fn is_available(&self) -> bool {
        // A session key exists only after a successful unlock, which needs both
        // an installed `bw` and a logged-in account — exactly what the two
        // probes below establish. Skipping them saves two `bw` spawns (seconds
        // each) on every `SecretManager` read, store and delete. An expired
        // session still surfaces, on the data command that follows.
        if self.session_key.is_some() || has_session_key() {
            return true;
        }

        // Check if bw CLI is installed
        let installed = bw_command(&self.bw_cmd)
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !installed {
            return false;
        }

        // Check if logged in (status != "unauthenticated")
        self.get_status()
            .await
            .map(|s| s.status != "unauthenticated")
            .unwrap_or(false)
    }

    fn backend_id(&self) -> &'static str {
        "bitwarden"
    }

    fn display_name(&self) -> &'static str {
        "Bitwarden"
    }

    fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn searches_from_root(&self) -> bool {
        self.root_search
    }
}

/// Bitwarden version information
#[derive(Debug, Clone)]
pub struct BitwardenVersion {
    /// CLI version string
    pub version: String,
    /// Whether CLI is installed
    pub installed: bool,
}

/// Gets Bitwarden CLI version
pub async fn get_bitwarden_version() -> Option<BitwardenVersion> {
    let bw_cmd = get_bw_cmd();
    let output = bw_command(&bw_cmd).arg("--version").output().await.ok()?;

    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Some(BitwardenVersion {
            version,
            installed: true,
        })
    } else {
        None
    }
}

/// Unlocks Bitwarden vault with master password
///
/// Uses `std::process::Command` (blocking) via `spawn_blocking` because the
/// new Rust-based Bitwarden CLI (v2026+) has compatibility issues with
/// `tokio::process::Command` for the `unlock` subcommand.
///
/// Tries three strategies in order:
/// 1. `--passwordenv BW_PASSWORD --raw` (returns session key directly)
/// 2. `--passwordenv BW_PASSWORD` without `--raw` (parses session key from verbose output)
/// 3. Stdin password pipe without `--raw` (for older CLI versions)
///
/// # Errors
/// Returns `SecretError` if the unlock command fails or password is incorrect
pub async fn unlock_vault(password: &SecretString) -> SecretResult<SecretString> {
    let pw = zeroize::Zeroizing::new(password.expose_secret().to_string());

    tokio::task::spawn_blocking(move || unlock_vault_sync(&pw))
        .await
        .map_err(|e| SecretError::ConnectionFailed(format!("Unlock task panicked: {e}")))?
}

/// Unlocks the Bitwarden vault from a thread that is already blocking.
///
/// Same three strategies and the same [`BW_INVOCATION_TIMEOUT`] as
/// [`unlock_vault`], without the `spawn_blocking` hop — for callers that are
/// themselves inside a blocking worker and would otherwise have to nest a
/// runtime. The GUI's Secrets page has four such call sites, each of which used
/// to build its own `bw unlock`: none passed the extended `PATH` a sandboxed `bw`
/// needs, none passed `--nointeraction`, none had a deadline, and only two of the
/// four implemented the verbose fallback (issue
/// [#312](https://github.com/totoshko88/RustConn/issues/312)).
///
/// Waits for any [`auto_unlock`] in progress, then holds the same process-wide
/// unlock lock for the whole unlock, so this never runs a `bw unlock` alongside
/// another one.
///
/// The wait has no deadline of its own. It is bounded by the holder: every
/// `auto_unlock` caller runs it under a 30 s or 45 s timeout, and each `bw`
/// child here under [`BW_INVOCATION_TIMEOUT`]. Two consequences are accepted:
/// a connect's `auto_unlock` that queues behind this unlock (up to three
/// strategies × 30 s) can hit its own timeout while waiting, and an
/// `auto_unlock` dropped by its caller's timeout releases the lock while the
/// `bw unlock` it spawned may still be finishing. Either way the callers see an
/// ordinary timeout or a second unlock, never a hang.
///
/// # Errors
/// Returns `SecretError::ConnectionFailed` if every strategy fails, the password
/// is wrong, or an invocation exceeded its budget.
///
/// # Panics
/// Panics if called from within an asynchronous execution context (a tokio
/// runtime), where [`tokio::sync::Mutex::blocking_lock`] refuses to block.
/// Call it from a blocking worker (`gio::spawn_blocking`,
/// `tokio::task::spawn_blocking`); async code uses [`unlock_vault`] or
/// [`auto_unlock`].
pub fn unlock_vault_blocking(password: &SecretString) -> SecretResult<SecretString> {
    with_unlock_lock_blocking(|| {
        // Zeroizing, not a bare String: this is the master password in the clear
        // and it must not be left in freed heap when the call returns.
        let pw = zeroize::Zeroizing::new(password.expose_secret().to_string());
        unlock_vault_sync(&pw)
    })
}

/// Runs `f` while holding [`BW_UNLOCK`], taken by blocking the current thread.
///
/// Separate from [`unlock_vault_blocking`] so the serialisation is testable
/// without a `bw` binary. [`unlock_vault_sync`] must not take the lock itself:
/// [`auto_unlock`] already holds it while [`unlock_vault`] runs that function.
///
/// # Panics
/// Panics inside an asynchronous execution context, like `blocking_lock`.
fn with_unlock_lock_blocking<T>(f: impl FnOnce() -> T) -> T {
    let _one_at_a_time = BW_UNLOCK.blocking_lock();
    f()
}

/// Strips anything that looks like a session key out of `bw`'s stderr.
///
/// Unlock stderr is logged at debug level and is also formatted into the
/// `SecretError` that the Secrets page renders, so it reaches two places a
/// credential must not. The session key is the vault's bearer token — it is what
/// [`get_session_key`] hands to every later `bw` call — and nothing here can
/// promise a given `bw` build keeps its `export BW_SESSION="…"` banner on stdout,
/// where [`extract_session_key`] expects it. A build that writes it to stderr
/// instead would leak the key through the diagnostic path on the one code path
/// that never parses it.
///
/// Redacts by line rather than dropping the whole message, because the useful
/// part of unlock stderr is the reason — "Invalid master password", "not logged
/// in" — and both callers match on it.
fn redact_session_keys(stderr: &str) -> String {
    stderr
        .lines()
        .map(|line| {
            if line.contains("BW_SESSION=") {
                "[redacted: line contained a session key]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Runs one bounded `bw unlock` attempt, with the master password in the child's
/// environment.
///
/// The password travels as `BW_PASSWORD` and is named to `bw` with
/// `--passwordenv`, never as an argument: argv is world-readable through
/// `/proc/PID/cmdline`.
///
/// `--nointeraction` is passed here but deliberately *not* by the stdin strategy
/// in [`unlock_vault_sync`], which needs `bw` to read the interactive password
/// prompt it would otherwise suppress.
///
/// # Errors
/// Returns `SecretError::ConnectionFailed` if the child cannot be spawned or
/// waited on, or if it exceeded [`BW_INVOCATION_TIMEOUT`].
fn unlock_attempt(bw_cmd: &str, password: &str, raw: bool) -> SecretResult<std::process::Output> {
    let mut cmd = std::process::Command::new(bw_cmd);
    cmd.env("PATH", crate::cli_download::get_extended_path());
    cmd.arg("--nointeraction");
    cmd.args(["unlock", "--passwordenv", "BW_PASSWORD"]);
    if raw {
        cmd.arg("--raw");
    }
    cmd.env("BW_PASSWORD", password)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let child = cmd
        .spawn()
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run bw unlock: {e}")))?;

    // "bw unlock" and not the argv: `what` is logged on timeout, and while this
    // particular argv holds only a variable *name*, keeping the habit is what
    // stops the next caller interpolating a value.
    crate::proc::wait_bounded(child, BW_INVOCATION_TIMEOUT, "bw unlock")
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to wait for bw unlock: {e}")))?
        .output()
        .ok_or_else(|| {
            SecretError::ConnectionFailed(format!(
                "bw unlock timed out after {}s",
                BW_INVOCATION_TIMEOUT.as_secs()
            ))
        })
}

/// Synchronous implementation of vault unlock.
///
/// Uses `std::process::Command` which is compatible with all Bitwarden CLI versions
/// including the new Rust-based CLI (v2026+).
///
/// Every child is bounded by [`BW_INVOCATION_TIMEOUT`]. None of them were: an
/// unlock that stalled on a network sync blocked its caller forever, and both
/// callers are worker threads with a status row waiting on them
/// (issue [#312](https://github.com/totoshko88/RustConn/issues/312)).
fn unlock_vault_sync(password: &str) -> SecretResult<SecretString> {
    // Presence, not length. A master password's length is bruteforce metadata,
    // and the GUI's unlock handler already declined to log it for that reason —
    // then sent all four of its call sites through here, where it was being
    // logged anyway.
    tracing::debug!(
        has_password = !password.is_empty(),
        "Bitwarden: unlock_vault_sync called"
    );

    let bw_cmd = get_bw_cmd();

    // Strategy 1: --passwordenv with --raw (returns session key directly)
    let output = unlock_attempt(&bw_cmd, password, true)?;

    let raw_succeeded = output.status.success();
    if raw_succeeded {
        let session_key = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !session_key.is_empty() {
            tracing::debug!("Bitwarden: unlocked with --passwordenv --raw");
            return Ok(SecretString::from(session_key));
        }
    }

    // `--raw` prints the key bare, with no `BW_SESSION=` in front of it, so
    // `redact_session_keys` has no marker to find here. That is why the content is
    // logged only when the command actually *failed*: exit 0 with empty stdout is
    // the one shape where the key could plausibly have gone to stderr instead, and
    // it is precisely the shape where redaction cannot recognise it. Kept as a
    // value regardless, because the `provided key` check below matches on it — a
    // `contains` yields a bool and cannot leak.
    let stderr_raw = redact_session_keys(&String::from_utf8_lossy(&output.stderr));
    if raw_succeeded {
        tracing::debug!(
            "Bitwarden: --raw unlock reported success but printed no key; trying verbose"
        );
    } else {
        tracing::debug!("Bitwarden: --raw unlock failed, trying verbose: {stderr_raw}");
    }

    // Strategy 2: --passwordenv without --raw (parse session key from verbose output)
    let output = unlock_attempt(&bw_cmd, password, false)?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(session_key) = extract_session_key(&stdout) {
            tracing::debug!("Bitwarden: unlocked with --passwordenv (verbose)");
            return Ok(SecretString::from(session_key));
        }
        tracing::debug!("Bitwarden: verbose unlock succeeded but no session key in output");
    }

    let stderr_verbose = redact_session_keys(&String::from_utf8_lossy(&output.stderr));
    tracing::debug!("Bitwarden: verbose unlock failed: {stderr_verbose}");

    // Strategy 3: stdin pipe without --raw (for maximum compatibility).
    //
    // No `--nointeraction` here, unlike the two attempts above: this strategy
    // exists for older CLI builds that have no `--passwordenv`, and it works by
    // answering the interactive prompt that `--nointeraction` suppresses.
    let mut child = std::process::Command::new(&bw_cmd)
        .env("PATH", crate::cli_download::get_extended_path())
        .arg("unlock")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run bw unlock: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(password.as_bytes());
        let _ = stdin.write_all(b"\n");
        drop(stdin);
    }

    // Bounded like the other two. The stdin write above is what unblocks a
    // healthy child; a child that never reads it used to park here forever.
    let output = crate::proc::wait_bounded(child, BW_INVOCATION_TIMEOUT, "bw unlock")
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to wait for bw: {e}")))?
        .output()
        .ok_or_else(|| {
            SecretError::ConnectionFailed(format!(
                "bw unlock timed out after {}s",
                BW_INVOCATION_TIMEOUT.as_secs()
            ))
        })?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(session_key) = extract_session_key(&stdout) {
            tracing::debug!("Bitwarden: unlocked with stdin pipe (verbose)");
            return Ok(SecretString::from(session_key));
        }
    }

    // Redacted before it is formatted into the error below, which the Secrets
    // page renders and `vault_ops` logs.
    let stderr = redact_session_keys(&String::from_utf8_lossy(&output.stderr));

    // Check if this is the "provided key" error — indicates corrupted vault data
    if stderr_raw.contains("provided key is not the expected type")
        || stderr.contains("provided key is not the expected type")
    {
        return Err(SecretError::ConnectionFailed(
            "Vault data may be corrupted (key type mismatch). \
             Try: bw logout → bw login → bw unlock"
                .to_string(),
        ));
    }

    Err(SecretError::ConnectionFailed(format!(
        "Failed to unlock vault: {stderr}"
    )))
}

/// Extracts session key from verbose `bw unlock` output.
///
/// Parses output lines looking for `BW_SESSION="<key>"` or `BW_SESSION=<key>`.
fn extract_session_key(output: &str) -> Option<String> {
    for line in output.lines() {
        if line.contains("BW_SESSION=") {
            // Try quoted format: export BW_SESSION="<key>"
            if let Some(start) = line.find('"')
                && let Some(end) = line.rfind('"')
                && end > start
            {
                return Some(line[start + 1..end].to_string());
            }
            // Try unquoted format: BW_SESSION=<key>
            if let Some(pos) = line.find("BW_SESSION=") {
                let value_start = pos + "BW_SESSION=".len();
                let value = line[value_start..].trim().trim_matches('"');
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

/// Locks the Bitwarden vault
///
/// # Errors
/// Returns `SecretError` if the lock command fails
pub async fn lock_vault() -> SecretResult<()> {
    clear_verified();
    clear_session_key();
    // Drop any persistent serve child: it holds the unlocked vault in its own
    // memory, so killing it is the lock for the serve transport. The external
    // `bw lock` below still runs for the CLI's own on-disk session state.
    shutdown_serve().await;
    let bw_cmd = get_bw_cmd();
    // Bounded like every other bw invocation: `lock` can trigger a network sync
    // that stalls, and this is awaited from the GUI's teardown path.
    let output = tokio::time::timeout(
        BW_INVOCATION_TIMEOUT,
        bw_command(&bw_cmd).arg("lock").output(),
    )
    .await
    .map_err(|_| {
        SecretError::ConnectionFailed(format!(
            "bw lock timed out after {}s",
            BW_INVOCATION_TIMEOUT.as_secs()
        ))
    })?
    .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run bw lock: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SecretError::ConnectionFailed(format!(
            "Failed to lock vault: {stderr}"
        )));
    }

    Ok(())
}

/// Logs in to Bitwarden using API key credentials
///
/// This is the recommended method for automated workflows and CI/CD pipelines.
/// Uses `BW_CLIENTID` and `BW_CLIENTSECRET` environment variables as documented.
///
/// After login, you must still call `unlock_vault()` to access vault data.
///
/// # Arguments
/// * `client_id` - Personal API key client_id
/// * `client_secret` - Personal API key client_secret
///
/// # Errors
/// Returns `SecretError` if login fails
pub async fn login_with_api_key(
    client_id: &SecretString,
    client_secret: &SecretString,
) -> SecretResult<()> {
    let bw_cmd = get_bw_cmd();
    // `login --apikey` reaches the Bitwarden server, so it is the most likely of
    // all bw invocations to hang on a slow or unreachable network. Bound it.
    let output = tokio::time::timeout(
        BW_INVOCATION_TIMEOUT,
        bw_command(&bw_cmd)
            .args(["login", "--apikey"])
            .env("BW_CLIENTID", client_id.expose_secret())
            .env("BW_CLIENTSECRET", client_secret.expose_secret())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| {
        SecretError::ConnectionFailed(format!(
            "bw login timed out after {}s",
            BW_INVOCATION_TIMEOUT.as_secs()
        ))
    })?
    .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run bw login: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SecretError::ConnectionFailed(format!(
            "Failed to login with API key: {stderr}"
        )));
    }

    Ok(())
}

/// Logs out from Bitwarden
///
/// # Errors
/// Returns `SecretError` if logout fails
pub async fn logout() -> SecretResult<()> {
    clear_verified();
    // A persistent serve child outlives a logout otherwise, still holding the
    // vault; drop it first.
    shutdown_serve().await;
    let bw_cmd = get_bw_cmd();
    // Bounded like the others: `logout` can attempt a final server round-trip.
    let output = tokio::time::timeout(
        BW_INVOCATION_TIMEOUT,
        bw_command(&bw_cmd).arg("logout").output(),
    )
    .await
    .map_err(|_| {
        SecretError::ConnectionFailed(format!(
            "bw logout timed out after {}s",
            BW_INVOCATION_TIMEOUT.as_secs()
        ))
    })?
    .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run bw logout: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Ignore "not logged in" error
        if !stderr.contains("not logged in") {
            return Err(SecretError::ConnectionFailed(format!(
                "Failed to logout: {stderr}"
            )));
        }
    }

    Ok(())
}

/// Configures Bitwarden CLI to use a self-hosted server
///
/// # Arguments
/// * `server_url` - URL of the self-hosted Bitwarden server
///
/// # Errors
/// Returns `SecretError` if configuration fails
pub async fn configure_server(server_url: &str) -> SecretResult<()> {
    let bw_cmd = get_bw_cmd();
    let output = bw_command(&bw_cmd)
        .args(["config", "server", server_url])
        .output()
        .await
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run bw config: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SecretError::ConnectionFailed(format!(
            "Failed to configure server: {stderr}"
        )));
    }

    Ok(())
}

// ============================================================================
// Keyring storage for Bitwarden credentials
// ============================================================================

const KEY_BW_MASTER: &str = "bitwarden-master";
const KEY_BW_CLIENT_ID: &str = "bitwarden-client-id";
const KEY_BW_CLIENT_SECRET: &str = "bitwarden-client-secret";

/// Stores Bitwarden master password in system keyring (libsecret)
///
/// # Errors
/// Returns `SecretError` if storage fails
pub async fn store_master_password_in_keyring(password: &SecretString) -> SecretResult<()> {
    super::keyring::store(
        KEY_BW_MASTER,
        password.expose_secret(),
        "Bitwarden Master Password",
    )
    .await
}

/// Retrieves Bitwarden master password from system keyring
///
/// # Errors
/// Returns `SecretError` if retrieval fails
pub async fn get_master_password_from_keyring() -> SecretResult<Option<SecretString>> {
    super::keyring::lookup(KEY_BW_MASTER)
        .await
        .map(|opt| opt.map(SecretString::from))
}

/// Deletes Bitwarden master password from system keyring
///
/// # Errors
/// Returns `SecretError` if deletion fails
pub async fn delete_master_password_from_keyring() -> SecretResult<()> {
    super::keyring::clear(KEY_BW_MASTER).await
}

/// Stores Bitwarden API credentials in system keyring
///
/// # Errors
/// Returns `SecretError` if storage fails
pub async fn store_api_credentials_in_keyring(
    client_id: &SecretString,
    client_secret: &SecretString,
) -> SecretResult<()> {
    super::keyring::store(
        KEY_BW_CLIENT_ID,
        client_id.expose_secret(),
        "Bitwarden API Client ID",
    )
    .await?;
    super::keyring::store(
        KEY_BW_CLIENT_SECRET,
        client_secret.expose_secret(),
        "Bitwarden API Client Secret",
    )
    .await?;
    Ok(())
}

/// Retrieves Bitwarden API credentials from system keyring
///
/// # Returns
/// Tuple of (`client_id`, `client_secret`) if both exist
///
/// # Errors
/// Returns `SecretError` if retrieval fails
pub async fn get_api_credentials_from_keyring() -> SecretResult<Option<(SecretString, SecretString)>>
{
    // Wrapped on the line it is read, so the plain `String` does not stay alive
    // across the second fallible lookup — where a failure would drop it un-wiped.
    let client_id = super::keyring::lookup(KEY_BW_CLIENT_ID)
        .await?
        .map(SecretString::from);
    let client_secret = super::keyring::lookup(KEY_BW_CLIENT_SECRET)
        .await?
        .map(SecretString::from);

    match (client_id, client_secret) {
        (Some(id), Some(secret)) => Ok(Some((id, secret))),
        _ => Ok(None),
    }
}

/// Deletes Bitwarden API credentials from system keyring
///
/// # Errors
/// Returns `SecretError` if deletion fails
pub async fn delete_api_credentials_from_keyring() -> SecretResult<()> {
    let _ = super::keyring::clear(KEY_BW_CLIENT_ID).await;
    let _ = super::keyring::clear(KEY_BW_CLIENT_SECRET).await;
    Ok(())
}

/// Attempts to automatically unlock the Bitwarden vault using saved credentials.
///
/// Tries the following sources in order:
/// 1. `BW_SESSION` environment variable (already unlocked)
/// 2. Master password from system keyring (libsecret)
/// 3. Master password from encrypted settings
///
/// Returns a `BitwardenBackend` with session key set, or an error
/// with a user-friendly message.
///
/// # Errors
/// Returns `SecretError` if vault cannot be unlocked from any source
/// Attempts API key login when vault is unauthenticated.
///
/// Decrypts stored API credentials if needed and calls `bw login --apikey`.
///
/// # Errors
/// Returns `SecretError::BackendUnavailable` if login fails or credentials are missing.
async fn try_api_key_login(settings: &crate::config::SecretSettings) -> SecretResult<()> {
    if !settings.bitwarden_use_api_key {
        tracing::warn!("Bitwarden: vault unauthenticated, API key login not enabled");
        return Err(SecretError::BackendUnavailable(
            "Bitwarden vault is not logged in. \
             Run 'bw login' in terminal or enable API key in Settings → Secrets."
                .to_string(),
        ));
    }

    let mut settings_clone = settings.clone();
    if (settings_clone.bitwarden_client_id.is_none()
        || settings_clone.bitwarden_client_secret.is_none())
        && (settings_clone.bitwarden_client_id_encrypted.is_some()
            || settings_clone.bitwarden_client_secret_encrypted.is_some())
    {
        settings_clone.decrypt_bitwarden_api_credentials();
    }

    if let (Some(ref client_id), Some(ref client_secret)) = (
        settings_clone.bitwarden_client_id,
        settings_clone.bitwarden_client_secret,
    ) {
        tracing::debug!("Bitwarden: attempting API key login");
        login_with_api_key(client_id, client_secret)
            .await
            .map_err(|e| {
                tracing::warn!("Bitwarden: API key login failed: {e}");
                SecretError::BackendUnavailable(
                    "Bitwarden vault is not logged in. \
                 API key login failed. Check credentials in Settings → Secrets."
                        .to_string(),
                )
            })?;
        tracing::info!("Bitwarden: API key login successful");
        Ok(())
    } else {
        tracing::warn!("Bitwarden: vault unauthenticated but no API credentials configured");
        Err(SecretError::BackendUnavailable(
            "Bitwarden vault is not logged in. \
             Run 'bw login' in terminal or configure API key in Settings → Secrets."
                .to_string(),
        ))
    }
}

/// Attempts to fix "key type mismatch" by re-logging in and retrying unlock.
///
/// This handles corrupted vault data (e.g. after CLI upgrade from Node.js to Rust).
/// Performs: `bw logout` → `bw login --apikey` → `bw unlock`.
///
/// # Errors
/// Returns `SecretError` if re-login or unlock fails, or if API key is not configured.
async fn try_relogin_and_unlock(
    settings: &crate::config::SecretSettings,
    password: &SecretString,
) -> SecretResult<()> {
    if !settings.bitwarden_use_api_key {
        tracing::debug!("Bitwarden: key type mismatch but no API key configured, cannot re-login");
        return Err(SecretError::BackendUnavailable(
            "Vault data corrupted. Run 'bw logout && bw login' manually.".to_string(),
        ));
    }

    tracing::info!("Bitwarden: key type mismatch detected, attempting re-login");

    // Logout (ignore errors — might already be logged out)
    let _ = logout().await;

    // Re-login with API key
    try_api_key_login(settings).await.map_err(|e| {
        tracing::warn!("Bitwarden: re-login failed: {e}");
        SecretError::BackendUnavailable(
            "Vault data corrupted and re-login failed. \
             Run 'bw logout && bw login' manually."
                .to_string(),
        )
    })?;

    // Retry unlock
    let session_key = unlock_vault(password).await.map_err(|e| {
        tracing::warn!("Bitwarden: unlock after re-login failed: {e}");
        SecretError::BackendUnavailable(
            "Re-login succeeded but unlock still failed. Check master password.".to_string(),
        )
    })?;

    set_session_key(session_key);
    tracing::info!("Bitwarden: re-login + unlock successful");
    Ok(())
}

/// Attempts to auto-unlock the Bitwarden vault using saved credentials.
///
/// Tries the following strategies in order:
/// 1. Existing session key from in-process storage
/// 2. Vault already unlocked (no session needed)
/// 3. API key login if vault is unauthenticated
/// 4. Master password from system keyring
/// 5. Master password from encrypted settings
///
/// The returned backend carries the `bitwarden_read_only` /
/// `bitwarden_root_search` toggles from `settings`
/// ([`BitwardenBackend::with_settings_toggles`]), so a write through it is
/// refused when the user put Bitwarden in read-only mode.
///
/// # Errors
/// Returns `SecretError::BackendUnavailable` if all strategies fail.
pub async fn auto_unlock(
    settings: &crate::config::SecretSettings,
) -> SecretResult<BitwardenBackend> {
    // Times the whole unlock to "vault ready" — the 0.24 target of ≤ 4 s starts
    // here. Off unless `RUST_LOG=rustconn::perf=info`; see `tracing::perf`.
    let _perf = crate::tracing::perf::PerfSpan::new("vault.unlock", "bitwarden");

    // Apply the persistent-CLI preference from the current settings. This is the
    // one choke point every startup and connect unlock passes through, so saving
    // the Settings → Secrets toggle and reconnecting is enough to switch the
    // transport; a toggle turned off also means the next lock/rebuild tears the
    // serve child down (see `set_serve_preferred` / `shutdown_serve`).
    set_serve_preferred(settings.bitwarden_persistent_cli);

    // One unlock at a time, process-wide. The GUI's startup unlock runs on a
    // thread of its own, outside the vault-op lock, so a connect made while it
    // was still going started a second `bw status` / `bw unlock` / `bw sync`
    // alongside it — every step twice in the issue #353 logs, each `bw` call
    // seconds long. Waiting here instead lets the second caller find the
    // session the first one just stored and take the fast path. The Settings
    // page's `unlock_vault_blocking` waits on the same lock.
    let _one_at_a_time = BW_UNLOCK.lock().await;

    // One place for the toggles: the unlock below has many return points, and
    // each used to hand back a backend with read-only off.
    unlock_with_saved_credentials(settings)
        .await
        .map(|backend| backend.with_settings_toggles(settings))
}

/// The unlock strategies behind [`auto_unlock`], without the settings toggles.
#[expect(
    clippy::too_many_lines,
    reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
)] // multi-strategy unlock with ordered fallbacks
async fn unlock_with_saved_credentials(
    settings: &crate::config::SecretSettings,
) -> SecretResult<BitwardenBackend> {
    // 0. Fast path: if session key exists and was recently verified, skip
    //    all CLI calls entirely. This makes reconnect near-instant.
    let stored_session = get_session_key().or_else(|| {
        std::env::var("BW_SESSION")
            .ok()
            .filter(|s| !s.is_empty())
            .map(SecretString::from)
    });
    if let Some(ref session) = stored_session
        && is_recently_verified()
    {
        tracing::debug!("Bitwarden: using cached session key (recently verified)");
        // Third site of the same round trip as the two below: going through
        // `expose_secret().to_owned()` puts the key in a bare `String` that drops
        // unwiped. `clone` and not a move because `stored_session` is borrowed and
        // is read again further down.
        return Ok(BitwardenBackend::with_session(session.clone()));
    }

    // 1. Check in-process session store, then fall back to BW_SESSION env var
    //    (supports externally unlocked vaults, e.g. `export BW_SESSION=...` in shell)
    //
    // Trust the session key without calling `bw status` — on Bitwarden CLI
    // v2026.4+ `bw status` ignores BW_SESSION and always reports "locked".
    // If the session is expired, the subsequent data command will fail and
    // the caller can retry.
    if let Some(session) = stored_session {
        tracing::debug!("Bitwarden: using existing session key (trusting without status check)");
        mark_verified();
        let backend = BitwardenBackend::with_session(session);
        // The verify TTL is short, and a sync costs seconds on every first
        // vault operation after it; sync only once SYNC_TTL_SECS has passed.
        // A refused sign-in clears the mark (`sync_on_next_unlock`), so the
        // retry still reads a fresh item (issue #351).
        if sync_due() {
            let _ = backend.sync().await;
        } else {
            tracing::debug!("Bitwarden: skipping bw sync (synced recently)");
        }
        return Ok(backend);
    }

    // 2. Check if vault is already unlocked (no session needed)
    let bare = BitwardenBackend::new();
    let status = bare.get_status().await;
    tracing::debug!(
        vault_status = ?status.as_ref().map(|s| &s.status),
        "Bitwarden: vault status before unlock"
    );

    if status
        .as_ref()
        .map(|s| s.status == "unlocked")
        .unwrap_or(false)
    {
        tracing::debug!("Bitwarden: vault already unlocked");
        mark_verified();
        // No blocking `bw sync` on the unlock path: it cost ~6 s in the #353
        // startup trace and sat on the critical path to a ready vault. The
        // sync is deferred — `BW_LAST_SYNCED` is left as it is, so `sync_due()`
        // fires it on the first vault operation (or the startup caller runs it
        // in the background after signalling readiness). Freshness is kept
        // (#351): the first read still syncs; only the *when* moved.
        return Ok(bare);
    }

    // 2b. Check if vault is unauthenticated — need login before unlock
    let needs_login = status
        .as_ref()
        .map(|s| s.status == "unauthenticated")
        .unwrap_or(false);

    if needs_login {
        try_api_key_login(settings).await?;
    }

    // 3. Try master password from system keyring
    if settings.bitwarden_save_to_keyring
        && let Ok(Some(password)) = get_master_password_from_keyring().await
    {
        tracing::debug!("Bitwarden: attempting unlock with keyring password");
        match unlock_vault(&password).await {
            Ok(session_key) => {
                // `clone`, not `SecretString::from(expose_secret().to_owned())`:
                // the round trip put the key in a bare `String` that dropped
                // unwiped. Same defect the Secrets page had at four call sites.
                set_session_key(session_key.clone());
                mark_verified();
                let backend = BitwardenBackend::with_session(session_key);
                // Deferred sync — see the "vault already unlocked" arm above.
                return Ok(backend);
            }
            Err(e) => {
                let is_key_type_error = e.to_string().contains("key type mismatch");
                tracing::warn!("Bitwarden: keyring password unlock failed: {e}");
                // If key type mismatch, try re-login before giving up
                if is_key_type_error
                    && try_relogin_and_unlock(settings, &password).await.is_ok()
                    && let Some(key) = get_session_key()
                {
                    mark_verified();
                    return Ok(BitwardenBackend::with_session(key));
                }
            }
        }
    }

    // 4. Try master password from encrypted settings
    if settings.bitwarden_password.is_some() || settings.bitwarden_password_encrypted.is_some() {
        let mut settings_clone = settings.clone();
        if settings_clone.bitwarden_password.is_none()
            && settings_clone.bitwarden_password_encrypted.is_some()
        {
            settings_clone.decrypt_bitwarden_password();
        }
        if let Some(ref password) = settings_clone.bitwarden_password {
            tracing::debug!("Bitwarden: attempting unlock with saved password");
            match unlock_vault(password).await {
                Ok(session_key) => {
                    // See the sibling arm above: `clone` keeps the key inside
                    // `SecretString` instead of round-tripping it through a bare
                    // `String` that drops unwiped.
                    set_session_key(session_key.clone());
                    mark_verified();
                    let backend = BitwardenBackend::with_session(session_key);
                    // Deferred sync — see the "vault already unlocked" arm above.
                    return Ok(backend);
                }
                Err(e) => {
                    let is_key_type_error = e.to_string().contains("key type mismatch");
                    tracing::warn!("Bitwarden: saved password unlock failed: {e}");
                    // If key type mismatch, try re-login before giving up
                    if is_key_type_error
                        && try_relogin_and_unlock(settings, password).await.is_ok()
                        && let Some(key) = get_session_key()
                    {
                        mark_verified();
                        return Ok(BitwardenBackend::with_session(key));
                    }
                }
            }
        }
    }

    Err(SecretError::BackendUnavailable(
        "Bitwarden vault is locked. Unlock it in Settings → Secrets \
         or run 'bw unlock' in terminal."
            .to_string(),
    ))
}

/// Base64 encode helper (standard base64 with padding)
fn base64_encode(data: &[u8]) -> String {
    data_encoding::BASE64.encode(data)
}

/// [`base64_encode`] for an item that carries a password. The encoding is
/// reversible, so it is wiped on drop exactly like the JSON it came from.
fn base64_encode_secret(data: &[u8]) -> zeroize::Zeroizing<String> {
    zeroize::Zeroizing::new(base64_encode(data))
}

impl std::fmt::Debug for BitwardenBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BitwardenBackend")
            // Never log the actual session key — only its presence.
            .field("session_present", &self.session_key.is_some())
            .field("server_url", &self.server_url)
            .field("organization_id", &self.organization_id)
            .field("folder_name", &self.folder_name)
            .field("read_only", &self.read_only)
            .field("root_search", &self.root_search)
            .field("bw_cmd", &self.bw_cmd)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    #[test]
    fn debug_does_not_leak_secret() {
        let session = SecretString::from("hunter2-bw-session".to_string());
        let backend = BitwardenBackend::with_session(session)
            .with_server_url("https://vault.example.org")
            .with_organization("org-hunter2");
        let rendered = format!("{backend:?}");
        assert!(
            !rendered.contains("hunter2-bw-session"),
            "Debug leaked the session key: {rendered}"
        );
        assert!(rendered.contains("BitwardenBackend"));
        assert!(rendered.contains("session_present"));
    }
}

#[cfg(test)]
mod unlock_tests {
    use super::{
        BW_INVOCATION_TIMEOUT, BW_SESSION_STORE, BW_UNLOCK, BitwardenBackend, SYNC_TTL_SECS,
        SecretBackend, SecretString, clear_bitwarden_session_state, has_session_key,
        is_recently_verified, is_sync_due, mark_synced, mark_verified, redact_session_keys,
        sync_due, sync_on_next_unlock, with_unlock_lock_blocking,
    };
    use std::sync::{Mutex, MutexGuard, PoisonError};

    /// The session store and the verify/sync marks are process-wide, and the
    /// test harness runs tests in parallel; every test here that touches them
    /// holds this.
    static GLOBALS: Mutex<()> = Mutex::new(());

    fn lock_globals() -> MutexGuard<'static, ()> {
        GLOBALS.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sets the global session store directly, so the verify mark is left
    /// alone (`clear_session_key` also clears it).
    fn set_global_session(key: Option<&str>) {
        *BW_SESSION_STORE.write().unwrap() = key.map(SecretString::from);
    }

    /// A stored session proves an unlock succeeded, so `is_available` answers
    /// without spawning `bw`. The bogus command path makes any spawn fail, so
    /// a `true` here can only come from the session check.
    #[test]
    fn is_available_trusts_a_session_without_spawning_bw() {
        let _globals = lock_globals();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let no_bw = |backend: BitwardenBackend| BitwardenBackend {
            bw_cmd: "/nonexistent/rustconn-test-bw".to_owned(),
            ..backend
        };

        set_global_session(None);
        assert!(!has_session_key());
        assert!(
            !runtime.block_on(no_bw(BitwardenBackend::new()).is_available()),
            "without a session the CLI probe decides, and there is no CLI"
        );

        let own = no_bw(BitwardenBackend::with_session(SecretString::from("k")));
        assert!(runtime.block_on(own.is_available()), "instance session");

        set_global_session(Some("global-k"));
        assert!(has_session_key());
        let global = runtime.block_on(no_bw(BitwardenBackend::new()).is_available());
        set_global_session(None);
        assert!(global, "global session from auto_unlock");
    }

    /// `unlock_vault_blocking` takes the lock `auto_unlock` holds: a blocking
    /// caller waits while it is held and proceeds once it is released.
    #[test]
    fn the_blocking_unlock_waits_for_a_held_unlock_lock() {
        use std::sync::mpsc;
        use std::time::Duration;

        // Free lock: no deadlock, and the closure's value comes back.
        assert_eq!(with_unlock_lock_blocking(|| 7), 7);
        assert_eq!(with_unlock_lock_blocking(|| 8), 8);

        let held = BW_UNLOCK.blocking_lock();
        let (started_tx, started_rx) = mpsc::channel();
        let (entered_tx, entered_rx) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            with_unlock_lock_blocking(|| entered_tx.send(()).unwrap());
        });
        started_rx.recv().unwrap();
        assert!(
            entered_rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "entered the unlock while another one held the lock"
        );

        drop(held);
        entered_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the waiter must proceed once the lock is released");
        waiter.join().unwrap();
    }

    /// An `auto_unlock` holding the lock across awaits on a runtime blocks the
    /// blocking entry the same way, and releasing it lets that entry through.
    #[test]
    fn the_blocking_unlock_waits_for_an_async_holder() {
        use std::sync::mpsc;
        use std::time::Duration;

        let (locked_tx, locked_rx) = mpsc::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let holder = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let _guard = BW_UNLOCK.lock().await;
                locked_tx.send(()).unwrap();
                let _ = release_rx.await;
            });
        });
        locked_rx.recv().unwrap();

        let (entered_tx, entered_rx) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            with_unlock_lock_blocking(|| entered_tx.send(()).unwrap());
        });
        assert!(entered_rx.recv_timeout(Duration::from_millis(200)).is_err());

        release_tx.send(()).unwrap();
        entered_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the waiter must proceed once auto_unlock is done");
        holder.join().unwrap();
        waiter.join().unwrap();
    }

    /// `store` hands the item — which holds the plaintext password — to `bw`
    /// on stdin. As an argument it was readable by every user in
    /// `/proc/<pid>/cmdline`. A stub `bw` records its argv and stdin; both the
    /// create path (no item yet) and the edit path (item exists) are checked.
    #[cfg(unix)]
    #[test]
    fn store_passes_the_item_on_stdin_not_in_argv() {
        use crate::models::Credentials;
        use std::os::unix::fs::PermissionsExt;

        let _globals = lock_globals();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        for (existing_items, verb) in [
            ("[]", "create item"),
            (
                r#"[{"id":"i1","name":"RustConn: c1","notes":null,"login":null}]"#,
                "edit item i1",
            ),
        ] {
            let dir = tempfile::TempDir::new().unwrap();
            let stub = dir.path().join("bw");
            // $1 is the `--nointeraction` every `bw` call starts with.
            let script = format!(
                "#!/bin/sh\n\
                 d=\"$(dirname \"$0\")\"\n\
                 printf '%s\\n' \"$*\" >> \"$d/argv\"\n\
                 case \"$2 $3\" in\n\
                 \"list folders\") echo '[{{\"id\":\"f1\",\"name\":\"RustConn\"}}]' ;;\n\
                 \"list items\") echo '{existing_items}' ;;\n\
                 \"create item\"|\"edit item\") cat > \"$d/stdin\"; echo '{{}}' ;;\n\
                 esac\n"
            );
            std::fs::write(&stub, script).unwrap();
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o700)).unwrap();

            let backend = BitwardenBackend {
                bw_cmd: stub.to_string_lossy().into_owned(),
                ..BitwardenBackend::with_session(SecretString::from("test-session"))
            };
            let mut creds = Credentials::with_username("alice");
            creds.password = Some(SecretString::from("hunter2-store"));
            // ETXTBSY race: a just-written, just-chmod-executable stub can still
            // be held open for write by the OS under parallel test load, so the
            // exec inside `store` intermittently fails with "Text file busy".
            // That is a test-harness artefact, not a backend bug, so retry the
            // store a few times on exactly that error before giving up.
            let mut attempts = 0;
            loop {
                attempts += 1;
                match runtime.block_on(backend.store("c1", &creds)) {
                    Ok(()) => break,
                    Err(e) if e.to_string().contains("Text file busy") && attempts < 10 => {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    Err(e) => panic!("store through the stub bw: {e:?}"),
                }
            }

            let argv = std::fs::read_to_string(dir.path().join("argv")).unwrap();
            let stdin = std::fs::read_to_string(dir.path().join("stdin")).unwrap();
            let item = data_encoding::BASE64
                .decode(stdin.trim().as_bytes())
                .expect("stdin is the base64 item");
            let item = String::from_utf8(item).unwrap();

            assert!(item.contains("hunter2-store"), "the item reached bw");
            assert!(
                argv.lines().any(|l| l == format!("--nointeraction {verb}")),
                "expected a bare `{verb}`: {argv}"
            );
            assert!(!argv.contains(stdin.trim()), "item in argv: {argv}");
            assert!(!argv.contains("hunter2"), "password in argv: {argv}");
        }
    }

    /// The `bw` calls `find_item_with_aliases` makes, recorded by a stub `bw`
    /// whose every search comes back empty (the worst case: nothing short-cuts).
    /// Root search off: the scoped search alone. On: one search per candidate
    /// and no scoped one, because the id search covers it. An id that
    /// `bare_candidates` trims is not the first candidate, so the scoped search
    /// comes back for it.
    #[cfg(unix)]
    #[test]
    fn root_search_issues_one_bw_search_per_candidate() {
        use std::os::unix::fs::PermissionsExt;

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        for (root_search, id, expected) in [
            (false, "c1", vec!["list items --search RustConn: c1"]),
            (
                true,
                "c1",
                vec!["list items --search c1", "list items --search host.example"],
            ),
            (
                true,
                " c1",
                vec![
                    "list items --search RustConn:  c1",
                    "list items --search c1",
                    "list items --search host.example",
                ],
            ),
        ] {
            let dir = tempfile::TempDir::new().unwrap();
            let stub = dir.path().join("bw");
            // $1 is the `--nointeraction` every `bw` call starts with.
            let script = "#!/bin/sh\n\
                 d=\"$(dirname \"$0\")\"\n\
                 shift\n\
                 printf '%s\\n' \"$*\" >> \"$d/argv\"\n\
                 echo '[]'\n";
            std::fs::write(&stub, script).unwrap();
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o700)).unwrap();

            let backend = BitwardenBackend {
                bw_cmd: stub.to_string_lossy().into_owned(),
                ..BitwardenBackend::with_session(SecretString::from("test-session"))
                    .with_root_search(root_search)
            };
            // ETXTBSY race: a just-written, just-chmod-executable stub can still
            // be held open for write under parallel test load, so the exec here
            // intermittently fails with "Text file busy". That is a test-harness
            // artefact, not a backend bug (the sibling `store` test retries the
            // same way); retry the lookup a few times on exactly that error.
            let mut attempts = 0;
            let found = loop {
                attempts += 1;
                match runtime.block_on(backend.find_item_with_aliases(id, &["host.example"])) {
                    Ok(found) => break found,
                    Err(e) if e.to_string().contains("Text file busy") && attempts < 10 => {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    Err(e) => panic!("lookup through the stub bw: {e:?}"),
                }
            };
            assert!(found.is_none());

            let argv = std::fs::read_to_string(dir.path().join("argv")).unwrap();
            assert_eq!(
                argv.lines().collect::<Vec<_>>(),
                expected,
                "root_search={root_search} id={id:?}"
            );
        }
    }

    /// Never synced, or synced longer ago than the TTL: sync. Synced within
    /// it: skip.
    #[test]
    fn sync_is_due_only_after_the_sync_ttl() {
        use std::time::{Duration, SystemTime};

        assert!(is_sync_due(None));
        assert!(!is_sync_due(Some(SystemTime::now())));
        let ttl = Duration::from_secs(SYNC_TTL_SECS);
        assert!(!is_sync_due(SystemTime::now().checked_sub(ttl / 2)));
        if let Some(old) = SystemTime::now().checked_sub(ttl + Duration::from_secs(1)) {
            assert!(is_sync_due(Some(old)));
        }
        // A mark ahead of the clock (time set back) must not suppress syncs.
        assert!(is_sync_due(SystemTime::now().checked_add(ttl)));
        // Longer than the verify TTL, or the separate mark saves nothing.
        const { assert!(SYNC_TTL_SECS > super::UNLOCK_VERIFY_TTL_SECS) };
    }

    /// A refused sign-in must force the next unlock to sync even when the last
    /// sync is recent (issue #351): `sync_on_next_unlock` clears the sync mark
    /// along with the verify mark.
    #[test]
    fn sync_on_next_unlock_forgets_the_recent_sync() {
        let _globals = lock_globals();
        mark_synced();
        mark_verified();
        assert!(!sync_due());

        sync_on_next_unlock();

        assert!(sync_due());
        assert!(!is_recently_verified());
    }

    /// Unlock stderr reaches a debug log *and* the error string the Secrets page
    /// renders. `extract_session_key` looks for the export banner on stdout, so a
    /// `bw` build that writes it to stderr instead would leak the vault's bearer
    /// token down the diagnostic path — the one path that never parses it.
    #[test]
    fn a_session_key_on_stderr_does_not_survive_into_a_message() {
        let stderr = "\
Something went wrong
export BW_SESSION=\"hunter2-session-key\"
Invalid master password.";

        let redacted = redact_session_keys(stderr);

        assert!(
            !redacted.contains("hunter2-session-key"),
            "session key survived redaction: {redacted}"
        );
        // The reason is what both callers match on, so it has to stay.
        assert!(redacted.contains("Invalid master password."));
        assert!(redacted.contains("Something went wrong"));
        // Redacted by line, so the shape of the output is preserved.
        assert_eq!(redacted.lines().count(), 3);
    }

    #[test]
    fn the_unquoted_form_is_redacted_too() {
        let redacted = redact_session_keys("BW_SESSION=bare-key-no-quotes");
        assert!(!redacted.contains("bare-key-no-quotes"));
    }

    /// Ordinary stderr must pass through untouched, or the callers that match on
    /// "not logged in" and "provided key is not the expected type" would stop
    /// recognising it.
    #[test]
    fn stderr_without_a_session_key_is_left_alone() {
        for message in [
            "You are not logged in.",
            "Invalid master password.",
            "provided key is not the expected type",
            "",
        ] {
            assert_eq!(redact_session_keys(message), message);
        }
    }

    /// The budget has to clear a real unlock, measured at about 4.5 s in issue
    /// #312, by enough margin that a slow vault server is not mistaken for a hang.
    #[test]
    fn the_invocation_budget_clears_a_real_unlock() {
        assert!(BW_INVOCATION_TIMEOUT >= std::time::Duration::from_secs(15));
    }

    /// After a refused sign-in the next unlock must not take the fast path that
    /// skips `bw sync`, or a corrected item is not seen (issue #351). No other
    /// test touches the verification timestamp, so this one does not race.
    #[test]
    fn sync_on_next_unlock_forgets_the_recent_check() {
        let _globals = lock_globals();
        mark_verified();
        assert!(is_recently_verified());

        sync_on_next_unlock();

        assert!(!is_recently_verified());
    }

    /// Switching the preferred backend away from Bitwarden must drop every
    /// process-global unlock mark, or a stale session would keep answering the
    /// fast path and `sync_due` would stay satisfied from the old backend.
    #[test]
    fn clearing_session_state_drops_session_verify_and_sync_marks() {
        let _globals = lock_globals();

        set_global_session(Some("stale-session-after-switch"));
        mark_verified();
        mark_synced();
        assert!(has_session_key(), "precondition: a session is stored");
        assert!(is_recently_verified(), "precondition: verified mark set");
        assert!(
            !sync_due(),
            "precondition: a recent sync satisfies sync_due"
        );

        clear_bitwarden_session_state();

        assert!(!has_session_key(), "session key must be cleared");
        assert!(!is_recently_verified(), "verify mark must be cleared");
        assert!(
            sync_due(),
            "sync mark must be cleared so the new backend syncs on first use"
        );

        // Leave the globals clean for sibling tests sharing the lock.
        set_global_session(None);
    }
}

/// Stopping the serve child on screen lock or suspend, against a local slot so
/// the process-wide child is never touched.
#[cfg(all(test, feature = "bitwarden-serve"))]
mod serve_lock_tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::{ServeManager, finish_pending_serve_stop, stop_serve_now_or_after_op};
    use crate::secret::bitwarden_serve::ServeProcess;

    /// A slot holding a long-lived stand-in child, killed when the test drops it.
    fn running_slot() -> tokio::sync::Mutex<ServeManager> {
        let mut command = tokio::process::Command::new("sleep");
        command.arg("30").kill_on_drop(true);
        let child = command.spawn().expect("spawn sleep");
        let mut manager = ServeManager::new();
        manager.process = Some(ServeProcess::from_child(child));
        tokio::sync::Mutex::new(manager)
    }

    #[tokio::test]
    async fn an_idle_child_is_stopped_at_once() {
        let slot = running_slot();
        let pending = AtomicBool::new(false);

        assert!(stop_serve_now_or_after_op(&slot, &pending));
        assert!(slot.lock().await.process.is_none());
        assert!(
            !pending.load(Ordering::SeqCst),
            "nothing left for an op to do"
        );
    }

    #[tokio::test]
    async fn a_busy_child_is_stopped_by_the_op_once_it_releases_the_slot() {
        let slot = running_slot();
        let pending = AtomicBool::new(false);

        // A vault op holds the slot when the screen locks.
        let held = slot.lock().await;
        assert!(!stop_serve_now_or_after_op(&slot, &pending));
        assert!(
            held.process.is_some(),
            "the op's child is not pulled from under it"
        );
        drop(held);

        // The op releases the slot and finishes the stop.
        assert!(finish_pending_serve_stop(&slot, &pending).await);
        assert!(slot.lock().await.process.is_none());
        assert!(!pending.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn an_op_without_a_pending_stop_leaves_the_child_running() {
        let slot = running_slot();
        let pending = AtomicBool::new(false);

        assert!(!finish_pending_serve_stop(&slot, &pending).await);
        assert!(slot.lock().await.process.is_some());
    }

    #[tokio::test]
    async fn no_child_means_nothing_stopped_and_nothing_pending() {
        let slot = tokio::sync::Mutex::new(ServeManager::new());
        let pending = AtomicBool::new(false);

        assert!(!stop_serve_now_or_after_op(&slot, &pending));
        assert!(!pending.load(Ordering::SeqCst));
    }

    /// A lock event must keep the fallback latch: a session that fell back to
    /// spawns because the CLI cannot serve should not retry serve after a lock.
    #[tokio::test]
    async fn the_fallback_latch_survives_a_lock_event() {
        let slot = running_slot();
        slot.lock().await.disabled = true;
        let pending = AtomicBool::new(false);

        assert!(stop_serve_now_or_after_op(&slot, &pending));
        assert!(slot.lock().await.disabled);
    }
}

#[cfg(test)]
mod read_only_tests {
    use super::*;
    use crate::error::SecretError;
    use crate::models::Credentials;

    #[test]
    fn with_read_only_sets_capability() {
        assert!(!BitwardenBackend::new().is_read_only());
        assert!(BitwardenBackend::new().with_read_only(true).is_read_only());
    }

    #[tokio::test]
    async fn read_only_refuses_store_before_unlock_check() {
        // ensure_writable() is the first line of store(), so the read-only
        // refusal takes precedence over the vault-locked check and no `bw`
        // process is spawned.
        let backend = BitwardenBackend::new().with_read_only(true);
        let err = backend
            .store("conn-1", &Credentials::default())
            .await
            .unwrap_err();
        assert!(matches!(err, SecretError::ReadOnly(name) if name == "Bitwarden"));
    }

    #[tokio::test]
    async fn read_only_refuses_delete() {
        let backend = BitwardenBackend::new().with_read_only(true);
        let err = backend.delete("conn-1").await.unwrap_err();
        assert!(matches!(err, SecretError::ReadOnly(_)));
    }
}

#[cfg(test)]
mod root_search_tests {
    use super::*;

    #[test]
    fn searches_from_root_defaults_false_and_builder_toggles_it() {
        assert!(!BitwardenBackend::new().searches_from_root());
        assert!(
            BitwardenBackend::new()
                .with_root_search(true)
                .searches_from_root()
        );
        // Setting it back to false is honoured.
        assert!(
            !BitwardenBackend::new()
                .with_root_search(true)
                .with_root_search(false)
                .searches_from_root()
        );
    }

    #[test]
    fn read_only_and_root_search_are_independent() {
        let both = BitwardenBackend::new()
            .with_read_only(true)
            .with_root_search(true);
        assert!(both.is_read_only());
        assert!(both.searches_from_root());
    }

    /// Documents the two names the root-search widening compares against: the
    /// scoped lookup matches `RustConn: {id}` exactly, and the fallback matches
    /// the bare `{id}`. The fallback in `find_item` runs a second
    /// `bw list items --search {id}` and keeps the item whose `name` equals the
    /// bare id, so these two strings are the whole of the predicate — but the
    /// list itself requires the live `bw` CLI and an unlocked vault, so the
    /// end-to-end widening cannot be unit-tested here without faking CLI output
    /// (which this suite deliberately does not do).
    #[test]
    fn entry_name_is_the_scoped_form_and_the_bare_id_is_the_fallback_form() {
        assert_eq!(BitwardenBackend::entry_name("conn-1"), "RustConn: conn-1");
        // The bare connection id — the fallback search term and match key — is
        // distinct from the scoped name, so an entry titled by the bare id is
        // not already caught by the scoped pass.
        assert_ne!(BitwardenBackend::entry_name("conn-1"), "conn-1");
    }

    /// Issue #353: the root-search fallback must match a vault entry titled by
    /// the connection NAME or HOST, not only by the UUID — case-insensitively,
    /// exact (not substring). `root_search_matches` over the candidate set
    /// `[uuid, host, name]` is the whole of that predicate (`find_item` only
    /// runs the `bw` searches around it), so it is unit-tested here without the
    /// live CLI.
    #[test]
    fn root_search_matches_uuid_name_and_host_exactly_and_case_insensitively() {
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let host = "db.example.com";
        let name = "Prod Database";
        // `bare_candidates` is the exact candidate order a real lookup builds:
        // id first, then the host/name aliases, de-duplicated.
        let candidates = BitwardenBackend::bare_candidates(uuid, &[host, name]);

        // The UUID still matches (back-compat).
        assert!(BitwardenBackend::root_search_matches(uuid, &candidates));
        // The host and name match (the #353 widening).
        assert!(BitwardenBackend::root_search_matches(host, &candidates));
        assert!(BitwardenBackend::root_search_matches(name, &candidates));
        // Case-insensitive and whitespace-trimmed.
        assert!(BitwardenBackend::root_search_matches(
            "  DB.EXAMPLE.COM  ",
            &candidates
        ));
        assert!(BitwardenBackend::root_search_matches(
            "prod database",
            &candidates
        ));

        // Exact, NOT substring: a longer title that merely contains a candidate
        // must not match, or a bare `db` would steal `database-prod`.
        assert!(!BitwardenBackend::root_search_matches(
            "db.example.com.evil",
            &candidates
        ));
        assert!(!BitwardenBackend::root_search_matches(
            "Prod Database (old)",
            &candidates
        ));
        // An unrelated title never matches.
        assert!(!BitwardenBackend::root_search_matches(
            "unrelated",
            &candidates
        ));
    }

    /// With no aliases the candidate set is just `[id]`, so the fallback is
    /// byte-identical to the pre-#353 id-only behaviour: the id matches, nothing
    /// else does.
    #[test]
    fn empty_aliases_reduces_to_id_only_matching() {
        let candidates = BitwardenBackend::bare_candidates("conn-1", &[]);
        assert_eq!(candidates, vec!["conn-1"]);
        assert!(BitwardenBackend::root_search_matches("conn-1", &candidates));
        assert!(!BitwardenBackend::root_search_matches(
            "other-host",
            &candidates
        ));
    }

    /// Issue #353 (the Bitwarden half): a connection NAMED by a slash path
    /// (`AME/DE/11/hanfile03`) must match a vault entry titled by the bare leaf
    /// (`hanfile03`). The leaf arrives as a candidate from
    /// `LookupIdentity::aliases`, so the full chain — identity -> aliases ->
    /// `bare_candidates` -> `root_search_matches` — is what a real lookup runs.
    /// Before the fix the leaf was never a candidate and this missed, while the
    /// KeePass path (basename-extracting) matched the same vault.
    #[test]
    fn slash_named_connection_matches_a_leaf_titled_entry() {
        use crate::secret::backend::LookupIdentity;

        let identity = LookupIdentity {
            key: "rustconn/hanfile03",
            name: Some("AME/DE/11/hanfile03"),
            host: Some("10.0.0.1"),
        };
        let aliases = identity.aliases();
        let candidates = BitwardenBackend::bare_candidates(identity.key, &aliases);

        // The bare leaf is now a candidate and matches the real vault title.
        assert!(BitwardenBackend::root_search_matches(
            "hanfile03",
            &candidates
        ));
        // The full slash path still matches when the user titled it that way.
        assert!(BitwardenBackend::root_search_matches(
            "AME/DE/11/hanfile03",
            &candidates
        ));
        // Exactness is preserved: a different leaf under the same path does not
        // match, and a substring of the leaf does not match.
        assert!(!BitwardenBackend::root_search_matches(
            "hanfile99",
            &candidates
        ));
        assert!(!BitwardenBackend::root_search_matches(
            "hanfile",
            &candidates
        ));
    }

    /// Builds an item the way `bw list items` returns one.
    fn item(id: &str, name: &str, uris: &[&str]) -> BitwardenItem {
        let uris: Vec<serde_json::Value> = uris
            .iter()
            .map(|u| serde_json::json!({ "match": null, "uri": u }))
            .collect();
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": name,
            "notes": null,
            "login": { "username": "u", "password": "p", "uris": uris },
        }))
        .expect("item json")
    }

    #[test]
    fn uri_host_strips_scheme_userinfo_port_and_path() {
        let host = BitwardenBackend::uri_host;
        assert_eq!(
            host("rdp://admin@hanfile03.corp:3389/x?y#z"),
            Some("hanfile03.corp")
        );
        assert_eq!(host("https://db.example.com"), Some("db.example.com"));
        assert_eq!(host("hanfile03"), Some("hanfile03"));
        assert_eq!(host("ssh://[2001:db8::1]:22"), Some("2001:db8::1"));
        assert_eq!(host("rustconn://"), None);
    }

    #[test]
    fn first_label_skips_ip_addresses_and_dotless_names() {
        let label = BitwardenBackend::first_label;
        assert_eq!(label("hanfile03.corp.example"), Some("hanfile03"));
        assert_eq!(label("10.0.0.1"), None);
        assert_eq!(label("hanfile03"), None);
        assert_eq!(label(".corp"), None);
    }

    /// Issue #353, the log from 2026-10-22: the search for `hanfile03`
    /// returned one item, but its title was not exactly a candidate, so the
    /// lookup said "no item found". An item reachable through its URI host or
    /// through the FQDN short name must now resolve.
    #[test]
    fn root_match_tiers_title_then_uri_host_then_short_name() {
        let candidates =
            BitwardenBackend::bare_candidates("rustconn/hanfile03", &["10.20.30.40", "hanfile03"]);

        let by_title = item("1", "HANFILE03", &[]);
        assert_eq!(
            BitwardenBackend::root_match(&by_title, &candidates),
            Some(RootMatch::Title)
        );

        let by_uri = item("2", "File server (RDP)", &["rdp://10.20.30.40:3389"]);
        assert_eq!(
            BitwardenBackend::root_match(&by_uri, &candidates),
            Some(RootMatch::UriHost)
        );

        let by_fqdn_title = item("3", "hanfile03.ame.corp", &[]);
        assert_eq!(
            BitwardenBackend::root_match(&by_fqdn_title, &candidates),
            Some(RootMatch::ShortName)
        );

        let by_fqdn_uri = item("4", "Windows file server", &["https://hanfile03.ame.corp"]);
        assert_eq!(
            BitwardenBackend::root_match(&by_fqdn_uri, &candidates),
            Some(RootMatch::ShortName)
        );

        // A dotted candidate (FQDN host) matches an entry titled by its
        // short name.
        let fqdn = BitwardenBackend::bare_candidates("k", &["hanfile03.ame.corp"]);
        assert_eq!(
            BitwardenBackend::root_match(&item("5", "hanfile03", &[]), &fqdn),
            Some(RootMatch::ShortName)
        );

        // Still no substring matching anywhere.
        for miss in [
            item("6", "hanfile03 (old)", &[]),
            item("7", "hanfile031.ame.corp", &[]),
            item("8", "x", &["https://nothanfile03.corp"]),
        ] {
            assert_eq!(
                BitwardenBackend::root_match(&miss, &candidates),
                None,
                "{}",
                miss.name
            );
        }
    }

    /// A connection whose host is a URL (a Web or SSO connection) contributes
    /// that URL's host as a candidate, right after the URL. Searching `bw` for
    /// the whole URL finds nothing (maintainer log, 2026-10-22, `aws sso`).
    #[test]
    fn a_url_host_alias_adds_its_host_as_a_candidate() {
        let candidates = BitwardenBackend::bare_candidates(
            "rustconn/aws sso",
            &["https://epam.awsapps.com/start/#/", "aws sso"],
        );
        assert_eq!(
            candidates,
            vec![
                "rustconn/aws sso",
                "https://epam.awsapps.com/start/#/",
                "epam.awsapps.com",
                "aws sso",
            ]
        );
        assert!(BitwardenBackend::root_search_matches(
            "EPAM.awsapps.com",
            &candidates
        ));

        // A plain host gains nothing extra.
        assert_eq!(
            BitwardenBackend::bare_candidates("k", &["192.168.0.122", "krdp"]),
            vec!["k", "192.168.0.122", "krdp"]
        );
    }

    /// The domain comes from a `domain` custom field on any item, but from
    /// the notes only on RustConn's own item — a root-search hit's notes are
    /// the user's free text and must never reach RDP as a domain.
    #[test]
    fn item_domain_reads_a_custom_field_and_only_trusts_own_notes() {
        let with = |name: &str, notes: Option<&str>, fields: serde_json::Value| -> BitwardenItem {
            serde_json::from_value(serde_json::json!({
                "id": "x", "name": name, "notes": notes, "login": null, "fields": fields,
            }))
            .expect("item json")
        };
        let field = serde_json::json!([{ "name": "Domain", "value": "CORP", "type": 0 }]);

        // Custom field, any item, any case.
        assert_eq!(
            BitwardenBackend::item_domain(&with("krdp", Some("free text"), field), "k"),
            Some("CORP".to_string())
        );
        // Root-search item without the field: its notes are ignored.
        assert_eq!(
            BitwardenBackend::item_domain(
                &with("krdp", Some("VPN via office"), serde_json::Value::Null),
                "k"
            ),
            None
        );
        // RustConn's own item: notes are where `store` writes the domain.
        assert_eq!(
            BitwardenBackend::item_domain(
                &with("RustConn: k", Some("CORP"), serde_json::Value::Null),
                "k"
            ),
            Some("CORP".to_string())
        );
        // An empty field falls back to the same rule as no field.
        let empty = serde_json::json!([{ "name": "domain", "value": "  " }]);
        assert_eq!(
            BitwardenBackend::item_domain(&with("krdp", Some("text"), empty), "k"),
            None
        );
    }

    /// A weaker tier wins only when it is unambiguous; a title match wins over
    /// any number of weaker ones.
    #[test]
    fn pick_root_match_prefers_strong_tiers_and_refuses_ambiguity() {
        let candidates = BitwardenBackend::bare_candidates("k", &["hanfile03"]);

        let pool = vec![
            item("a", "hanfile03.one.corp", &[]),
            item("b", "hanfile03", &[]),
        ];
        assert_eq!(
            BitwardenBackend::pick_root_match(&pool, &candidates),
            Ok(Some((1, RootMatch::Title)))
        );

        let pool = vec![
            item("a", "unrelated", &[]),
            item("b", "hanfile03.one.corp", &[]),
        ];
        assert_eq!(
            BitwardenBackend::pick_root_match(&pool, &candidates),
            Ok(Some((1, RootMatch::ShortName)))
        );

        let pool = vec![
            item("a", "hanfile03.one.corp", &[]),
            item("b", "hanfile03.two.corp", &[]),
        ];
        assert_eq!(
            BitwardenBackend::pick_root_match(&pool, &candidates),
            Err(RootMatch::ShortName)
        );

        assert_eq!(
            BitwardenBackend::pick_root_match(&[], &candidates),
            Ok(None)
        );
    }

    /// What [`select_root_item`] chose.
    #[derive(Debug, PartialEq)]
    enum Chosen {
        /// A term's results ended the search (`RootSelection::offer`).
        Hit(String, TermHit),
        /// No title matched; `pick_root_match` chose from the pool.
        Pooled(String, RootMatch),
        /// No title matched and the pool held nothing usable.
        Nothing,
        /// No title matched and the pool was ambiguous.
        Ambiguous(RootMatch),
    }

    /// Drives [`RootSelection`] over canned per-term results exactly the way
    /// `find_item_with_aliases` drives it over `bw --search` output. Returns
    /// the choice, how many searches it consumed, and the pool's ids.
    fn select_root_item(
        results_per_term: Vec<Vec<BitwardenItem>>,
        candidates: &[&str],
        own_name: &str,
    ) -> (Chosen, usize, Vec<String>) {
        assert_eq!(
            results_per_term.len(),
            candidates.len(),
            "one result per term"
        );
        let mut selection = RootSelection::new(candidates, own_name);
        for (searches, items) in results_per_term.into_iter().enumerate() {
            if let Some((item, how)) = selection.offer(items) {
                return (Chosen::Hit(item.id, how), searches + 1, Vec::new());
            }
        }
        let pool = selection.into_pool();
        let ids = pool.iter().map(|i| i.id.clone()).collect();
        let chosen = match BitwardenBackend::pick_root_match(&pool, candidates) {
            Ok(Some((pos, how))) => Chosen::Pooled(pool[pos].id.clone(), how),
            Ok(None) => Chosen::Nothing,
            Err(how) => Chosen::Ambiguous(how),
        };
        (chosen, candidates.len(), ids)
    }

    /// The first term is the connection id, whose results include RustConn's
    /// own item; that item wins over a bare-title match in the same results,
    /// as it did when a scoped search ran first.
    #[test]
    fn own_item_beats_a_bare_title_in_the_first_terms_results() {
        let candidates = BitwardenBackend::bare_candidates("krdp-id", &["krdp"]);
        assert_eq!(candidates[0], "krdp-id", "first term is the connection id");
        let own = BitwardenBackend::entry_name("krdp-id");

        let (chosen, searches, _) = select_root_item(
            vec![
                vec![item("bare", "krdp-id", &[]), item("own", &own, &[])],
                vec![item("never", "krdp", &[])],
            ],
            &candidates,
            &own,
        );

        assert_eq!(chosen, Chosen::Hit("own".into(), TermHit::Own));
        assert_eq!(searches, 1, "the own item ends the search at once");
    }

    /// Without the own item, the first exact title wins, and no later term is
    /// searched once one has.
    #[test]
    fn the_first_title_match_ends_the_search() {
        let candidates = BitwardenBackend::bare_candidates("krdp-id", &["10.0.0.5", "krdp"]);
        let own = BitwardenBackend::entry_name("krdp-id");

        let (chosen, searches, _) = select_root_item(
            vec![
                vec![item("x", "unrelated", &["rdp://10.0.0.5"])],
                vec![item("t", "10.0.0.5", &[]), item("t2", "krdp", &[])],
                vec![item("late", "krdp", &[])],
            ],
            &candidates,
            &own,
        );

        assert_eq!(chosen, Chosen::Hit("t".into(), TermHit::Title));
        assert_eq!(searches, 2);
    }

    /// The own item found by a later term (a scoped search could miss it while
    /// a `store` was still creating it) is a title match, by position.
    #[test]
    fn the_own_item_in_a_later_term_is_a_title_match() {
        let candidates = BitwardenBackend::bare_candidates("krdp-id", &["krdp"]);
        let own = BitwardenBackend::entry_name("krdp-id");

        let (chosen, _, _) = select_root_item(
            vec![vec![], vec![item("own", &own, &[])]],
            &candidates,
            &own,
        );

        assert_eq!(chosen, Chosen::Hit("own".into(), TermHit::Title));
    }

    /// With no title anywhere, every term is searched, the same item seen by
    /// two terms is pooled once, and the pool goes to `pick_root_match` — so
    /// that item is an unambiguous URI-host match rather than a tie with
    /// itself.
    #[test]
    fn untitled_results_are_deduplicated_into_the_pool() {
        let candidates = BitwardenBackend::bare_candidates("krdp-id", &["10.0.0.5", "krdp"]);
        let own = BitwardenBackend::entry_name("krdp-id");
        let by_uri = || item("a", "Office RDP", &["rdp://10.0.0.5:3389"]);

        let (chosen, searches, pool) = select_root_item(
            vec![
                vec![by_uri()],
                vec![by_uri(), item("b", "Other", &[])],
                vec![],
            ],
            &candidates,
            &own,
        );

        assert_eq!(chosen, Chosen::Pooled("a".into(), RootMatch::UriHost));
        assert_eq!(searches, 3);
        assert_eq!(pool, vec!["a".to_owned(), "b".to_owned()]);

        // Two distinct items at the same weak tier stay ambiguous.
        let (chosen, _, _) = select_root_item(
            vec![
                vec![by_uri()],
                vec![item("c", "Second RDP", &["rdp://10.0.0.5"])],
                vec![],
            ],
            &candidates,
            &own,
        );
        assert_eq!(chosen, Chosen::Ambiguous(RootMatch::UriHost));

        let (chosen, _, _) = select_root_item(vec![vec![], vec![], vec![]], &candidates, &own);
        assert_eq!(chosen, Chosen::Nothing);
    }
}
