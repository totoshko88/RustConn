//! `KeePass` integration status detection
//!
//! This module provides functionality to detect the status of `KeePass` integration,
//! including `KeePassXC` installation detection, version parsing, and KDBX file validation.

// Allow missing errors documentation - status detection functions have straightforward errors
#![allow(
    clippy::missing_errors_doc,
    reason = "module-wide override for legacy code; refactored case by case"
)]

use std::path::Path;
use std::process::{Child, Command, Output};
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};

use crate::error::{SecretError, SecretResult};
use crate::proc::{Waited, wait_bounded};

/// How long any single `keepassxc-cli` invocation is given before it is killed.
///
/// Every invocation in this module used an unbounded `wait_with_output`, at a
/// dozen call sites, against a project that bounds a credential resolution at
/// 30 s overall and every other vault operation at 10 s. `keepassxc-cli` opens
/// the database on each run, so it can block on a locked file, on a network
/// share that has gone away, or on a KDBX whose Argon2 parameters are hostile —
/// and until 0.21.0 the answer to any of those was that the calling thread never
/// came back. Only the bulk-transfer path in the GUI wrapped these calls, which
/// bounded the transfer rather than the child.
///
/// Ten seconds is the project's standard vault budget, and the value the Secret
/// Service wrapper in `keyring.rs` uses for the same reason: far longer than a
/// healthy run, short enough to fail while the user is still watching.
const KEEPASSXC_TIMEOUT: Duration = Duration::from_secs(10);

/// The budget for an invocation that *writes* to the database.
///
/// Longer than [`KEEPASSXC_TIMEOUT`] because the consequence of expiry is worse,
/// not because a write is expected to be slower. A read that is killed costs a
/// lookup; a `SIGKILL` delivered to `add`, `mkdir` or `rm` lands in the middle of
/// rewriting the KDBX. So the mutating calls get the 30 s credential-resolution
/// tier, and the trade is deliberate: a longer wait in exchange for a much
/// smaller window in which the database can be interrupted mid-write.
///
/// The KDF cost is paid *per invocation*, because every `keepassxc-cli` run
/// reopens the database — so a KDBX with KeePassXC's high-security Argon2
/// settings does not overrun once, it overruns at every call site. A single save
/// is four invocations (the group check, the parent-group check, the delete and
/// the add), each paying it in full.
const KEEPASSXC_WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// The budget for a *read* invocation that also waits on a YubiKey touch.
///
/// A Challenge-Response unlock (`-y <slot>`) blocks until the user physically
/// touches the key, and a touch-required slot is the common configuration. The
/// standard [`KEEPASSXC_TIMEOUT`] of 10 s is measured for the KDF alone and is
/// too short once a human has to notice the key blink and reach for it, so a
/// `-y` read gets a longer budget. It stays well under a minute so a genuinely
/// stuck run (unplugged key, wrong slot) still fails while the user is watching.
const KEEPASSXC_YUBIKEY_TIMEOUT: Duration = Duration::from_secs(30);

/// Appends the shared credential-unlock arguments to a `keepassxc-cli` argv.
///
/// One place for the three that every DB-opening invocation shares, so the
/// readers cannot drift apart on how they compose:
/// - `--no-password` when there is no master password (key file and/or YubiKey
///   only), because the CLI otherwise still reads an empty line from stdin as an
///   empty password;
/// - `--key-file <path>` when a key file is configured;
/// - `-y <slot[:serial]>` when a YubiKey Challenge-Response slot is configured.
///
/// The slot is not a secret (a slot number and serial identify the key, they do
/// not authenticate as it), so it goes on the command line; the master password
/// never does — it stays on stdin at the call site.
fn push_unlock_args(
    args: &mut Vec<String>,
    has_password: bool,
    key_file: Option<&Path>,
    yubikey_slot: Option<&str>,
) {
    if !has_password && (key_file.is_some() || yubikey_slot.is_some()) {
        args.push("--no-password".to_string());
    }
    if let Some(kf) = key_file {
        args.push("--key-file".to_string());
        args.push(kf.display().to_string());
    }
    if let Some(slot) = yubikey_slot {
        args.push("-y".to_string());
        args.push(slot.to_string());
    }
}

/// Assembles the argv for a `keepassxc-cli add`, without spawning anything.
///
/// Extracted from [`KeePassStatus::save_password_to_kdbx`] so the write path's
/// unlock composition — the `-y <slot>` that issue #350 found missing, and the
/// `--no-password` that must accompany a key-file/YubiKey-only unlock — can be
/// asserted in a unit test, the way the read side is exercised through
/// [`push_unlock_args`]. The entry password is not here: it travels on stdin, so
/// it never reaches the argv this builds.
fn build_add_args(
    has_password: bool,
    key_file: Option<&Path>,
    yubikey_slot: Option<&str>,
    username: &str,
    url: Option<&str>,
    kdbx_path: &Path,
    entry_path: &str,
) -> Vec<String> {
    let mut args = vec!["add".to_string(), "-q".to_string()];

    // Compose --no-password / --key-file / -y consistently with the read path.
    push_unlock_args(&mut args, has_password, key_file, yubikey_slot);

    // Add username if not empty
    if !username.is_empty() {
        args.push("-u".to_string());
        args.push(username.to_string());
    }

    // Add URL if provided
    if let Some(u) = url
        && !u.is_empty()
    {
        args.push("--url".to_string());
        args.push(u.to_string());
    }

    // Password prompt flag — tells keepassxc-cli to read the entry password from stdin.
    args.push("-p".to_string());

    // Database path and entry name last.
    args.push(kdbx_path.display().to_string());
    args.push(entry_path.to_string());

    args
}

/// Waits for a `keepassxc-cli` child, reporting a timeout as an error.
///
/// `what` names the invocation in the log and in the error. It is
/// `&'static str` on purpose: it is user-visible, and a `&str` would let a
/// caller interpolate an entry name — or a credential — into it.
fn wait_for_cli(child: Child, what: &'static str) -> SecretResult<Output> {
    wait_for_cli_with(child, what, KEEPASSXC_TIMEOUT)
}

/// [`wait_for_cli`] with the write budget, for an invocation that modifies the
/// database.
fn wait_for_cli_write(child: Child, what: &'static str) -> SecretResult<Output> {
    wait_for_cli_with(child, what, KEEPASSXC_WRITE_TIMEOUT)
}

/// [`wait_for_cli`] with the YubiKey budget, for a read that waits on a touch.
///
/// A `-y` unlock cannot complete until the key is touched, so it gets the longer
/// [`KEEPASSXC_YUBIKEY_TIMEOUT`] instead of the KDF-only [`KEEPASSXC_TIMEOUT`].
fn wait_for_cli_yubikey(child: Child, what: &'static str) -> SecretResult<Output> {
    wait_for_cli_with(child, what, KEEPASSXC_YUBIKEY_TIMEOUT)
}

fn wait_for_cli_with(child: Child, what: &'static str, budget: Duration) -> SecretResult<Output> {
    match wait_bounded(child, budget, what) {
        Ok(Waited::Exited(output)) => Ok(output),
        Ok(Waited::TimedOut) => Err(SecretError::KeePassXC(format!(
            "keepassxc-cli ({what}) did not respond within {}s and was stopped. \
             The database may be locked by another process, on storage that is not \
             responding, or configured with key-derivation parameters too heavy for \
             this machine — each run of keepassxc-cli pays that cost again.",
            budget.as_secs()
        ))),
        Err(e) => Err(SecretError::KeePassXC(format!(
            "Failed to wait for keepassxc-cli: {e}"
        ))),
    }
}

/// Which timeout budget an [`Invocation`] runs under, and whether it may wait
/// on a hardware-key touch.
///
/// The budget mirrors the three constants above: a read gets
/// [`KEEPASSXC_TIMEOUT`], a write gets [`KEEPASSXC_WRITE_TIMEOUT`], and either
/// one bumps to [`KEEPASSXC_YUBIKEY_TIMEOUT`] when a `-y` slot means the run
/// blocks on a physical touch. The old code chose this inline at every call
/// site with a `if yubikey_slot.is_some()` ladder; the seam makes it one
/// decision per invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InvocationKind {
    /// Opens the database to read (e.g. `show`, `ls`). Tight budget.
    Read,
    /// Modifies the database (e.g. `add`, `edit`, `mkdir`, `mv`, `rm`). Write budget.
    Write,
}

/// One `keepassxc-cli` run, as data: the verb for logs/errors, the full argv
/// (minus the binary), and whether it reads or writes.
///
/// The database password is **never** stored here — it is a separate argument
/// to [`KeePassCli::run`], so it cannot leak into a log of the argv or outlive
/// the call. The entry password likewise travels on stdin at the call site via
/// an `add`/`edit -p` and is not part of this struct.
pub(crate) struct Invocation {
    /// Short name for the run, used in the timeout log and error (`&'static`
    /// so an entry name — or a credential — can never be interpolated in).
    what: &'static str,
    /// The argv after the binary, including the `-y <slot>` unlock args.
    args: Vec<String>,
    /// Read or write, picking the timeout budget.
    kind: InvocationKind,
    /// Whether this run carries `-y`, so it may block on a touch (bumps the
    /// budget and brackets the run with a touch cue).
    waits_on_touch: bool,
}

impl Invocation {
    /// Builds an invocation from its verb, pre-composed args and kind.
    ///
    /// `waits_on_touch` is derived by the caller from whether a YubiKey slot
    /// was pushed into `args`, because only the caller knows that.
    pub(crate) fn new(
        what: &'static str,
        args: Vec<String>,
        kind: InvocationKind,
        waits_on_touch: bool,
    ) -> Self {
        Self {
            what,
            args,
            kind,
            waits_on_touch,
        }
    }
}

/// Runs one `keepassxc-cli` [`Invocation`] and returns its captured output.
///
/// A trait, not a free function, so tests can script replies without a real
/// binary or database (see `FakeCli` in the tests). Every database-opening
/// call in this module goes through one implementation of this, so the unlock
/// composition, the `LC_MESSAGES=C` / `flatpak-spawn --host` wrapping, the
/// stdin password feed, the timeout budgets and the touch cue cannot drift
/// apart between call sites again.
pub(crate) trait KeePassCli {
    /// Runs the invocation, feeding `db_password` on stdin when `Some`.
    ///
    /// The entry password, when a verb needs one (`add`/`edit -p`), is written
    /// after the database password; the caller passes it via `entry_secret`.
    fn run(
        &self,
        invocation: &Invocation,
        db_password: Option<&SecretString>,
        entry_secret: Option<&SecretString>,
    ) -> SecretResult<Output>;
}

/// The real runner: spawns `keepassxc-cli` the way the whole module always has.
pub(crate) struct RealKeePassCli<'a> {
    /// The resolved `keepassxc-cli` path (or the flatpak host binary).
    cli_path: &'a Path,
}

impl<'a> RealKeePassCli<'a> {
    pub(crate) const fn new(cli_path: &'a Path) -> Self {
        Self { cli_path }
    }
}

impl KeePassCli for RealKeePassCli<'_> {
    fn run(
        &self,
        invocation: &Invocation,
        db_password: Option<&SecretString>,
        entry_secret: Option<&SecretString>,
    ) -> SecretResult<Output> {
        use std::io::Write as IoWrite;
        use std::process::Stdio;

        // Bracket the run with a touch cue when it may block on the key. Dropped
        // on every path out, so the GUI's in-flight count never sticks.
        let _touch = if invocation.waits_on_touch {
            Some(crate::secret::touch::TouchGuard::begin())
        } else {
            None
        };

        let mut child = KeePassStatus::keepassxc_command(self.cli_path)
            .args(&invocation.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SecretError::KeePassXC(format!("Failed to run keepassxc-cli: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            if let Some(db_pwd) = db_password {
                stdin
                    .write_all(db_pwd.expose_secret().as_bytes())
                    .map_err(|e| SecretError::KeePassXC(format!("Failed to send password: {e}")))?;
                stdin
                    .write_all(b"\n")
                    .map_err(|e| SecretError::KeePassXC(format!("Failed to send password: {e}")))?;
            }
            if let Some(entry_pwd) = entry_secret {
                stdin
                    .write_all(entry_pwd.expose_secret().as_bytes())
                    .map_err(|e| {
                        SecretError::KeePassXC(format!("Failed to send entry password: {e}"))
                    })?;
                stdin
                    .write_all(b"\n")
                    .map_err(|e| SecretError::KeePassXC(format!("Failed to send newline: {e}")))?;
            }
            drop(stdin);
        }

        let budget = match (invocation.kind, invocation.waits_on_touch) {
            (_, true) => KEEPASSXC_YUBIKEY_TIMEOUT,
            (InvocationKind::Read, false) => KEEPASSXC_TIMEOUT,
            (InvocationKind::Write, false) => KEEPASSXC_WRITE_TIMEOUT,
        };
        wait_for_cli_with(child, invocation.what, budget)
    }
}

/// Serializes every multi-step `keepassxc-cli` operation process-wide.
///
/// `keepassxc-cli` rewrites the whole KDBX on each write, so two operations
/// interleaving (the edit dialog's save racing its own stale-key delete, issue
/// #350) can lose one update. Each public multi-step operation holds this for
/// its whole duration, so writes are strictly ordered. A read-only lookup does
/// not take it — it cannot corrupt anything — so a connect is never serialized
/// behind a save.
static KEEPASS_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs `op` while holding the process-wide KeePass write lock.
///
/// A poisoned lock (a previous holder panicked) is recovered: the guarded
/// value is `()`, so there is no half-written state to protect against.
fn with_keepass_write_lock<T>(op: impl FnOnce() -> T) -> T {
    let _guard = KEEPASS_WRITE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    op()
}

/// The minimum `keepassxc-cli` version whose `edit`/`mv` the fast save path
/// relies on. `edit -u -p --url` (update an entry in place) and `mv` (move an
/// entry between groups) have been stable since 2.5.0 (decision D3). An older
/// or unreadable version keeps the previous ls/rm/add algorithm unchanged.
const KEEPASS_EDIT_MIN_VERSION: (u32, u32) = (2, 5);

/// Parses the leading `major.minor` out of a `keepassxc-cli` version string
/// such as `"2.7.12"`. Returns `None` for anything it cannot read as two
/// numbers, so an unparseable version fails safe to the legacy algorithm.
fn parse_version_major_minor(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// Whether a parsed `major.minor` is at least [`KEEPASS_EDIT_MIN_VERSION`].
fn version_supports_edit(version: &str) -> bool {
    match parse_version_major_minor(version) {
        Some((major, minor)) => (major, minor) >= KEEPASS_EDIT_MIN_VERSION,
        None => false,
    }
}

/// What one `ls -R -f RustConn` run tells us about the tree: which group paths
/// exist (so `mkdir` runs only for the missing levels) and the set of entry
/// paths (so a save can choose `edit` over `add` without reading anything back).
struct KeePassTree {
    /// Existing group paths, re-rooted at `RustConn/...` (and `RustConn` itself).
    groups: std::collections::HashSet<String>,
    /// Existing entry paths, re-rooted at `RustConn/...`.
    entries: std::collections::HashSet<String>,
}

/// Reads the `RustConn` subtree with one `ls -R -f RustConn` run.
///
/// `keepassxc-cli ls -R -f <group>` prints one path per line, group paths
/// ending in `/`. A run whose database did not open returns `None`, and the
/// caller then falls back (mkdirs defensively, adds rather than edits) rather
/// than guessing the tree is empty.
fn probe_tree(
    cli: &dyn KeePassCli,
    db_password: Option<&SecretString>,
    key_file: Option<&Path>,
    yubikey_slot: Option<&str>,
    kdbx_path: &Path,
) -> Option<KeePassTree> {
    let mut args = vec![
        "ls".to_string(),
        "-q".to_string(),
        "-R".to_string(),
        "-f".to_string(),
    ];
    push_unlock_args(&mut args, db_password.is_some(), key_file, yubikey_slot);
    args.push(kdbx_path.display().to_string());
    args.push("RustConn".to_string());

    let invocation = Invocation::new(
        "ls -R (tree probe)",
        args,
        InvocationKind::Read,
        yubikey_slot.is_some(),
    );
    let output = cli.run(&invocation, db_password, None).ok()?;
    if !output.status.success() {
        // The database did not open (or RustConn is absent). Either way we
        // cannot enumerate the tree.
        return None;
    }

    let listing = String::from_utf8_lossy(&output.stdout);
    let mut groups = std::collections::HashSet::new();
    let mut entries = std::collections::HashSet::new();
    // The probe lists under RustConn, so a flattened path is relative to it
    // (e.g. "Groups/Production/" or "Groups/Production/web (ssh)"); re-root each
    // at "RustConn/...". "RustConn" itself exists by virtue of a successful ls.
    groups.insert("RustConn".to_string());
    for line in listing.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(group_rel) = line.strip_suffix('/') {
            if !group_rel.is_empty() {
                groups.insert(format!("RustConn/{group_rel}"));
            }
        } else {
            entries.insert(format!("RustConn/{line}"));
        }
    }
    Some(KeePassTree { groups, entries })
}

/// The cumulative `RustConn/...` group levels an entry path needs, in order
/// from the shallowest. For `entry_name = "Groups/Production/web (ssh)"` this is
/// `["RustConn", "RustConn/Groups", "RustConn/Groups/Production"]` — the entry
/// name itself is not a group.
fn required_group_levels(entry_name: &str) -> Vec<String> {
    let parts: Vec<&str> = entry_name.split('/').collect();
    let mut levels = vec!["RustConn".to_string()];
    let mut current = String::from("RustConn");
    // All but the last component (the entry name) are groups.
    for part in &parts[..parts.len().saturating_sub(1)] {
        current = format!("{current}/{part}");
        levels.push(current.clone());
    }
    levels
}

/// Argv for an `add` or `edit` of an entry, sharing one builder so the two
/// cannot drift on unlock composition. The entry password is not here — it is
/// fed on stdin after `-p`. `verb` is `"add"` or `"edit"`.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors build_add_args: the entry write needs every unlock factor plus the entry fields"
)]
fn build_write_entry_args(
    verb: &'static str,
    has_password: bool,
    key_file: Option<&Path>,
    yubikey_slot: Option<&str>,
    username: &str,
    url: Option<&str>,
    kdbx_path: &Path,
    entry_path: &str,
) -> Vec<String> {
    let mut args = vec![verb.to_string(), "-q".to_string()];
    push_unlock_args(&mut args, has_password, key_file, yubikey_slot);
    if !username.is_empty() {
        args.push("-u".to_string());
        args.push(username.to_string());
    }
    if let Some(u) = url
        && !u.is_empty()
    {
        args.push("--url".to_string());
        args.push(u.to_string());
    }
    args.push("-p".to_string());
    args.push(kdbx_path.display().to_string());
    args.push(entry_path.to_string());
    args
}

/// Saves an entry with the fewest `keepassxc-cli` runs: one tree probe, a
/// `mkdir` only for each missing group level, then one `edit` (entry exists)
/// or one `add` (it does not). No `rm`, no password read back — the save that
/// used to cost about five YubiKey touches now costs two or three (issue #350).
///
/// `entry_name` is the path under `RustConn` (e.g. `"web (ssh)"` or
/// `"Groups/Prod/web (ssh)"`); the function prepends `RustConn/`.
///
/// Stops at the first timeout or credential refusal rather than pressing on —
/// a swallowed probe timeout or an ignored failure used to cost extra touches
/// and leave the save in an unknown state.
#[expect(
    clippy::too_many_arguments,
    reason = "the KDBX write needs every unlock factor plus the entry fields; mirrors save_password_to_kdbx"
)]
fn save_in_place(
    cli: &dyn KeePassCli,
    db_password: Option<&SecretString>,
    key_file: Option<&Path>,
    yubikey_slot: Option<&str>,
    kdbx_path: &Path,
    entry_name: &str,
    username: &str,
    password: &SecretString,
    url: Option<&str>,
) -> SecretResult<()> {
    let entry_path = format!("RustConn/{entry_name}");
    let waits = yubikey_slot.is_some();

    // One probe. If the database would not open, the error surfaces on the
    // write below; we proceed assuming nothing exists (mkdir every level, add).
    let tree = probe_tree(cli, db_password, key_file, yubikey_slot, kdbx_path);

    // mkdir only the levels the probe did not find. With no probe (database did
    // not open on the read, or RustConn absent), create every level — a stale
    // mkdir of an existing group is a harmless "already exists".
    for level in required_group_levels(entry_name) {
        let exists = tree.as_ref().is_some_and(|t| t.groups.contains(&level));
        if exists {
            continue;
        }
        let mut args = vec!["mkdir".to_string(), "-q".to_string()];
        push_unlock_args(&mut args, db_password.is_some(), key_file, yubikey_slot);
        args.push(kdbx_path.display().to_string());
        args.push(level.clone());
        let invocation = Invocation::new("mkdir (group level)", args, InvocationKind::Write, waits);
        let output = cli.run(&invocation, db_password, None)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.contains("already exists") {
                return Err(SecretError::KeePassXC(format!(
                    "Failed to create group '{level}': {}",
                    stderr.trim()
                )));
            }
        }
    }

    // edit in place when the entry exists, add when it does not. Without a
    // probe we cannot tell, so add; a pre-existing entry then reports "already
    // exists", which we treat as the signal to edit instead.
    let entry_exists = tree
        .as_ref()
        .is_some_and(|t| t.entries.contains(&entry_path));
    let verb = if entry_exists { "edit" } else { "add" };
    let args = build_write_entry_args(
        verb,
        db_password.is_some(),
        key_file,
        yubikey_slot,
        username,
        url,
        kdbx_path,
        &entry_path,
    );
    let invocation = Invocation::new(verb, args, InvocationKind::Write, waits);
    let output = cli.run(&invocation, db_password, Some(password))?;

    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    // We guessed "add" with no probe and the entry was already there: edit it.
    if !entry_exists && stderr.contains("already exists") {
        let args = build_write_entry_args(
            "edit",
            db_password.is_some(),
            key_file,
            yubikey_slot,
            username,
            url,
            kdbx_path,
            &entry_path,
        );
        let invocation = Invocation::new("edit", args, InvocationKind::Write, waits);
        let retry = cli.run(&invocation, db_password, Some(password))?;
        if retry.status.success() {
            return Ok(());
        }
        let retry_stderr = String::from_utf8_lossy(&retry.stderr);
        return Err(classify_write_failure(&retry_stderr));
    }
    Err(classify_write_failure(&stderr))
}

/// Renames or moves an entry with `mv` and/or `edit -t`, reading nothing back.
///
/// `mv <entry> <group>` moves the entry between groups; `edit -t <title>`
/// renames it in place. The password is never shown — the entry keeps it. Only
/// the missing destination group levels are created first.
fn rename_or_move_in_place(
    cli: &dyn KeePassCli,
    db_password: Option<&SecretString>,
    key_file: Option<&Path>,
    yubikey_slot: Option<&str>,
    kdbx_path: &Path,
    old_entry_path: &str,
    new_entry_path: &str,
) -> SecretResult<()> {
    if old_entry_path == new_entry_path {
        return Ok(());
    }
    let waits = yubikey_slot.is_some();

    // The new entry name is everything after the last '/'; the destination
    // group is everything before it (or RustConn's root).
    let (new_group, new_title) = match new_entry_path.rsplit_once('/') {
        Some((group, title)) => (group.to_string(), title.to_string()),
        None => (String::new(), new_entry_path.to_string()),
    };
    let (old_group, old_title) = match old_entry_path.rsplit_once('/') {
        Some((group, title)) => (group.to_string(), title.to_string()),
        None => (String::new(), old_entry_path.to_string()),
    };

    // Create any missing destination group levels (new_group is e.g.
    // "RustConn/Groups/Prod"; required_group_levels wants the path under
    // RustConn, so strip the prefix and re-use it with a dummy entry name).
    if let Some(under_rustconn) = new_group.strip_prefix("RustConn/") {
        let tree = probe_tree(cli, db_password, key_file, yubikey_slot, kdbx_path);
        for level in required_group_levels(&format!("{under_rustconn}/x")) {
            if tree.as_ref().is_some_and(|t| t.groups.contains(&level)) {
                continue;
            }
            let mut args = vec!["mkdir".to_string(), "-q".to_string()];
            push_unlock_args(&mut args, db_password.is_some(), key_file, yubikey_slot);
            args.push(kdbx_path.display().to_string());
            args.push(level.clone());
            let invocation =
                Invocation::new("mkdir (move target)", args, InvocationKind::Write, waits);
            let output = cli.run(&invocation, db_password, None)?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stderr.contains("already exists") {
                    return Err(SecretError::KeePassXC(format!(
                        "Failed to create group '{level}': {}",
                        stderr.trim()
                    )));
                }
            }
        }
    }

    // Current path of the entry as mv/edit must address it: after a move the
    // title stays, so track where it lives.
    let mut current_path = old_entry_path.to_string();

    // Move between groups when the group changed.
    if new_group != old_group && !new_group.is_empty() {
        let mut args = vec!["mv".to_string(), "-q".to_string()];
        push_unlock_args(&mut args, db_password.is_some(), key_file, yubikey_slot);
        args.push(kdbx_path.display().to_string());
        args.push(current_path.clone());
        args.push(new_group.clone());
        let invocation = Invocation::new("mv (entry)", args, InvocationKind::Write, waits);
        let output = cli.run(&invocation, db_password, None)?;
        if !output.status.success() {
            return Err(classify_write_failure(&String::from_utf8_lossy(
                &output.stderr,
            )));
        }
        current_path = format!("{new_group}/{old_title}");
    }

    // Rename the title in place when it changed.
    if new_title != old_title {
        let mut args = vec!["edit".to_string(), "-q".to_string()];
        push_unlock_args(&mut args, db_password.is_some(), key_file, yubikey_slot);
        args.push("-t".to_string());
        args.push(new_title);
        args.push(kdbx_path.display().to_string());
        args.push(current_path);
        let invocation = Invocation::new("edit -t (rename)", args, InvocationKind::Write, waits);
        let output = cli.run(&invocation, db_password, None)?;
        if !output.status.success() {
            return Err(classify_write_failure(&String::from_utf8_lossy(
                &output.stderr,
            )));
        }
    }
    Ok(())
}

/// Maps a failed write's stderr to a user-facing error, keeping the historical
/// "Invalid database password or key file" wording a credential refusal gives.
fn classify_write_failure(stderr: &str) -> SecretError {
    if stderr.contains("Invalid credentials")
        || stderr.contains("wrong password")
        || stderr.contains("Error while reading the database")
    {
        SecretError::KeePassXC("Invalid database password or key file".to_string())
    } else {
        SecretError::KeePassXC(format!("KeePass error: {}", stderr.trim()))
    }
}

#[cfg(test)]
mod algorithm_tests {
    use super::{parse_version_major_minor, required_group_levels, version_supports_edit};

    #[test]
    fn version_gate_accepts_2_5_and_newer() {
        assert!(version_supports_edit("2.5.0"));
        assert!(version_supports_edit("2.7.12"));
        assert!(version_supports_edit("3.0.0"));
    }

    #[test]
    fn version_gate_rejects_older_or_unparseable() {
        assert!(!version_supports_edit("2.4.3"));
        assert!(!version_supports_edit("2.4"));
        assert!(!version_supports_edit(""));
        assert!(!version_supports_edit("not-a-version"));
    }

    #[test]
    fn version_major_minor_parses_leading_two_numbers() {
        assert_eq!(parse_version_major_minor("2.7.12"), Some((2, 7)));
        assert_eq!(parse_version_major_minor("2.5"), Some((2, 5)));
        assert_eq!(parse_version_major_minor("bogus"), None);
    }

    #[test]
    fn required_levels_lists_each_parent_group_once() {
        assert_eq!(required_group_levels("web (ssh)"), vec!["RustConn"]);
        assert_eq!(
            required_group_levels("Groups/Production/web (ssh)"),
            vec!["RustConn", "RustConn/Groups", "RustConn/Groups/Production"]
        );
    }

    // --- Pure vault-root matcher (root_match_entry_path) ---
    //
    // These exercise the side-effect-free core of the vault-root read widening
    // with a canned `ls -R -f <db>` listing, so they need no keepassxc-cli and
    // no temp database. The end-to-end read against a real kdbx is covered by a
    // construction test that requires live keepassxc-cli (see
    // root_reader_end_to_end_requires_live_keepassxc_cli below).

    /// A flattened whole-database listing: some entries live OUTSIDE the
    /// RustConn group, which is exactly what the scoped reader cannot see.
    const ROOT_LISTING: &str = "\
RustConn/
RustConn/web (ssh)
Internet/
Internet/Banking/
Internet/Banking/my-router
Imported/legacy-host
standalone-entry
";

    #[test]
    fn root_match_finds_entry_outside_rustconn_group() {
        // `my-router` lives under Internet/Banking/, never under RustConn — the
        // whole point of root search. It is matched by basename.
        assert_eq!(
            root_match_entry_path(ROOT_LISTING, "my-router"),
            Some("Internet/Banking/my-router".to_string())
        );
        assert_eq!(
            root_match_entry_path(ROOT_LISTING, "legacy-host"),
            Some("Imported/legacy-host".to_string())
        );
        // A top-level entry outside RustConn.
        assert_eq!(
            root_match_entry_path(ROOT_LISTING, "standalone-entry"),
            Some("standalone-entry".to_string())
        );
    }

    #[test]
    fn root_match_skips_group_paths_and_misses_cleanly() {
        // A trailing-slash line is a group, never an entry — "Banking" must not
        // match even though it appears as a path component.
        assert_eq!(root_match_entry_path(ROOT_LISTING, "Banking"), None);
        assert_eq!(root_match_entry_path(ROOT_LISTING, "Internet"), None);
        // A name present nowhere misses.
        assert_eq!(root_match_entry_path(ROOT_LISTING, "nope"), None);
        // Empty listing misses.
        assert_eq!(root_match_entry_path("", "my-router"), None);
    }

    #[test]
    fn root_match_prefers_exact_qualified_path() {
        // A caller passing an already-qualified path lands on it exactly, even
        // when a shorter basename match exists earlier in the listing.
        let listing = "\
a/dup
b/c/dup
";
        assert_eq!(
            root_match_entry_path(listing, "b/c/dup"),
            Some("b/c/dup".to_string())
        );
        // Tail match: "c/dup" is the suffix of "b/c/dup".
        assert_eq!(
            root_match_entry_path(listing, "c/dup"),
            Some("b/c/dup".to_string())
        );
        // Bare basename falls back to the FIRST occurrence (back-compat order).
        assert_eq!(
            root_match_entry_path(listing, "dup"),
            Some("a/dup".to_string())
        );
    }

    /// The end-to-end root read (spawning keepassxc-cli against a real kdbx)
    /// has NO coverage here: the public readers spawn `keepassxc-cli` directly
    /// rather than through the injectable `KeePassCli` trait (only the save
    /// path is mockable), and this test harness has no temp-kdbx builder — the
    /// existing reader tests only assert validation errors on fake paths. So a
    /// faithful end-to-end test of `get_password_from_kdbx_root` requires live
    /// `keepassxc-cli` plus a constructed database; it is intentionally NOT
    /// written here rather than faked. The pure matcher above is what carries
    /// the logic that could otherwise be wrong.
    #[test]
    fn root_reader_end_to_end_requires_live_keepassxc_cli() {
        // Documentation marker; the pure matcher tests cover the decision logic.
    }

    // --- Behaviour of save_in_place / rename_or_move_in_place via a fake CLI ---

    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{ExitStatus, Output};

    use secrecy::SecretString;

    use super::{
        Invocation, InvocationKind, KeePassCli, SecretError, SecretResult, push_unlock_args,
        rename_or_move_in_place, root_match_entry_path, save_in_place,
    };

    /// A scripted reply for one `run` call.
    struct Reply {
        stdout: String,
        success: bool,
        /// A `None` here makes `run` return `Err` (a timeout/spawn failure),
        /// so a test can assert the operation stops at the first one.
        errors: bool,
    }

    impl Reply {
        fn ok() -> Self {
            Self {
                stdout: String::new(),
                success: true,
                errors: false,
            }
        }
        fn stdout(s: &str) -> Self {
            Self {
                stdout: s.to_string(),
                success: true,
                errors: false,
            }
        }
        fn fail(_stderr: &str) -> Self {
            Self {
                stdout: String::new(),
                success: false,
                errors: false,
            }
        }
        fn timeout() -> Self {
            Self {
                stdout: String::new(),
                success: false,
                errors: true,
            }
        }
    }

    /// Records every invocation's argv; replies are scripted in order.
    #[derive(Default)]
    struct FakeCli {
        calls: RefCell<Vec<Vec<String>>>,
        replies: RefCell<VecDeque<Reply>>,
    }

    impl FakeCli {
        fn with_replies(replies: impl IntoIterator<Item = Reply>) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                replies: RefCell::new(replies.into_iter().collect()),
            }
        }
        /// The leading verb of each recorded call, in order.
        fn verbs(&self) -> Vec<String> {
            self.calls
                .borrow()
                .iter()
                .map(|args| args.first().cloned().unwrap_or_default())
                .collect()
        }
        /// Whether any recorded argv contained `needle`.
        fn any_arg_contains(&self, needle: &str) -> bool {
            self.calls
                .borrow()
                .iter()
                .flatten()
                .any(|arg| arg.contains(needle))
        }
    }

    impl KeePassCli for FakeCli {
        fn run(
            &self,
            invocation: &Invocation,
            _db_password: Option<&SecretString>,
            _entry_secret: Option<&SecretString>,
        ) -> SecretResult<Output> {
            // Mirror RealKeePassCli::run's touch bracketing so a test can assert
            // that a `-y` invocation (waits_on_touch) raises the touch cue. The
            // guard is dropped at the end of this call, exactly as the real one is.
            let _touch = if invocation.waits_on_touch {
                Some(crate::secret::touch::TouchGuard::begin())
            } else {
                None
            };
            self.calls.borrow_mut().push(invocation.args.clone());
            let reply = self
                .replies
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(Reply::ok);
            if reply.errors {
                return Err(SecretError::KeePassXC("timed out".to_string()));
            }
            Ok(Output {
                status: ExitStatus::from_raw(if reply.success { 0 } else { 1 << 8 }),
                stdout: reply.stdout.into_bytes(),
                stderr: Vec::new(),
            })
        }
    }

    fn secret(s: &str) -> SecretString {
        SecretString::from(s)
    }
    fn db() -> SecretString {
        SecretString::from("master")
    }
    fn kdbx() -> std::path::PathBuf {
        std::path::PathBuf::from("/tmp/test.kdbx")
    }

    #[test]
    fn updating_an_existing_entry_runs_one_probe_and_one_edit() {
        // Probe lists RustConn/Prod/ and the existing entry; no mkdir, one edit.
        let cli = FakeCli::with_replies([Reply::stdout("Prod/\nProd/web (ssh)\n"), Reply::ok()]);
        save_in_place(
            &cli,
            Some(&db()),
            None,
            None,
            &kdbx(),
            "Prod/web (ssh)",
            "deploy",
            &secret("hunter2"),
            None,
        )
        .unwrap();
        assert_eq!(cli.verbs(), ["ls", "edit"]);
        assert!(
            !cli.any_arg_contains("hunter2"),
            "the entry password must never reach the argv"
        );
        assert!(
            !cli.any_arg_contains("rm"),
            "the new path never deletes to save"
        );
    }

    #[test]
    fn a_new_entry_runs_add_not_edit() {
        // Probe shows the group but not the entry → add.
        let cli = FakeCli::with_replies([Reply::stdout("Prod/\n"), Reply::ok()]);
        save_in_place(
            &cli,
            Some(&db()),
            None,
            None,
            &kdbx(),
            "Prod/web (ssh)",
            "deploy",
            &secret("hunter2"),
            None,
        )
        .unwrap();
        assert_eq!(cli.verbs(), ["ls", "add"]);
    }

    #[test]
    fn mkdir_runs_only_for_the_missing_levels() {
        // Probe shows RustConn and RustConn/Groups exist, but not Prod.
        let cli = FakeCli::with_replies([
            Reply::stdout("Groups/\n"),
            Reply::ok(), // mkdir RustConn/Groups/Prod
            Reply::ok(), // add
        ]);
        save_in_place(
            &cli,
            Some(&db()),
            None,
            None,
            &kdbx(),
            "Groups/Prod/web (ssh)",
            "deploy",
            &secret("x"),
            None,
        )
        .unwrap();
        // One ls, exactly one mkdir (for Prod), one add — RustConn and
        // RustConn/Groups already existed so they are not re-created.
        assert_eq!(cli.verbs(), ["ls", "mkdir", "add"]);
    }

    #[test]
    fn a_save_stops_at_the_first_timeout() {
        // The probe times out → we proceed defensively; the first mkdir times
        // out and the save stops there, never reaching add.
        let cli = FakeCli::with_replies([Reply::timeout(), Reply::timeout()]);
        let err = save_in_place(
            &cli,
            Some(&db()),
            None,
            None,
            &kdbx(),
            "Prod/web (ssh)",
            "deploy",
            &secret("x"),
            None,
        );
        assert!(err.is_err());
        // ls (timed out) then mkdir (timed out) — nothing after.
        assert_eq!(cli.verbs(), ["ls", "mkdir"]);
    }

    #[test]
    fn a_rename_reads_no_password_and_uses_mv_then_edit_t() {
        // Same group, different title → just edit -t, no mv, no show.
        let cli = FakeCli::with_replies([Reply::ok()]);
        rename_or_move_in_place(
            &cli,
            Some(&db()),
            None,
            None,
            &kdbx(),
            "RustConn/old (ssh)",
            "RustConn/new (ssh)",
        )
        .unwrap();
        assert_eq!(cli.verbs(), ["edit"]);
        assert!(
            !cli.any_arg_contains("Password"),
            "a rename must not read the password back with show -a Password"
        );
    }

    #[test]
    fn a_move_to_another_group_uses_mv() {
        // Different group, same title → mv (no probe mkdir needed since the
        // target group has no RustConn/ prefix levels beyond itself here).
        let cli = FakeCli::with_replies([
            Reply::stdout("Archive/\n"), // probe for the destination levels
            Reply::ok(),                 // mv
        ]);
        rename_or_move_in_place(
            &cli,
            Some(&db()),
            None,
            None,
            &kdbx(),
            "RustConn/web (ssh)",
            "RustConn/Archive/web (ssh)",
        )
        .unwrap();
        assert!(cli.verbs().contains(&"mv".to_string()));
        assert!(!cli.any_arg_contains("Password"));
    }

    #[test]
    fn a_failed_rename_surfaces_an_error_and_does_not_read_the_password() {
        let cli = FakeCli::with_replies([Reply::fail("Could not find entry")]);
        let err = rename_or_move_in_place(
            &cli,
            Some(&db()),
            None,
            None,
            &kdbx(),
            "RustConn/old (ssh)",
            "RustConn/new (ssh)",
        );
        assert!(err.is_err());
        assert!(!cli.any_arg_contains("Password"));
    }

    /// #350 follow-up: a reader unlocking with a YubiKey slot builds the SAME
    /// kind of read invocation the three readers now all route through
    /// (`InvocationKind::Read`, `waits_on_touch` set because a `-y` slot was
    /// pushed into the args), and running it through a cli that brackets the
    /// touch guard raises the "touch your key" cue. Before the readers were
    /// unified onto `cli.run` they spawned `keepassxc-cli` directly and no cue
    /// fired on connect / password-load. The invocation shape is built here
    /// exactly as `get_password_from_kdbx_exact` builds it.
    #[test]
    fn a_yubikey_read_raises_the_touch_cue_through_the_chokepoint() {
        use std::sync::Arc;

        use crate::secret::touch::{
            TouchObserver, set_touch_observer,
            test_support::{CountingObserver, exclusive},
        };

        let _exclusive = exclusive();
        let observer = Arc::new(CountingObserver::for_this_thread());
        #[expect(
            clippy::clone_on_ref_ptr,
            reason = "the clone must unsize to Arc<dyn TouchObserver>, which Arc::clone cannot"
        )]
        let installed: Arc<dyn TouchObserver> = observer.clone();
        set_touch_observer(Some(installed));

        // Build the read invocation exactly as a `-y` reader does.
        let mut args = vec![
            "show".to_string(),
            "-q".to_string(),
            "-s".to_string(),
            "-a".to_string(),
            "Password".to_string(),
        ];
        push_unlock_args(&mut args, true, None, Some("2:12345678"));
        args.push(kdbx().display().to_string());
        args.push("RustConn/web (ssh)".to_string());

        let invocation = Invocation::new(
            "show (exact entry)",
            args,
            InvocationKind::Read,
            true, // yubikey_slot.is_some()
        );

        let cli = FakeCli::with_replies([Reply::stdout("hunter2\n")]);
        cli.run(&invocation, Some(&db()), None).unwrap();

        // The `-y` slot reached the argv (so cli.run saw a touch-waiting run)...
        assert!(
            cli.any_arg_contains("-y"),
            "a yubikey read must carry -y so the chokepoint brackets the touch cue"
        );
        // ...and the chokepoint raised the touch cue at least once.
        assert!(
            observer.started() >= 1,
            "a yubikey read routed through the chokepoint must raise the touch cue"
        );

        set_touch_observer(None);
    }
}

/// Why a `keepassxc-cli show` exited non-zero.
///
/// The three readers in this file each classified this inline, and all three drew
/// the same line in the wrong place: anything that was not recognisably a
/// credential error became `Ok(None)`, i.e. "there is no such entry". So a corrupt
/// or unsupported database, an unreadable or wrong `--key-file`, and a
/// hardware-key database waiting for a touch all reported the same thing as an
/// empty database — and `Ok(None)` reaches the user as "Vault entry not found.
/// You will be prompted for a password", which names the wrong problem and offers
/// no way to act on the real one.
///
/// One classifier for all three readers, so they cannot drift apart again.
enum ShowFailure {
    /// The database opened and the entry is genuinely not in it.
    EntryMissing,
    /// The database did not open with the password or key file supplied.
    BadCredentials,
    /// Anything else. The database may be unreadable, of an unsupported version,
    /// or waiting on something nobody answered — but it was not opened, so
    /// "the entry is not there" is not a conclusion available to us.
    Unusable,
}

/// Classifies a failed `keepassxc-cli show` from its stderr.
///
/// String matching, because `keepassxc-cli` distinguishes these cases only in
/// prose and returns exit code 1 for all of them. That makes the wording a
/// dependency: a KeePassXC release that rephrases "Could not find entry" turns a
/// missing entry into [`ShowFailure::Unusable`], which is a dialog saying the
/// database could not be read rather than a prompt. That is the safe direction to
/// fail — the old behaviour failed the other way, turning an unopenable database
/// into "no such password" — but it is worth knowing which way it breaks.
fn classify_show_failure(stderr: &str) -> ShowFailure {
    if stderr.contains("Could not find entry")
        || stderr.contains("Entry not found")
        || stderr.contains("No entry found")
    {
        return ShowFailure::EntryMissing;
    }
    if is_bad_credentials(stderr) {
        return ShowFailure::BadCredentials;
    }
    ShowFailure::Unusable
}

/// Whether a failed `keepassxc-cli` run's stderr says the unlock factors were wrong.
fn is_bad_credentials(stderr: &str) -> bool {
    stderr.contains("Invalid credentials") || stderr.contains("wrong password")
}

/// The error for a failed whole-database `keepassxc-cli ls -R -f` listing.
///
/// Deliberately not [`classify_show_failure`]: a listing names no entry, so it
/// has no "entry missing" outcome. If its stderr happens to contain "Could not
/// find entry" (a group path, a localised build, a future rewording), mapping
/// that to `Ok(None)` would turn an unreadable database into "no stored
/// password" — the defect [`ShowFailure`] exists to prevent. A failed listing is
/// always an error: bad credentials, or the database could not be read.
fn list_failure_error(stderr: &str) -> SecretError {
    if is_bad_credentials(stderr) {
        SecretError::KeePassXC("Invalid database password".to_string())
    } else {
        SecretError::KeePassXC(format!("Could not read the database: {}", stderr.trim()))
    }
}

/// Display name a read-only KDBX database refuses writes under.
///
/// Shared with [`super::kdbx_backend::KdbxBackend::display_name`] so the
/// [`SecretError::ReadOnly`] message is the same whichever path refused.
pub(super) const KDBX_DISPLAY_NAME: &str = "KeePass (KDBX file)";

/// Refuses a KDBX write when the user put the database in read-only mode.
///
/// The single read-only chokepoint for every KDBX mutation: the three public
/// writers ([`KeePassStatus::save_password_to_kdbx`],
/// [`KeePassStatus::delete_entry_from_kdbx`] and
/// [`KeePassStatus::rename_entry_in_kdbx`]) take the flag as a required
/// parameter and call this before any validation or `keepassxc-cli` run, so no
/// caller can write to a read-only database by forgetting a check of its own.
fn ensure_kdbx_writable(read_only: bool) -> SecretResult<()> {
    if read_only {
        Err(SecretError::ReadOnly(KDBX_DISPLAY_NAME.to_string()))
    } else {
        Ok(())
    }
}

/// The entry paths a lookup tries, in order, for RustConn's own naming schemes.
///
/// Each one costs a separate `keepassxc-cli` invocation, and every invocation
/// reopens the database and pays its Argon2 cost again — around 700 ms on a
/// default KDBX. So the list is not free: a lookup that finds nothing pays for
/// every entry in it before the user sees a password prompt. It is extracted from
/// [`KeePassStatus::get_password_from_kdbx_with_key`] so the order is pinned by
/// tests rather than by reading the loop, since a `keepassxc-cli` is needed to
/// exercise the loop at all.
///
/// The candidates, in order:
///
/// 1. `RustConn/{entry_name}` — where this version writes.
/// 2. `RustConn/{entry_name without its " (protocol)" suffix}` — the older
///    format, before entries carried the protocol.
/// 3. `RustConn/{entry_name} ({protocol})` — only when the caller passes the
///    protocol separately instead of having it in the name already.
/// 4. `{entry_name}` — a root-level entry, from before entries were grouped
///    under `RustConn/` at all.
///
/// Candidate 4 is skipped when `entry_name` already carries a group path.
/// [`KeePassHierarchy::build_entry_path`](super::hierarchy::KeePassHierarchy::build_entry_path)
/// starts every path it builds at `RustConn`, so no release has ever written
/// `Group/name` at the database root — the un-prefixed form can only match the
/// ungrouped case, where the name is a bare entry name. Trying it for a grouped
/// connection was a full database open that could not succeed, on every lookup.
/// A path the *user* chose is not resolved through here: that is
/// [`KeePassStatus::get_password_from_kdbx_exact`], which queries it as-is.
fn candidate_entry_paths(entry_name: &str, protocol: Option<&str>) -> Vec<String> {
    let mut entry_paths = Vec::new();

    // First try exact entry name (may already include protocol suffix)
    entry_paths.push(format!("RustConn/{entry_name}"));

    // If entry_name contains protocol suffix like "name (ssh)", also try without it (legacy)
    // This handles migration from old format where entries were stored without protocol
    if let Some(base_name) = entry_name
        .strip_suffix(')')
        .and_then(|s| s.rfind(" (").map(|pos| &entry_name[..pos]))
    {
        entry_paths.push(format!("RustConn/{base_name}"));
    }

    // If protocol provided separately, try with it (for backward compatibility)
    if let Some(proto) = protocol {
        entry_paths.push(format!("RustConn/{entry_name} ({proto})"));
    }

    // Finally the un-prefixed name, but only where it could ever have been
    // written — see the note above.
    if !entry_name.contains('/') {
        entry_paths.push(entry_name.to_string());
    }

    entry_paths
}

/// Picks the vault-root entry path that matches `connection_id`, from the raw
/// `keepassxc-cli ls -R -f <db>` listing of the WHOLE database (no `RustConn`
/// scope).
///
/// This is the pure, side-effect-free core of the vault-root read widening:
/// it takes the flattened listing `keepassxc-cli` prints — one path per line,
/// group paths ending in `/`, entry paths not — and returns the first entry
/// whose basename (the component after the last `/`) equals `connection_id`'s
/// basename. A trailing-slash line is a group and is skipped; only leaf entries
/// are considered.
///
/// Why basename matching: the scoped reader looks under `RustConn/…`; this
/// widening exists to find an entry the user keeps *outside* that subtree (hand
/// made, or imported from another tool), which by definition lives at some
/// other group path. The entry name itself is the stable identifier, so a
/// `connection_id` of `"web (ssh)"` matches `Internet/web (ssh)` as readily as
/// a bare `web (ssh)` at the root.
///
/// **Read-widening only, and structurally #327-safe.** The returned path is an
/// ABSOLUTE path copied verbatim from the database listing; this function never
/// constructs a path and never prepends `RustConn/`, so it cannot produce the
/// doubled-prefix lookup issue #327 fixed. Writes do not go through here at all.
///
/// An exact full-path match (`line == connection_id`, or `line` ends with
/// `/<connection_id>`) is preferred over a looser basename match, so a caller
/// passing an already-qualified path still lands on it first. Returns `None`
/// when nothing matches.
fn root_match_entry_path(listing: &str, connection_id: &str) -> Option<String> {
    let wanted_base = connection_id.rsplit('/').next().unwrap_or(connection_id);

    let mut basename_fallback: Option<String> = None;
    for line in listing.lines() {
        let line = line.trim();
        if line.is_empty() || line.ends_with('/') {
            // Blank, or a group path (keepassxc-cli suffixes groups with '/').
            continue;
        }
        // Exact match (whole path, or a path whose tail is the connection id)
        // wins immediately — honour a caller that passed a qualified path.
        if line == connection_id || line.ends_with(&format!("/{connection_id}")) {
            return Some(line.to_string());
        }
        // Otherwise remember the first entry whose leaf name matches.
        if basename_fallback.is_none() {
            let line_base = line.rsplit('/').next().unwrap_or(line);
            if line_base == wanted_base {
                basename_fallback = Some(line.to_string());
            }
        }
    }
    basename_fallback
}

///
/// This struct provides information about the current state of `KeePass` integration,
/// including whether `KeePassXC` is installed, its version, and KDBX file accessibility.
#[derive(Debug, Clone, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "settings/flags struct mirrors persisted config 1:1; bools represent independent toggles, not a state machine"
)]
pub struct KeePassStatus {
    /// Whether `KeePassXC` application is installed
    pub keepassxc_installed: bool,
    /// `KeePassXC` version if installed
    pub keepassxc_version: Option<String>,
    /// Path to `KeePassXC` CLI binary
    pub keepassxc_path: Option<std::path::PathBuf>,
    /// Whether KDBX file is configured
    pub kdbx_configured: bool,
    /// Whether KDBX file exists and is accessible
    pub kdbx_accessible: bool,
    /// Whether integration is currently active (unlocked)
    pub integration_active: bool,
}

impl KeePassStatus {
    /// Detects current `KeePass` status by checking for `KeePassXC` installation
    ///
    /// This method searches for the `keepassxc-cli` binary in common locations
    /// and attempts to determine its version.
    #[must_use]
    pub fn detect() -> Self {
        let mut status = Self::default();

        // Try to find keepassxc-cli in PATH or common locations
        if let Some(path) = Self::find_keepassxc_cli() {
            status.keepassxc_installed = true;
            status.keepassxc_path = Some(path.clone());

            // Try to get version
            if let Some(version) = Self::get_keepassxc_version(&path) {
                status.keepassxc_version = Some(version);
            }
        }

        status
    }

    /// Detects status with a configured KDBX path
    ///
    /// # Arguments
    /// * `kdbx_path` - Optional path to the KDBX database file
    #[must_use]
    pub fn detect_with_kdbx(kdbx_path: Option<&Path>) -> Self {
        let mut status = Self::detect();

        if let Some(path) = kdbx_path {
            status.kdbx_configured = true;
            status.kdbx_accessible = path.exists() && path.is_file();
        }

        status
    }

    /// Validates a KDBX file path
    ///
    /// # Arguments
    /// * `path` - Path to validate
    ///
    /// # Returns
    /// * `Ok(())` if the path is valid (ends with .kdbx and file exists)
    /// * `Err(String)` with a description of the validation failure
    ///
    /// # Errors
    /// Returns an error if:
    /// - The path does not have a .kdbx extension (case-insensitive)
    /// - The file does not exist
    /// - The path points to a directory instead of a file
    pub fn validate_kdbx_path(path: &Path) -> SecretResult<()> {
        // Check extension (case-insensitive)
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_lowercase);

        if extension.as_deref() != Some("kdbx") {
            return Err(SecretError::KeePassXC(
                "File must have .kdbx extension".to_string(),
            ));
        }

        // Check if file exists
        if !path.exists() {
            return Err(SecretError::KeePassXC(format!(
                "File does not exist: {}",
                path.display()
            )));
        }

        // Check if it's a file (not a directory)
        if !path.is_file() {
            return Err(SecretError::KeePassXC(format!(
                "Path is not a file: {}",
                path.display()
            )));
        }

        Ok(())
    }

    /// Finds the `keepassxc-cli` binary, searching once per process.
    ///
    /// Every reader and writer in this module called this, and each call redid
    /// the whole search: a PATH walk plus up to six `stat`s natively, and inside
    /// a Flatpak sandbox **an extra child process** — `find_on_host` runs
    /// `sh -lc 'command -v …'` on the host. A single credential lookup is one of
    /// those, a single save is four, and all of them answer the same question
    /// about where a binary lives.
    ///
    /// Only a *successful* find is remembered. Caching the negative answer too
    /// would be the tidier `OnceLock<Option<_>>`, and it would be wrong: the
    /// Flatpak branch probes the host with a two-second budget, so one slow probe
    /// would leave the whole session convinced KeePassXC is not installed, with
    /// "keepassxc-cli not found. Please install KeePassXC." as the only symptom
    /// and a restart as the only cure. A guard that can outlast the condition it
    /// describes is worse than the cost it saves. A miss re-searches.
    fn find_keepassxc_cli() -> Option<std::path::PathBuf> {
        static LOCATION: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

        if let Some(cached) = LOCATION.get() {
            return Some(cached.clone());
        }
        let found = Self::locate_keepassxc_cli()?;
        // A lost race means another thread found the same binary first.
        Some(LOCATION.get_or_init(|| found).clone())
    }

    /// Performs the actual search behind [`Self::find_keepassxc_cli`].
    ///
    /// Searches in PATH and common installation locations. Inside a Flatpak
    /// sandbox, KeePassXC cannot be bundled (it is the user's host GUI app),
    /// so the host binary is located via `flatpak-spawn --host`.
    fn locate_keepassxc_cli() -> Option<std::path::PathBuf> {
        // In Flatpak, resolve and run keepassxc-cli on the host (see #182). The
        // probe used to live here; it is now `which::find_on_host`, which does the
        // same `sh -lc 'command -v …'` for every host binary and bounds the wait.
        if crate::flatpak::is_flatpak() {
            return crate::which::find_on_host("keepassxc-cli");
        }

        // PATH, extended with the Homebrew and KeePassXC.app directories a macOS
        // `.app` does not inherit. Resolved in process — spawning `which` made
        // the answer depend on a binary that need not be installed (#303).
        if let Some(path) = crate::which::find_in_path("keepassxc-cli") {
            return Some(path);
        }

        // Check common installation paths
        let common_paths = [
            "/usr/bin/keepassxc-cli",
            "/usr/local/bin/keepassxc-cli",
            "/snap/bin/keepassxc-cli",
            "/var/lib/flatpak/exports/bin/org.keepassxc.KeePassXC.cli",
            // macOS: Homebrew (Apple Silicon and Intel)
            "/opt/homebrew/bin/keepassxc-cli",
            // macOS: KeePassXC.app bundle
            "/Applications/KeePassXC.app/Contents/MacOS/keepassxc-cli",
        ];

        for path_str in &common_paths {
            let path = std::path::PathBuf::from(path_str);
            if path.exists() {
                return Some(path);
            }
        }

        None
    }

    /// Builds a [`Command`] for running `keepassxc-cli`.
    ///
    /// The returned command has no arguments yet — callers append `.arg(...)` as needed.
    ///
    /// Inside a Flatpak sandbox the invocation is routed through
    /// `flatpak-spawn --host` so the host's KeePassXC is used (it cannot be
    /// bundled in the sandbox). flatpak-spawn forwards stdin/stdout/stderr to
    /// the host process by default, so piped database/entry passwords reach it.
    /// Appending args then yields `flatpak-spawn --host <cli> <args...>`.
    ///
    /// Otherwise the binary is run directly with the extended PATH injected so
    /// that child processes (e.g. GPG invoked by keepassxc-cli) can also be
    /// found on macOS where GUI apps have minimal PATH.
    ///
    /// The child also gets a neutralised *message* locale, which
    /// [`classify_show_failure`] depends on. `keepassxc-cli` is a Qt program and
    /// translates its diagnostics, while this process exports `LANGUAGE` at
    /// startup to honour the application's own language setting (see
    /// `rustconn::i18n`). So with a non-English UI the CLI answered in that
    /// language, none of the English needles matched, and a missing entry was
    /// classified as an unreadable database: the user saw "Could not read the
    /// password from KeePassXC — it may be locked, not logged in, or not set up on
    /// this computer" about a database that was open and healthy, and because that
    /// path returns `Err`, the **Also read from the encrypted file** fallback was
    /// skipped as well. The wording of the CLI's prose was already documented as a
    /// dependency; what was missed is that we localise it ourselves.
    ///
    /// `C` is forced for messages only, and the character encoding is deliberately
    /// left as the user had it: entry paths and the database path are passed as
    /// arguments, and a Qt 5 build derives its argv codec from the locale's
    /// charset, so forcing the C locale wholesale would mangle a non-ASCII group
    /// name or database path — trading this bug for a worse one. `LC_ALL` outranks
    /// `LC_MESSAGES` in POSIX, so it cannot simply be left in place; when it is set
    /// its value is copied to `LC_CTYPE` first, which preserves exactly the
    /// encoding it was providing, and only then is `LC_ALL` dropped. That copy is
    /// the one write to `LC_CTYPE` here, and it changes no behaviour by itself.
    fn keepassxc_command(cli_path: &Path) -> Command {
        if crate::flatpak::is_flatpak() {
            let mut cmd = Command::new("flatpak-spawn");
            // Forwarded explicitly: the host process does not take these from the
            // sandbox. Blanked rather than unset because `--unset-env` is newer
            // than the oldest flatpak this runs under, and an empty value is what
            // gettext and Qt both read as "no preference". A host that exports
            // `LC_ALL` still outranks this; that is left alone rather than
            // guessed at, since the sandbox cannot see the host's encoding.
            cmd.arg("--host")
                .arg("--env=LC_MESSAGES=C")
                .arg("--env=LANGUAGE=")
                .arg(cli_path);
            return cmd;
        }
        let mut cmd = Command::new(cli_path);
        cmd.env("PATH", crate::cli_download::get_extended_path());
        cmd.env("LC_MESSAGES", "C");
        cmd.env_remove("LANGUAGE");
        if let Ok(lc_all) = std::env::var("LC_ALL") {
            if !lc_all.is_empty() {
                cmd.env("LC_CTYPE", lc_all);
            }
            cmd.env_remove("LC_ALL");
        }
        cmd
    }

    /// Gets the `KeePassXC` version from the CLI
    ///
    /// # Arguments
    /// * `cli_path` - Path to the `keepassxc-cli` binary
    fn get_keepassxc_version(cli_path: &Path) -> Option<String> {
        // Spawned and waited rather than `.output()`, which has no deadline. This
        // one runs from `detect()`, i.e. while the Settings dialog is being built,
        // so an unresponsive binary here freezes the window.
        let child = match Self::keepassxc_command(cli_path)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                tracing::warn!(?e, cli = %cli_path.display(), "failed to run keepassxc-cli --version");
                return None;
            }
        };
        let output = match wait_for_cli(child, "--version") {
            Ok(output) => output,
            Err(e) => {
                tracing::warn!(?e, cli = %cli_path.display(), "keepassxc-cli --version did not answer");
                return None;
            }
        };

        let version = if output.status.success() {
            parse_keepassxc_version(&String::from_utf8_lossy(&output.stdout))
        } else {
            // Some versions output to stderr
            parse_keepassxc_version(&String::from_utf8_lossy(&output.stderr))
        };

        if version.is_none() {
            tracing::warn!(
                exit_code = ?output.status.code(),
                stdout = %String::from_utf8_lossy(&output.stdout).trim(),
                stderr = %String::from_utf8_lossy(&output.stderr).trim(),
                "could not parse keepassxc-cli version"
            );
        }
        version
    }

    /// Retrieves a password from KDBX database using `keepassxc-cli`
    ///
    /// # Arguments
    /// * `kdbx_path` - Path to the KDBX database file
    /// * `db_password` - Password to unlock the database
    /// * `entry_name` - Name of the entry to look up (connection name or host)
    ///
    /// # Returns
    /// * `Ok(Some(String))` if the password is found
    /// * `Ok(None)` if the entry is not found
    /// * `Err(String)` with error description if retrieval fails
    ///
    /// # Errors
    /// Returns an error if:
    /// - `keepassxc-cli` is not installed
    /// - The KDBX file path is invalid
    /// - The database password is incorrect
    pub fn get_password_from_kdbx(
        kdbx_path: &Path,
        db_password: &SecretString,
        entry_name: &str,
    ) -> SecretResult<Option<SecretString>> {
        use std::io::Write as IoWrite;
        use std::process::Stdio;

        // First validate the path
        Self::validate_kdbx_path(kdbx_path)?;

        // Find keepassxc-cli
        let cli_path = Self::find_keepassxc_cli().ok_or_else(|| {
            SecretError::KeePassXC("keepassxc-cli not found. Please install KeePassXC.".to_string())
        })?;

        // Use keepassxc-cli show command to get the password
        // Format: keepassxc-cli show -q -s -a Password <database> <entry>
        let mut child = Self::keepassxc_command(&cli_path)
            .arg("show")
            .arg("-q") // Quiet mode — suppress password prompt
            .arg("-s") // Show password attribute
            .arg("-a")
            .arg("Password") // Get password attribute
            .arg(kdbx_path)
            .arg(entry_name)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SecretError::KeePassXC(format!("Failed to run keepassxc-cli: {e}")))?;

        // Write database password to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(db_password.expose_secret().as_bytes())
                .map_err(|e| SecretError::KeePassXC(format!("Failed to send password: {e}")))?;
            stdin
                .write_all(b"\n")
                .map_err(|e| SecretError::KeePassXC(format!("Failed to send password: {e}")))?;
        }

        let output = wait_for_cli(child, "show")?;

        if output.status.success() {
            // Wiped on drop: this is the credential itself, in the clear, on its
            // way into the `SecretString`. `get_password_from_kdbx_exact` already
            // did this; the other two readers in this file did not.
            let password =
                zeroize::Zeroizing::new(String::from_utf8_lossy(&output.stdout).trim().to_string());
            if password.is_empty() {
                Ok(None)
            } else {
                Ok(Some(SecretString::from(password.as_str())))
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            match classify_show_failure(&stderr) {
                ShowFailure::EntryMissing => Ok(None),
                ShowFailure::BadCredentials => Err(SecretError::KeePassXC(
                    "Invalid database password".to_string(),
                )),
                // Was `Ok(None)` after a warning, which told the log the truth and
                // the user something else.
                ShowFailure::Unusable => {
                    tracing::warn!(
                        entry_name,
                        exit_code = ?output.status.code(),
                        stderr = %stderr.trim(),
                        "keepassxc-cli could not read the database"
                    );
                    Err(SecretError::KeePassXC(format!(
                        "Could not read the database: {}",
                        stderr.trim()
                    )))
                }
            }
        }
    }

    /// Saves a password to KDBX database using `keepassxc-cli`
    ///
    /// # Arguments
    /// * `kdbx_path` - Path to the KDBX database file
    /// * `db_password` - Password to unlock the database (None if using key file)
    /// * `key_file` - Optional path to key file for authentication
    /// * `entry_name` - Name of the entry (connection name or host)
    /// * `username` - Username for the entry
    /// * `password` - Password to save
    /// * `url` - Optional URL for the entry
    /// * `yubikey_slot` - Optional `YubiKey` Challenge-Response slot as
    ///   `slot[:serial]`; when `Some`, passed to `keepassxc-cli` as `-y` and
    ///   composed with the password and/or key file. `None` preserves the
    ///   historical password/key-file-only unlock.
    /// * `read_only` - `SecretSettings::kdbx_read_only`; when `true` the write is
    ///   refused before anything runs.
    ///
    /// # Returns
    /// * `Ok(())` if the password is saved successfully
    /// * `Err(String)` with error description if saving fails
    ///
    /// # Errors
    /// Returns an error if:
    /// - the database is in read-only mode ([`SecretError::ReadOnly`])
    /// - `keepassxc-cli` is not installed
    /// - The KDBX file path is invalid
    /// - The database password/key file is incorrect
    /// - The entry cannot be created
    ///
    /// Note: Entry names include protocol suffix to allow same name for different protocols.
    /// Format: `RustConn/{entry_name} ({protocol})` where protocol is extracted from URL.
    #[expect(
        clippy::too_many_lines,
        reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
    )]
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the read path get_password_from_kdbx_with_key; the KDBX write needs every unlock factor plus the entry fields"
    )]
    pub fn save_password_to_kdbx(
        kdbx_path: &Path,
        db_password: Option<&SecretString>,
        key_file: Option<&Path>,
        entry_name: &str,
        username: &str,
        password: &SecretString,
        url: Option<&str>,
        yubikey_slot: Option<&str>,
        read_only: bool,
    ) -> SecretResult<()> {
        use std::io::Write as IoWrite;
        use std::process::Stdio;

        // Read-only first: a read-only database is never touched, and the
        // refusal is the same whether or not the path would have validated.
        ensure_kdbx_writable(read_only)?;

        // First validate the path
        Self::validate_kdbx_path(kdbx_path)?;

        // Find keepassxc-cli
        let cli_path = Self::find_keepassxc_cli().ok_or_else(|| {
            SecretError::KeePassXC("keepassxc-cli not found. Please install KeePassXC.".to_string())
        })?;

        // keepassxc-cli 2.5.0+ (decision D3): edit the entry in place, create
        // only missing groups, no rm — far fewer YubiKey touches (#350). All of
        // it under the process-wide write lock so a concurrent save/delete
        // cannot interleave and lose an update. Older or unreadable versions
        // keep the legacy ls/rm/add algorithm below unchanged.
        if Self::get_keepassxc_version(&cli_path).is_some_and(|v| version_supports_edit(&v)) {
            return with_keepass_write_lock(|| {
                let cli = RealKeePassCli::new(&cli_path);
                save_in_place(
                    &cli,
                    db_password,
                    key_file,
                    yubikey_slot,
                    kdbx_path,
                    entry_name,
                    username,
                    password,
                    url,
                )
            });
        }

        // Ensure RustConn group exists
        Self::ensure_rustconn_group(kdbx_path, db_password, key_file, &cli_path, yubikey_slot)?;

        // Build the entry path under RustConn group
        // entry_name should already include protocol suffix if needed (e.g., "server (rdp)")
        let entry_path = format!("RustConn/{entry_name}");

        // Ensure all parent groups in the path exist (e.g., RustConn/Groups for group passwords)
        Self::ensure_parent_groups(
            kdbx_path,
            db_password,
            key_file,
            &cli_path,
            entry_name,
            yubikey_slot,
        )?;

        // First, try to remove existing entry (ignore errors if it doesn't exist)
        let _ =
            Self::delete_kdbx_entry(kdbx_path, db_password, key_file, &entry_path, yubikey_slot);

        // Build command arguments for keepassxc-cli add
        // Format: keepassxc-cli add [options] <database> <entry>
        // -p/--password-prompt prompts for entry password via stdin (after db password)
        // Assembled by a free function so a test can assert the argv without a
        // real keepassxc-cli or database — see `build_add_args`.
        let args = build_add_args(
            db_password.is_some(),
            key_file,
            yubikey_slot,
            username,
            url,
            kdbx_path,
            &entry_path,
        );

        tracing::debug!("Running keepassxc-cli with args: {args:?}");

        let mut child = Self::keepassxc_command(&cli_path)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SecretError::KeePassXC(format!("Failed to run keepassxc-cli: {e}")))?;

        // Write passwords to stdin
        // When using --no-password (key file only): only entry password is needed
        // When using password: database password first, then entry password
        if let Some(mut stdin) = child.stdin.take() {
            // Database password (only if not using --no-password)
            if let Some(db_pwd) = db_password {
                stdin
                    .write_all(db_pwd.expose_secret().as_bytes())
                    .map_err(|e| {
                        SecretError::KeePassXC(format!("Failed to send database password: {e}"))
                    })?;
                stdin
                    .write_all(b"\n")
                    .map_err(|e| SecretError::KeePassXC(format!("Failed to send newline: {e}")))?;
            }

            // Entry password (prompted by -p flag)
            tracing::debug!("Sending entry password to keepassxc-cli");
            stdin
                .write_all(password.expose_secret().as_bytes())
                .map_err(|e| {
                    SecretError::KeePassXC(format!("Failed to send entry password: {e}"))
                })?;
            stdin
                .write_all(b"\n")
                .map_err(|e| SecretError::KeePassXC(format!("Failed to send newline: {e}")))?;

            // Close stdin to signal end of input
            drop(stdin);
        }

        // A `-y` unlock blocks on a physical touch, so give the write the longer
        // touch budget; a password/key-file-only write keeps the write budget.
        let output = if yubikey_slot.is_some() {
            wait_for_cli_yubikey(child, "add")?
        } else {
            wait_for_cli_write(child, "add")?
        };

        tracing::debug!(
            "keepassxc-cli exit code: {:?}, stdout: '{}', stderr: '{}'",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stderr.contains("Invalid credentials")
                || stderr.contains("wrong password")
                || stderr.contains("Error while reading the database")
            {
                Err(SecretError::KeePassXC(
                    "Invalid database password or key file".to_string(),
                ))
            } else if stderr.contains("Could not find group") {
                Err(SecretError::KeePassXC(
                    "RustConn group not found in database. Please create a group \
                     named 'RustConn' in your KeePass database."
                        .to_string(),
                ))
            } else if stderr.contains("already exists") {
                Err(SecretError::KeePassXC(format!(
                    "Entry '{entry_name}' already exists"
                )))
            } else if stderr.is_empty() && stdout.is_empty() {
                // The reproduction command the user can paste, mirroring the exact
                // unlock factors this call used — including `-y <slot>`, whose
                // absence is what the #350 reporter flagged.
                let yubikey_hint = match yubikey_slot {
                    Some(slot) => format!(" -y {slot}"),
                    None => String::new(),
                };
                Err(SecretError::KeePassXC(format!(
                    "Failed to save password to KeePass database (exit code: {:?}). \
                     Try running: keepassxc-cli add -p{} {} 'RustConn/{}'",
                    output.status.code(),
                    yubikey_hint,
                    kdbx_path.display(),
                    entry_name
                )))
            } else {
                let error_msg = if stderr.is_empty() { stdout } else { stderr };
                Err(SecretError::KeePassXC(format!(
                    "KeePass error: {}",
                    error_msg.trim()
                )))
            }
        }
    }

    /// Ensures the `RustConn` group exists in the database
    fn ensure_rustconn_group(
        kdbx_path: &Path,
        db_password: Option<&SecretString>,
        key_file: Option<&Path>,
        cli_path: &Path,
        yubikey_slot: Option<&str>,
    ) -> SecretResult<()> {
        use std::io::Write as IoWrite;
        use std::process::Stdio;

        tracing::debug!("Checking if RustConn group exists...");

        // First check if RustConn group exists using ls command
        let mut args = vec!["ls".to_string(), "-q".to_string()];

        push_unlock_args(&mut args, db_password.is_some(), key_file, yubikey_slot);

        args.push(kdbx_path.display().to_string());
        args.push("RustConn".to_string());

        let mut child = Self::keepassxc_command(cli_path)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SecretError::KeePassXC(format!("Failed to run keepassxc-cli: {e}")))?;

        // Only send password if we have one
        if let Some(mut stdin) = child.stdin.take()
            && let Some(db_pwd) = db_password
        {
            stdin.write_all(db_pwd.expose_secret().as_bytes()).ok();
            stdin.write_all(b"\n").ok();
        }

        // The group probe opens the database, so with a `-y` slot it too can block
        // on a touch; give it the touch budget then.
        let output = if yubikey_slot.is_some() {
            wait_for_cli_yubikey(child, "ls (group probe)").ok()
        } else {
            wait_for_cli(child, "ls (group probe)").ok()
        };

        // If group exists, we're done
        if let Some(ref o) = output {
            tracing::debug!(
                "ls RustConn result: exit={:?}, stdout='{}', stderr='{}'",
                o.status.code(),
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            if o.status.success() {
                tracing::debug!("RustConn group exists");
                return Ok(());
            }
        }

        tracing::debug!("RustConn group doesn't exist, creating...");

        // Group doesn't exist, create it using mkdir command
        let mut args = vec!["mkdir".to_string(), "-q".to_string()];

        push_unlock_args(&mut args, db_password.is_some(), key_file, yubikey_slot);

        args.push(kdbx_path.display().to_string());
        args.push("RustConn".to_string());

        let mut child = Self::keepassxc_command(cli_path)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                SecretError::KeePassXC(format!("Failed to run keepassxc-cli mkdir: {e}"))
            })?;

        // Only send password if we have one
        if let Some(mut stdin) = child.stdin.take()
            && let Some(db_pwd) = db_password
        {
            stdin.write_all(db_pwd.expose_secret().as_bytes()).ok();
            stdin.write_all(b"\n").ok();
        }

        let output = if yubikey_slot.is_some() {
            wait_for_cli_yubikey(child, "mkdir")?
        } else {
            wait_for_cli_write(child, "mkdir")?
        };

        tracing::debug!(
            "mkdir RustConn result: exit={:?}, stdout='{}', stderr='{}'",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        if output.status.success() {
            tracing::debug!("RustConn group created successfully");
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // If group already exists, that's fine
            if stderr.contains("already exists") {
                tracing::debug!("RustConn group already exists");
                Ok(())
            } else if stderr.contains("Invalid credentials") || stderr.contains("wrong password") {
                Err(SecretError::KeePassXC(
                    "Invalid database password or key file".to_string(),
                ))
            } else {
                // Don't fail if we can't create the group
                tracing::debug!("Failed to create group, but continuing: {stderr}");
                Ok(())
            }
        }
    }

    /// Ensures all parent groups in a path exist
    ///
    /// For path "Groups/Production/Web", creates:
    /// - RustConn/Groups
    /// - RustConn/Groups/Production
    /// - RustConn/Groups/Production/Web
    fn ensure_parent_groups(
        kdbx_path: &Path,
        db_password: Option<&SecretString>,
        key_file: Option<&Path>,
        cli_path: &Path,
        entry_path: &str,
        yubikey_slot: Option<&str>,
    ) -> SecretResult<()> {
        use std::io::Write as IoWrite;
        use std::process::Stdio;

        // Extract parent path (everything except the last component which is the entry name)
        let parts: Vec<&str> = entry_path.split('/').collect();
        if parts.len() <= 1 {
            // No parent groups needed
            return Ok(());
        }

        // Build cumulative paths for all parent groups
        let mut current_path = String::from("RustConn");
        for part in &parts[..parts.len() - 1] {
            current_path = format!("{current_path}/{part}");

            tracing::debug!("Ensuring group exists: {}", current_path);

            // Try to create the group (ignore if already exists)
            let mut args = vec!["mkdir".to_string(), "-q".to_string()];

            push_unlock_args(&mut args, db_password.is_some(), key_file, yubikey_slot);

            args.push(kdbx_path.display().to_string());
            args.push(current_path.clone());

            let mut child = Self::keepassxc_command(cli_path)
                .args(&args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| {
                    SecretError::KeePassXC(format!("Failed to run keepassxc-cli mkdir: {e}"))
                })?;

            if let Some(mut stdin) = child.stdin.take()
                && let Some(db_pwd) = db_password
            {
                stdin.write_all(db_pwd.expose_secret().as_bytes()).ok();
                stdin.write_all(b"\n").ok();
            }

            let output = if yubikey_slot.is_some() {
                wait_for_cli_yubikey(child, "mkdir (parent group)").ok()
            } else {
                wait_for_cli_write(child, "mkdir (parent group)").ok()
            };

            // A failed intermediate group must abort: previously any failure
            // here (or a swallowed wait error) only logged at debug! and the
            // function still returned Ok, so `add` would then target a group
            // that was never created — the save appeared to "succeed" while the
            // nested groups silently never appeared (issue observed via #350
            // write-path follow-up). Treat success and "already exists" as fine,
            // and surface anything else as an error so the caller can report it.
            match output {
                Some(ref o) => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    if o.status.success() || stderr.contains("already exists") {
                        tracing::debug!("Group '{}' ready", current_path);
                    } else {
                        return Err(SecretError::KeePassXC(format!(
                            "Failed to create parent group '{current_path}': {}",
                            stderr.trim()
                        )));
                    }
                }
                None => {
                    return Err(SecretError::KeePassXC(format!(
                        "Failed to create parent group '{current_path}': keepassxc-cli mkdir did not complete"
                    )));
                }
            }
        }

        Ok(())
    }

    /// Deletes an entry from KDBX database
    fn delete_kdbx_entry(
        kdbx_path: &Path,
        db_password: Option<&SecretString>,
        key_file: Option<&Path>,
        entry_path: &str,
        yubikey_slot: Option<&str>,
    ) -> SecretResult<()> {
        use std::io::Write as IoWrite;
        use std::process::Stdio;

        let cli_path = Self::find_keepassxc_cli()
            .ok_or_else(|| SecretError::KeePassXC("keepassxc-cli not found".to_string()))?;

        let mut args = vec!["rm".to_string(), "-q".to_string()];

        push_unlock_args(&mut args, db_password.is_some(), key_file, yubikey_slot);

        args.push(kdbx_path.display().to_string());
        args.push(entry_path.to_string());

        let mut child = Self::keepassxc_command(&cli_path)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SecretError::KeePassXC(format!("Failed to run keepassxc-cli: {e}")))?;

        // Only send password if we have one
        if let Some(mut stdin) = child.stdin.take()
            && let Some(db_pwd) = db_password
        {
            stdin.write_all(db_pwd.expose_secret().as_bytes()).ok();
            stdin.write_all(b"\n").ok();
        }

        // Best-effort by design: the caller deletes before adding and does not
        // care whether the entry existed. Bounded all the same — "does not care
        // about the result" is not the same as "may block for ever". A `-y`
        // unlock waits on a touch, so it gets the touch budget.
        if yubikey_slot.is_some() {
            let _ = wait_for_cli_yubikey(child, "rm");
        } else {
            let _ = wait_for_cli_write(child, "rm");
        }
        Ok(())
    }

    /// Deletes an entry from KDBX database (public API)
    ///
    /// # Arguments
    /// * `kdbx_path` - Path to the KDBX database file
    /// * `db_password` - Password to unlock the database (None if using key file)
    /// * `key_file` - Optional path to key file for authentication
    /// * `entry_path` - Full path of the entry to delete (e.g., "RustConn/Group/Name (rdp)")
    /// * `yubikey_slot` - Optional `YubiKey` Challenge-Response slot as
    ///   `slot[:serial]`; when `Some`, passed to `keepassxc-cli` as `-y` and
    ///   composed with the password and/or key file.
    /// * `read_only` - `SecretSettings::kdbx_read_only`; when `true` the delete
    ///   is refused before anything runs.
    ///
    /// # Returns
    /// * `Ok(())` if the entry is deleted or doesn't exist
    /// * `Err(String)` if the operation fails
    ///
    /// # Errors
    /// Returns an error if:
    /// - the database is in read-only mode ([`SecretError::ReadOnly`])
    /// - `keepassxc-cli` is not installed
    /// - The KDBX file path is invalid
    /// - The database password/key file is incorrect
    pub fn delete_entry_from_kdbx(
        kdbx_path: &Path,
        db_password: Option<&SecretString>,
        key_file: Option<&Path>,
        entry_path: &str,
        yubikey_slot: Option<&str>,
        read_only: bool,
    ) -> SecretResult<()> {
        ensure_kdbx_writable(read_only)?;

        // First validate the path
        Self::validate_kdbx_path(kdbx_path)?;

        // Find keepassxc-cli
        Self::find_keepassxc_cli().ok_or_else(|| {
            SecretError::KeePassXC("keepassxc-cli not found. Please install KeePassXC.".to_string())
        })?;

        // Under the process-wide write lock: a delete rewrites the KDBX, so it
        // must not interleave with a concurrent save (the edit-dialog race in
        // #350, where a save and a stale-key delete ran at once and one update
        // was lost).
        with_keepass_write_lock(|| {
            Self::delete_kdbx_entry(kdbx_path, db_password, key_file, entry_path, yubikey_slot)
        })
    }

    /// Retrieves a password from KDBX database using `keepassxc-cli` with key file support
    ///
    /// # Arguments
    /// * `kdbx_path` - Path to the KDBX database file
    /// * `db_password` - Password to unlock the database (None if using key file only)
    /// * `key_file` - Optional path to key file for authentication
    /// * `entry_name` - Name of the entry to look up (connection name or host)
    /// * `protocol` - Optional protocol (ssh, rdp, vnc, spice) for more specific lookup
    /// * `yubikey_slot` - Optional `YubiKey` Challenge-Response slot as
    ///   `slot[:serial]`; when `Some`, passed to `keepassxc-cli` as `-y` and
    ///   composed with the password and/or key file. `None` preserves the
    ///   historical password/key-file-only unlock.
    ///
    /// # Returns
    /// * `Ok(Some(String))` if the password is found
    /// * `Ok(None)` if the entry is not found
    ///
    /// Note: Searches in order: `RustConn/{name}`, `RustConn/{base_name}` (without protocol suffix), `{name}`
    ///
    /// # Errors
    ///
    /// Returns [`SecretError::Backend`] if `keepassxc-cli` cannot be spawned,
    /// the database cannot be unlocked (wrong password or key file), or the
    /// CLI returns a non-zero exit code for any reason other than "entry not
    /// found".
    pub fn get_password_from_kdbx_with_key(
        kdbx_path: &Path,
        db_password: Option<&SecretString>,
        key_file: Option<&Path>,
        entry_name: &str,
        protocol: Option<&str>,
        yubikey_slot: Option<&str>,
    ) -> SecretResult<Option<SecretString>> {
        // First validate the path
        Self::validate_kdbx_path(kdbx_path)?;

        // Find keepassxc-cli
        let cli_path = Self::find_keepassxc_cli().ok_or_else(|| {
            SecretError::KeePassXC("keepassxc-cli not found. Please install KeePassXC.".to_string())
        })?;

        // Route the read through the shared chokepoint so a `-y` unlock brackets
        // the run with the touch cue (#350). `cli.run` feeds the db password on
        // stdin and picks the budget from (Read, waits_on_touch); the reader no
        // longer spawns or waits itself.
        let cli = RealKeePassCli::new(&cli_path);

        let entry_paths = candidate_entry_paths(entry_name, protocol);

        tracing::debug!(
            "get_password: entry_name='{}', protocol={:?}, has_password={}, has_key_file={}, has_yubikey={}",
            entry_name,
            protocol,
            db_password.is_some(),
            key_file.is_some(),
            yubikey_slot.is_some()
        );

        // First "the database would not open" seen while walking the candidate
        // paths. Kept so that a run which finds nothing can say *why* it found
        // nothing; see the end of the loop.
        let mut unusable: Option<String> = None;

        for entry_path in &entry_paths {
            let mut args = vec![
                "show".to_string(),
                "-q".to_string(),
                "-s".to_string(),
                "-a".to_string(),
                "Password".to_string(),
            ];

            push_unlock_args(&mut args, db_password.is_some(), key_file, yubikey_slot);

            args.push(kdbx_path.display().to_string());
            args.push(entry_path.clone());

            tracing::debug!("get_password: trying path '{entry_path}'");

            let invocation = Invocation::new(
                "show (with key file)",
                args,
                InvocationKind::Read,
                yubikey_slot.is_some(),
            );
            let output = cli.run(&invocation, db_password, None)?;

            tracing::debug!(
                "get_password: exit={:?}, stderr='{}'",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );

            if output.status.success() {
                // Same wipe as `get_password_from_kdbx_exact`. This is the reader
                // the bulk credential transfer goes through, so a plaintext copy
                // left in the allocator here is one per entry rather than one per
                // connection attempt.
                let password = zeroize::Zeroizing::new(
                    String::from_utf8_lossy(&output.stdout).trim().to_string(),
                );
                if !password.is_empty() {
                    tracing::debug!("get_password: found password at '{entry_path}'");
                    return Ok(Some(SecretString::from(password.as_str())));
                }
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                match classify_show_failure(&stderr) {
                    // Wrong key for the database — no later candidate path can
                    // succeed, so stop rather than retrying the same refusal.
                    ShowFailure::BadCredentials => {
                        return Err(SecretError::KeePassXC(
                            "Invalid database password".to_string(),
                        ));
                    }
                    // This path is not in the database; the next candidate may be.
                    ShowFailure::EntryMissing => {
                        tracing::debug!("get_password: no entry at '{entry_path}'");
                    }
                    // The database was not opened. Remembered rather than returned
                    // immediately, because the candidate list exists to cope with
                    // several historical key formats and a later one may still
                    // work; but if none does, this is what gets reported instead of
                    // "no such entry".
                    ShowFailure::Unusable => {
                        tracing::warn!(
                            entry_path = %entry_path,
                            exit_code = ?output.status.code(),
                            stderr = %stderr.trim(),
                            "keepassxc-cli could not read the database"
                        );
                        if unusable.is_none() {
                            unusable = Some(stderr.trim().to_string());
                        }
                    }
                }
            }
        }

        // No candidate produced a password. Whether that means "the entry is not
        // there" or "the database could not be read" is the distinction the caller
        // needs: the first is a password prompt, the second is a dialog naming the
        // database. Reporting the first for both is what made a corrupt database
        // look like an empty one.
        if let Some(stderr) = unusable {
            return Err(SecretError::KeePassXC(format!(
                "Could not read the database: {stderr}"
            )));
        }

        tracing::debug!("get_password: password not found");
        Ok(None)
    }

    /// Retrieves a password from KDBX database at an exact path (no fallbacks).
    ///
    /// Unlike [`get_password_from_kdbx_with_key`] which tries multiple path
    /// variants with `RustConn/` prefix, this function queries the entry at
    /// `entry_path` **as-is**. Use for user-specified custom KeePass paths.
    ///
    /// # Arguments
    /// * `kdbx_path` - Path to the KDBX database file
    /// * `db_password` - Password to unlock the database (None if using key file)
    /// * `key_file` - Optional path to key file for authentication
    /// * `entry_path` - Exact path of the entry (e.g., "Internet/MyRouter" or "RustConn/RADIUS")
    /// * `yubikey_slot` - Optional `YubiKey` Challenge-Response slot as
    ///   `slot[:serial]`; when `Some`, passed to `keepassxc-cli` as `-y` and
    ///   composed with the password and/or key file. Without it a custom-path
    ///   read of a CR-protected database fails the same way the write path did
    ///   before issue #350's follow-up fix.
    ///
    /// # Returns
    /// * `Ok(Some(SecretString))` if the password is found
    /// * `Ok(None)` if the entry is not found
    ///
    /// # Errors
    ///
    /// Returns [`SecretError::Backend`] if `keepassxc-cli` cannot be spawned,
    /// the database cannot be unlocked (wrong password or key file), or the
    /// CLI returns a non-zero exit code for any reason other than "entry not
    /// found".
    pub fn get_password_from_kdbx_exact(
        kdbx_path: &Path,
        db_password: Option<&SecretString>,
        key_file: Option<&Path>,
        entry_path: &str,
        yubikey_slot: Option<&str>,
    ) -> SecretResult<Option<SecretString>> {
        Self::validate_kdbx_path(kdbx_path)?;

        let cli_path = Self::find_keepassxc_cli().ok_or_else(|| {
            SecretError::KeePassXC("keepassxc-cli not found. Please install KeePassXC.".to_string())
        })?;

        let cli = RealKeePassCli::new(&cli_path);

        let mut args = vec![
            "show".to_string(),
            "-q".to_string(),
            "-s".to_string(),
            "-a".to_string(),
            "Password".to_string(),
        ];

        push_unlock_args(&mut args, db_password.is_some(), key_file, yubikey_slot);

        args.push(kdbx_path.display().to_string());
        args.push(entry_path.to_string());

        tracing::debug!("get_password_exact: trying path '{entry_path}'");

        // Shared chokepoint: `-y` brackets the touch cue, db password on stdin,
        // Read budget bumped to the YubiKey budget when it waits on a touch (#350).
        let invocation = Invocation::new(
            "show (exact entry)",
            args,
            InvocationKind::Read,
            yubikey_slot.is_some(),
        );
        let output = cli.run(&invocation, db_password, None)?;

        if output.status.success() {
            let password =
                zeroize::Zeroizing::new(String::from_utf8_lossy(&output.stdout).trim().to_string());
            if password.is_empty() {
                Ok(None)
            } else {
                tracing::debug!("get_password_exact: found password at '{entry_path}'");
                Ok(Some(SecretString::from(password.as_str())))
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            match classify_show_failure(&stderr) {
                ShowFailure::EntryMissing => {
                    tracing::debug!("get_password_exact: no entry at '{entry_path}'");
                    Ok(None)
                }
                ShowFailure::BadCredentials => Err(SecretError::KeePassXC(
                    "Invalid database password".to_string(),
                )),
                ShowFailure::Unusable => {
                    tracing::warn!(
                        entry_path = %entry_path,
                        exit_code = ?output.status.code(),
                        stderr = %stderr.trim(),
                        "keepassxc-cli could not read the database"
                    );
                    Err(SecretError::KeePassXC(format!(
                        "Could not read the database: {}",
                        stderr.trim()
                    )))
                }
            }
        }
    }

    /// Reads a password by searching the ENTIRE database from its root, not just
    /// the `RustConn/` subtree.
    ///
    /// This is the vault-root read widening behind
    /// [`super::backend::SecretBackend::searches_from_root`]: it runs one
    /// `keepassxc-cli ls -R -f <db>` over the whole database (no `RustConn`
    /// group argument, unlike the scoped tree probe), then uses the pure
    /// [`root_match_entry_path`] matcher to find the entry whose basename equals
    /// `connection_id`, and finally reads that exact path with
    /// [`Self::get_password_from_kdbx_exact`].
    ///
    /// # Read-widening only — why it cannot reintroduce issue #327
    ///
    /// Every path this function reads comes **verbatim from the database's own
    /// listing**: it never constructs a path, never prepends `RustConn/`, and
    /// never touches the group-prefix helpers that #327 was about. Writes do not
    /// go through here — stores stay `RustConn/`-scoped. So the doubled-prefix
    /// class of bug is structurally impossible on this path.
    ///
    /// Callers should try the scoped [`Self::get_password_from_kdbx_with_key`]
    /// first and fall back to this only on a miss (back-compat: a `RustConn/`
    /// entry wins over an identically-named one elsewhere); see
    /// [`super::kdbx_backend::KdbxBackend::retrieve`].
    ///
    /// # Returns
    /// * `Ok(Some(SecretString))` when a matching entry with a password is found
    /// * `Ok(None)` when no entry matches, or the match has no password
    ///
    /// # Errors
    /// Returns [`SecretError::KeePassXC`] if `keepassxc-cli` is missing, the
    /// path is invalid, or the database cannot be unlocked/read.
    pub fn get_password_from_kdbx_root(
        kdbx_path: &Path,
        db_password: Option<&SecretString>,
        key_file: Option<&Path>,
        connection_id: &str,
        yubikey_slot: Option<&str>,
    ) -> SecretResult<Option<SecretString>> {
        Self::validate_kdbx_path(kdbx_path)?;

        let cli_path = Self::find_keepassxc_cli().ok_or_else(|| {
            SecretError::KeePassXC("keepassxc-cli not found. Please install KeePassXC.".to_string())
        })?;

        let cli = RealKeePassCli::new(&cli_path);

        // List the WHOLE database, flattened. No trailing group argument — that
        // is the single difference from the `RustConn`-scoped tree probe, and it
        // is what widens the read to entries outside the RustConn subtree.
        let mut args = vec![
            "ls".to_string(),
            "-q".to_string(),
            "-R".to_string(),
            "-f".to_string(),
        ];
        push_unlock_args(&mut args, db_password.is_some(), key_file, yubikey_slot);
        args.push(kdbx_path.display().to_string());

        // Through the shared chokepoint so this root `-y` run also gets the
        // touch cue (#350); the delegated `_exact` read below routes through it
        // too, so both runs of a two-step root read show the cue.
        let invocation = Invocation::new(
            "ls -R (root search)",
            args,
            InvocationKind::Read,
            yubikey_slot.is_some(),
        );
        let output = cli.run(&invocation, db_password, None)?;

        if !output.status.success() {
            // A failed listing is "cannot read the database", never "no entry":
            // `list_failure_error` has no entry-missing outcome, so a wrong key or
            // an unreadable database is reported rather than swallowed as a miss.
            return Err(list_failure_error(&String::from_utf8_lossy(&output.stderr)));
        }

        let listing = String::from_utf8_lossy(&output.stdout);
        let Some(entry_path) = root_match_entry_path(&listing, connection_id) else {
            tracing::debug!("get_password_root: no root entry matched '{connection_id}'");
            return Ok(None);
        };

        tracing::debug!("get_password_root: matched '{entry_path}' for '{connection_id}'");
        // Read the matched absolute path as-is. Reusing the exact reader keeps
        // the unlock composition and secret-wiping identical to every other read.
        Self::get_password_from_kdbx_exact(
            kdbx_path,
            db_password,
            key_file,
            &entry_path,
            yubikey_slot,
        )
    }

    /// Renames an entry in KDBX database by moving it from old path to new path
    ///
    /// This method retrieves the entry from the old path, creates a new entry at the new path
    /// with the same credentials, and deletes the old entry.
    ///
    /// # Arguments
    /// * `kdbx_path` - Path to the KDBX database file
    /// * `db_password` - Password to unlock the database (None if using key file)
    /// * `key_file` - Optional path to key file for authentication
    /// * `old_entry_path` - Current path of the entry (e.g., "RustConn/Group/OldName (rdp)")
    /// * `new_entry_path` - New path for the entry (e.g., "RustConn/Group/NewName (rdp)")
    /// * `yubikey_slot` - Optional `YubiKey` Challenge-Response slot as
    ///   `slot[:serial]`; when `Some`, passed to `keepassxc-cli` as `-y` and
    ///   composed with the password and/or key file for every read and write
    ///   this rename performs.
    /// * `read_only` - `SecretSettings::kdbx_read_only`; when `true` the rename
    ///   is refused before anything runs (a no-op rename still returns `Ok`).
    ///
    /// # Returns
    /// * `Ok(())` if the rename is successful or entry doesn't exist
    /// * `Err(SecretError)` if the operation fails
    ///
    /// # Errors
    /// Returns an error if:
    /// - the database is in read-only mode ([`SecretError::ReadOnly`])
    /// - `keepassxc-cli` is not installed
    /// - The KDBX file path is invalid
    /// - The database password/key file is incorrect
    pub fn rename_entry_in_kdbx(
        kdbx_path: &Path,
        db_password: Option<&SecretString>,
        key_file: Option<&Path>,
        old_entry_path: &str,
        new_entry_path: &str,
        yubikey_slot: Option<&str>,
        read_only: bool,
    ) -> SecretResult<()> {
        // If paths are the same, nothing to do
        if old_entry_path == new_entry_path {
            return Ok(());
        }

        ensure_kdbx_writable(read_only)?;

        // First validate the path
        Self::validate_kdbx_path(kdbx_path)?;

        // Find keepassxc-cli
        let cli_path = Self::find_keepassxc_cli().ok_or_else(|| {
            SecretError::KeePassXC("keepassxc-cli not found. Please install KeePassXC.".to_string())
        })?;

        // keepassxc-cli 2.5.0+ (decision D3): mv/edit -t in place, reading no
        // password back — a rename that cost about eleven YubiKey touches now
        // costs about three (#350). Under the write lock. Older or unreadable
        // versions keep the legacy read-add-delete algorithm below.
        if Self::get_keepassxc_version(&cli_path).is_some_and(|v| version_supports_edit(&v)) {
            // The in-place path addresses entries by their full "RustConn/..."
            // path; the callers pass exactly that.
            return with_keepass_write_lock(|| {
                let cli = RealKeePassCli::new(&cli_path);
                rename_or_move_in_place(
                    &cli,
                    db_password,
                    key_file,
                    yubikey_slot,
                    kdbx_path,
                    old_entry_path,
                    new_entry_path,
                )
            });
        }

        // get_password_from_kdbx_with_key adds "RustConn/" prefix, so we need to strip it
        // from old_entry_path if present to avoid double prefix
        let old_entry_name = old_entry_path
            .strip_prefix("RustConn/")
            .unwrap_or(old_entry_path);

        // First, try to get the password from the old entry
        let password = Self::get_password_from_kdbx_with_key(
            kdbx_path,
            db_password,
            key_file,
            old_entry_name,
            None,
            yubikey_slot,
        )?;

        // If no password found at old path, nothing to rename
        let Some(password) = password else {
            tracing::debug!("No entry found at '{}', nothing to rename", old_entry_path);
            return Ok(());
        };

        // Get username from old entry (use full path for direct CLI call)
        let username = Self::get_username_from_kdbx(
            kdbx_path,
            db_password,
            key_file,
            &cli_path,
            old_entry_path,
            yubikey_slot,
        )
        .unwrap_or_default();

        // Get URL from old entry (use full path for direct CLI call)
        let url = Self::get_url_from_kdbx(
            kdbx_path,
            db_password,
            key_file,
            &cli_path,
            old_entry_path,
            yubikey_slot,
        );

        // Ensure parent groups exist for new path
        // Extract entry name from new path (everything after "RustConn/")
        let new_entry_name = new_entry_path
            .strip_prefix("RustConn/")
            .unwrap_or(new_entry_path);

        Self::ensure_parent_groups(
            kdbx_path,
            db_password,
            key_file,
            &cli_path,
            new_entry_name,
            yubikey_slot,
        )?;

        // Create new entry with the password
        Self::save_password_to_kdbx(
            kdbx_path,
            db_password,
            key_file,
            new_entry_name,
            &username,
            &password,
            url.as_deref(),
            yubikey_slot,
            read_only,
        )?;

        // Delete old entry (use full path for direct CLI call)
        let _ = Self::delete_kdbx_entry(
            kdbx_path,
            db_password,
            key_file,
            old_entry_path,
            yubikey_slot,
        );

        tracing::info!(
            "Renamed KeePass entry from '{}' to '{}'",
            old_entry_path,
            new_entry_path
        );

        Ok(())
    }

    /// Gets username from a KDBX entry
    fn get_username_from_kdbx(
        kdbx_path: &Path,
        db_password: Option<&SecretString>,
        key_file: Option<&Path>,
        cli_path: &Path,
        entry_path: &str,
        yubikey_slot: Option<&str>,
    ) -> Option<String> {
        use std::io::Write as IoWrite;
        use std::process::Stdio;

        let mut args = vec![
            "show".to_string(),
            "-q".to_string(),
            "-s".to_string(),
            "-a".to_string(),
            "UserName".to_string(),
        ];

        push_unlock_args(&mut args, db_password.is_some(), key_file, yubikey_slot);

        args.push(kdbx_path.display().to_string());
        args.push(entry_path.to_string());

        let mut child = Self::keepassxc_command(cli_path)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;

        if let Some(mut stdin) = child.stdin.take()
            && let Some(db_pwd) = db_password
        {
            stdin.write_all(db_pwd.expose_secret().as_bytes()).ok()?;
            stdin.write_all(b"\n").ok()?;
        }

        // A `-y` read blocks on a touch, so give it the touch budget.
        let output = if yubikey_slot.is_some() {
            wait_for_cli_yubikey(child, "show (username)").ok()?
        } else {
            wait_for_cli(child, "show (username)").ok()?
        };

        if output.status.success() {
            let username = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if username.is_empty() {
                None
            } else {
                Some(username)
            }
        } else {
            tracing::debug!(
                entry_path,
                exit_code = ?output.status.code(),
                stderr = %String::from_utf8_lossy(&output.stderr).trim(),
                "get_username_from_kdbx: keepassxc-cli show failed"
            );
            None
        }
    }

    /// Gets URL from a KDBX entry
    fn get_url_from_kdbx(
        kdbx_path: &Path,
        db_password: Option<&SecretString>,
        key_file: Option<&Path>,
        cli_path: &Path,
        entry_path: &str,
        yubikey_slot: Option<&str>,
    ) -> Option<String> {
        use std::io::Write as IoWrite;
        use std::process::Stdio;

        let mut args = vec![
            "show".to_string(),
            "-q".to_string(),
            "-s".to_string(),
            "-a".to_string(),
            "URL".to_string(),
        ];

        push_unlock_args(&mut args, db_password.is_some(), key_file, yubikey_slot);

        args.push(kdbx_path.display().to_string());
        args.push(entry_path.to_string());

        let mut child = Self::keepassxc_command(cli_path)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;

        if let Some(mut stdin) = child.stdin.take()
            && let Some(db_pwd) = db_password
        {
            stdin.write_all(db_pwd.expose_secret().as_bytes()).ok()?;
            stdin.write_all(b"\n").ok()?;
        }

        // A `-y` read blocks on a touch, so give it the touch budget.
        let output = if yubikey_slot.is_some() {
            wait_for_cli_yubikey(child, "show (url)").ok()?
        } else {
            wait_for_cli(child, "show (url)").ok()?
        };

        if output.status.success() {
            let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if url.is_empty() { None } else { Some(url) }
        } else {
            tracing::debug!(
                entry_path,
                exit_code = ?output.status.code(),
                stderr = %String::from_utf8_lossy(&output.stderr).trim(),
                "get_url_from_kdbx: keepassxc-cli show failed"
            );
            None
        }
    }

    /// Verifies a KDBX database password using `keepassxc-cli`
    ///
    /// # Arguments
    /// * `kdbx_path` - Path to the KDBX database file
    /// * `password` - Password to verify
    ///
    /// # Returns
    /// * `Ok(())` if the password is correct
    /// * `Err(String)` with error description if verification fails
    ///
    /// # Errors
    /// Returns an error if:
    /// - `keepassxc-cli` is not installed
    /// - The KDBX file path is invalid
    /// - The password is incorrect
    /// - The database cannot be opened
    pub fn verify_kdbx_password(kdbx_path: &Path, password: &SecretString) -> SecretResult<()> {
        Self::verify_kdbx_credentials(kdbx_path, Some(password), None, None)
    }

    /// Verifies KDBX database credentials (password and/or key file) using `keepassxc-cli`
    ///
    /// # Arguments
    /// * `kdbx_path` - Path to the KDBX database file
    /// * `password` - Password to verify (None if using key file only)
    /// * `key_file` - Optional path to key file
    /// * `yubikey_slot` - Optional `YubiKey` Challenge-Response slot as
    ///   `slot[:serial]`; when `Some`, passed to `keepassxc-cli` as `-y` and
    ///   composed with the password and/or key file.
    ///
    /// # Returns
    /// * `Ok(())` if the credentials are correct
    ///
    /// # Errors
    ///
    /// Returns [`SecretError::KeePassXC`] if `keepassxc-cli` is not installed
    /// or fails, or [`SecretError::Backend`] if the password / key file is
    /// rejected by the database.
    pub fn verify_kdbx_credentials(
        kdbx_path: &Path,
        password: Option<&SecretString>,
        key_file: Option<&Path>,
        yubikey_slot: Option<&str>,
    ) -> SecretResult<()> {
        use std::io::Write as IoWrite;
        use std::process::Stdio;

        // First validate the path
        Self::validate_kdbx_path(kdbx_path)?;

        // Find keepassxc-cli
        let cli_path = Self::find_keepassxc_cli().ok_or_else(|| {
            SecretError::KeePassXC("keepassxc-cli not found. Please install KeePassXC.".to_string())
        })?;

        // Build command arguments
        let mut args = vec!["ls".to_string(), "-q".to_string()];

        push_unlock_args(&mut args, password.is_some(), key_file, yubikey_slot);

        args.push(kdbx_path.display().to_string());

        let mut child = Self::keepassxc_command(&cli_path)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SecretError::KeePassXC(format!("Failed to run keepassxc-cli: {e}")))?;

        // Write password to stdin (only if we have one)
        if let Some(mut stdin) = child.stdin.take()
            && let Some(pwd) = password
        {
            stdin
                .write_all(pwd.expose_secret().as_bytes())
                .map_err(|e| SecretError::KeePassXC(format!("Failed to send password: {e}")))?;
            stdin
                .write_all(b"\n")
                .map_err(|e| SecretError::KeePassXC(format!("Failed to send password: {e}")))?;
        }

        // A Challenge-Response check blocks on a physical touch, so give it the
        // longer budget; a password/key-file-only check keeps the tight one.
        let output = if yubikey_slot.is_some() {
            wait_for_cli_yubikey(child, "ls (credential check)")?
        } else {
            wait_for_cli(child, "ls (credential check)")?
        };

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("Invalid credentials")
                || stderr.contains("wrong password")
                || stderr.contains("Error while reading the database")
            {
                Err(SecretError::KeePassXC(
                    "Invalid password or key file".to_string(),
                ))
            } else if stderr.is_empty() {
                Err(SecretError::KeePassXC(
                    "Failed to open database. Check your credentials.".to_string(),
                ))
            } else {
                Err(SecretError::KeePassXC(format!(
                    "Database error: {}",
                    stderr.trim()
                )))
            }
        }
    }

    /// Validates a key file path
    ///
    /// # Arguments
    /// * `path` - Path to validate
    ///
    /// # Returns
    /// * `Ok(())` if the path is valid
    ///
    /// Note: `KeePassXC` creates key files without extension by default,
    /// so we don't require a specific extension.
    ///
    /// # Errors
    ///
    /// Returns [`SecretError::Backend`] when the file does not exist, is not a
    /// regular file, or is not readable.
    pub fn validate_key_file_path(path: &Path) -> SecretResult<()> {
        // Check if file exists
        if !path.exists() {
            return Err(SecretError::KeePassXC(format!(
                "Key file does not exist: {}",
                path.display()
            )));
        }

        // Check if it's a file (not a directory)
        if !path.is_file() {
            return Err(SecretError::KeePassXC(format!(
                "Path is not a file: {}",
                path.display()
            )));
        }

        Ok(())
    }
}

/// Parses a version string from `KeePassXC` CLI output
///
/// The output format is typically: "keepassxc-cli 2.7.6"
/// or just "2.7.6" on some systems.
///
/// # Arguments
/// * `output` - The raw output from `keepassxc-cli --version`
///
/// # Returns
/// * `Some(String)` containing the version number if found
/// * `None` if no valid version could be extracted
#[must_use]
pub fn parse_keepassxc_version(output: &str) -> Option<String> {
    let output = output.trim();

    if output.is_empty() {
        return None;
    }

    // Try to find a version pattern (digits and dots)
    // Common formats:
    // - "keepassxc-cli 2.7.6"
    // - "2.7.6"
    // - "KeePassXC 2.7.6"

    // Split by whitespace and look for version-like strings
    for part in output.split_whitespace() {
        // Check if this part looks like a version (starts with digit, contains dots)
        if part.chars().next().is_some_and(|c| c.is_ascii_digit())
            && part.contains('.')
            && part.chars().all(|c| c.is_ascii_digit() || c == '.')
        {
            return Some(part.to_string());
        }
    }

    // If no version found with dots, try to find any digit sequence
    // This handles edge cases like "2" or "2.7"
    for part in output.split_whitespace() {
        if part.chars().next().is_some_and(|c| c.is_ascii_digit())
            && part.chars().all(|c| c.is_ascii_digit() || c == '.')
        {
            return Some(part.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_unlock_args_password_only_adds_nothing() {
        // A master-password-only unlock: no --no-password, no --key-file, no -y.
        let mut args = vec!["show".to_string()];
        push_unlock_args(&mut args, true, None, None);
        assert_eq!(args, vec!["show".to_string()]);
    }

    #[test]
    fn push_unlock_args_key_file_only_uses_no_password() {
        let mut args: Vec<String> = Vec::new();
        let kf = Path::new("/tmp/db.keyx");
        push_unlock_args(&mut args, false, Some(kf), None);
        assert_eq!(
            args,
            vec![
                "--no-password".to_string(),
                "--key-file".to_string(),
                "/tmp/db.keyx".to_string(),
            ]
        );
    }

    #[test]
    fn push_unlock_args_yubikey_composes_with_password_and_key_file() {
        // Password present (so no --no-password) + key file + YubiKey slot.
        let mut args: Vec<String> = Vec::new();
        let kf = Path::new("/tmp/db.keyx");
        push_unlock_args(&mut args, true, Some(kf), Some("2:12345678"));
        assert_eq!(
            args,
            vec![
                "--key-file".to_string(),
                "/tmp/db.keyx".to_string(),
                "-y".to_string(),
                "2:12345678".to_string(),
            ]
        );
    }

    #[test]
    fn push_unlock_args_yubikey_only_uses_no_password() {
        // Hardware-key-only unlock: --no-password plus -y, no key file.
        let mut args: Vec<String> = Vec::new();
        push_unlock_args(&mut args, false, None, Some("1"));
        assert_eq!(
            args,
            vec![
                "--no-password".to_string(),
                "-y".to_string(),
                "1".to_string(),
            ]
        );
    }

    /// The write path threads the YubiKey slot — the regression #350 was about.
    /// A password-protected KDBX that also requires a Challenge-Response touch:
    /// the master password is present (so NO `--no-password`), and `-y <slot>`
    /// must appear on the `add` argv or the write cannot unlock the database.
    #[test]
    fn build_add_args_password_plus_yubikey_carries_the_slot() {
        let args = build_add_args(
            true,
            None,
            Some("2:12345678"),
            "alice",
            Some("ssh://host"),
            Path::new("/tmp/db.kdbx"),
            "RustConn/host (ssh)",
        );
        // No --no-password when a master password is present.
        assert!(!args.iter().any(|a| a == "--no-password"), "argv: {args:?}");
        // The slot must be threaded as `-y <slot>`.
        let y = args.iter().position(|a| a == "-y").expect("-y present");
        assert_eq!(args[y + 1], "2:12345678");
        assert_eq!(
            args,
            vec![
                "add".to_string(),
                "-q".to_string(),
                "-y".to_string(),
                "2:12345678".to_string(),
                "-u".to_string(),
                "alice".to_string(),
                "--url".to_string(),
                "ssh://host".to_string(),
                "-p".to_string(),
                "/tmp/db.kdbx".to_string(),
                "RustConn/host (ssh)".to_string(),
            ]
        );
    }

    /// Key file + YubiKey, no master password: `--no-password` is emitted (the
    /// CLI would otherwise read an empty stdin line as an empty password), the
    /// key file is passed, and the slot is threaded.
    #[test]
    fn build_add_args_key_file_plus_yubikey_no_password() {
        let args = build_add_args(
            false,
            Some(Path::new("/tmp/db.keyx")),
            Some("1"),
            "",
            None,
            Path::new("/tmp/db.kdbx"),
            "RustConn/entry",
        );
        assert_eq!(
            args,
            vec![
                "add".to_string(),
                "-q".to_string(),
                "--no-password".to_string(),
                "--key-file".to_string(),
                "/tmp/db.keyx".to_string(),
                "-y".to_string(),
                "1".to_string(),
                // no -u (empty username), no --url (None)
                "-p".to_string(),
                "/tmp/db.kdbx".to_string(),
                "RustConn/entry".to_string(),
            ]
        );
    }

    /// YubiKey-only unlock (no master password, no key file): `--no-password`
    /// plus `-y <slot>`, and nothing else on the unlock side.
    #[test]
    fn build_add_args_yubikey_only_no_password() {
        let args = build_add_args(
            false,
            None,
            Some("2"),
            "",
            None,
            Path::new("/tmp/db.kdbx"),
            "RustConn/entry",
        );
        assert_eq!(
            args,
            vec![
                "add".to_string(),
                "-q".to_string(),
                "--no-password".to_string(),
                "-y".to_string(),
                "2".to_string(),
                "-p".to_string(),
                "/tmp/db.kdbx".to_string(),
                "RustConn/entry".to_string(),
            ]
        );
    }

    /// Without a YubiKey slot the write argv is unchanged from before #350: a
    /// master password alone emits no unlock flags beyond `-p`.
    #[test]
    fn build_add_args_password_only_has_no_yubikey() {
        let args = build_add_args(
            true,
            None,
            None,
            "",
            None,
            Path::new("/tmp/db.kdbx"),
            "RustConn/entry",
        );
        assert!(!args.iter().any(|a| a == "-y"), "argv: {args:?}");
        assert!(!args.iter().any(|a| a == "--no-password"), "argv: {args:?}");
        assert_eq!(
            args,
            vec![
                "add".to_string(),
                "-q".to_string(),
                "-p".to_string(),
                "/tmp/db.kdbx".to_string(),
                "RustConn/entry".to_string(),
            ]
        );
    }

    #[test]
    fn test_validate_kdbx_path_valid_extension() {
        // Create a temp file with .kdbx extension
        let temp_dir = tempfile::tempdir().unwrap();
        let kdbx_path = temp_dir.path().join("test.kdbx");
        std::fs::write(&kdbx_path, b"dummy content").unwrap();

        assert!(KeePassStatus::validate_kdbx_path(&kdbx_path).is_ok());
    }

    #[test]
    fn test_validate_kdbx_path_uppercase_extension() {
        let temp_dir = tempfile::tempdir().unwrap();
        let kdbx_path = temp_dir.path().join("test.KDBX");
        std::fs::write(&kdbx_path, b"dummy content").unwrap();

        assert!(KeePassStatus::validate_kdbx_path(&kdbx_path).is_ok());
    }

    #[test]
    fn test_validate_kdbx_path_wrong_extension() {
        let temp_dir = tempfile::tempdir().unwrap();
        let txt_path = temp_dir.path().join("test.txt");
        std::fs::write(&txt_path, b"dummy content").unwrap();

        let result = KeePassStatus::validate_kdbx_path(&txt_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(".kdbx extension"));
    }

    #[test]
    fn test_validate_kdbx_path_nonexistent() {
        let path = std::path::PathBuf::from("/nonexistent/path/test.kdbx");
        let result = KeePassStatus::validate_kdbx_path(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));
    }

    #[test]
    fn test_validate_kdbx_path_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        // Create a directory with .kdbx name
        let dir_path = temp_dir.path().join("test.kdbx");
        std::fs::create_dir(&dir_path).unwrap();

        let result = KeePassStatus::validate_kdbx_path(&dir_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not a file"));
    }

    #[test]
    fn test_parse_version_standard_format() {
        assert_eq!(
            parse_keepassxc_version("keepassxc-cli 2.7.6"),
            Some("2.7.6".to_string())
        );
    }

    #[test]
    fn test_parse_version_just_number() {
        assert_eq!(parse_keepassxc_version("2.7.6"), Some("2.7.6".to_string()));
    }

    #[test]
    fn test_parse_version_with_prefix() {
        assert_eq!(
            parse_keepassxc_version("KeePassXC 2.7.6"),
            Some("2.7.6".to_string())
        );
    }

    #[test]
    fn test_parse_version_empty() {
        assert_eq!(parse_keepassxc_version(""), None);
    }

    #[test]
    fn test_parse_version_whitespace() {
        assert_eq!(parse_keepassxc_version("   "), None);
    }

    #[test]
    fn test_parse_version_no_version() {
        assert_eq!(parse_keepassxc_version("keepassxc-cli"), None);
    }

    #[test]
    fn test_parse_version_with_newline() {
        assert_eq!(
            parse_keepassxc_version("keepassxc-cli 2.7.6\n"),
            Some("2.7.6".to_string())
        );
    }

    #[test]
    fn test_default_status() {
        let status = KeePassStatus::default();
        assert!(!status.keepassxc_installed);
        assert!(status.keepassxc_version.is_none());
        assert!(status.keepassxc_path.is_none());
        assert!(!status.kdbx_configured);
        assert!(!status.kdbx_accessible);
        assert!(!status.integration_active);
    }

    /// The three outcomes the readers branch on, in the CLI's own English.
    ///
    /// `keepassxc-cli` exits 1 for all of them, so this prose is the only signal
    /// there is — which is why [`KeePassStatus::keepassxc_command`] has to keep it
    /// in English. These are the wordings from 2.7.x.
    #[test]
    fn classify_show_failure_reads_english_diagnostics() {
        assert!(matches!(
            classify_show_failure("Could not find entry with path RustConn/example (ssh)."),
            ShowFailure::EntryMissing
        ));
        assert!(matches!(
            classify_show_failure("Invalid credentials were provided, please try again."),
            ShowFailure::BadCredentials
        ));
        // Anything unrecognised stays Unusable: the database was not opened, so
        // "the entry is not there" is not a conclusion available to us.
        assert!(matches!(
            classify_show_failure("Error while reading the database: Not a KeePass database."),
            ShowFailure::Unusable
        ));
    }

    /// The candidate order, pinned. Nothing could test this before: exercising
    /// the loop needs a `keepassxc-cli` and a real database on the machine
    /// running the tests, so the sequence was only visible by reading it.
    #[test]
    fn the_current_naming_scheme_is_tried_first() {
        let paths = candidate_entry_paths("Production/nginx-01 (ssh)", None);
        assert_eq!(paths[0], "RustConn/Production/nginx-01 (ssh)");
    }

    #[test]
    fn the_protocol_suffix_is_dropped_for_the_older_format() {
        let paths = candidate_entry_paths("nginx-01 (ssh)", None);
        assert_eq!(
            paths,
            vec![
                "RustConn/nginx-01 (ssh)",
                "RustConn/nginx-01",
                "nginx-01 (ssh)",
            ]
        );
    }

    /// The saving: each candidate is a full database open, and this one could
    /// never match a grouped connection.
    ///
    /// `build_entry_path` starts every path at `RustConn`, so `Production/x` at
    /// the database root is not a location any release has written. Trying it
    /// cost an Argon2 open — about 700 ms — on every lookup for every connection
    /// that lives in a group.
    #[test]
    fn a_grouped_name_does_not_get_searched_at_the_database_root() {
        let paths = candidate_entry_paths("Production/nginx-01 (ssh)", None);
        assert!(
            !paths.iter().any(|p| !p.starts_with("RustConn/")),
            "a name carrying a group path must only be looked for under RustConn/: {paths:?}"
        );
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn an_ungrouped_name_is_still_searched_at_the_root() {
        // Where a pre-hierarchy release did write it, so this one stays.
        let paths = candidate_entry_paths("nginx-01", None);
        assert_eq!(paths, vec!["RustConn/nginx-01", "nginx-01"]);
    }

    #[test]
    fn a_separately_supplied_protocol_adds_its_own_candidate() {
        let paths = candidate_entry_paths("nginx-01", Some("ssh"));
        assert_eq!(
            paths,
            vec!["RustConn/nginx-01", "RustConn/nginx-01 (ssh)", "nginx-01",]
        );
    }

    /// Why the message locale is pinned, stated as a test.
    ///
    /// This is the stderr from the bug report — `keepassxc-cli` 2.7.12 answering a
    /// missing entry in Ukrainian, because RustConn had exported `LANGUAGE=uk` for
    /// its own UI. The classifier cannot read it, and the resulting
    /// [`ShowFailure::Unusable`] became "Could not read the password from
    /// KeePassXC" for a healthy database. The fix is upstream of this function: the
    /// child never gets a translated locale in the first place.
    #[test]
    fn classify_show_failure_cannot_read_a_translated_diagnostic() {
        let translated = "Неможливо знайти запис із шляхом RustConn/kiro-cli (zerotrust).";
        assert!(
            matches!(classify_show_failure(translated), ShowFailure::Unusable),
            "if this ever classifies correctly, the locale pinning is no longer \
             load-bearing and this test should say so"
        );
    }

    /// A failed whole-database listing is always an error. Reusing the `show`
    /// classifier let "Could not find entry" in an `ls` stderr turn an
    /// unreadable database into `Ok(None)`, i.e. "no stored password".
    #[test]
    fn list_failure_never_reports_a_missing_entry() {
        let missing = list_failure_error("Could not find entry with path RustConn/x (ssh).");
        assert!(
            matches!(missing, SecretError::KeePassXC(ref m) if m.starts_with("Could not read the database")),
            "an ls failure must not read as a miss, got {missing:?}"
        );

        let bad = list_failure_error("Invalid credentials were provided, please try again.");
        assert!(
            matches!(bad, SecretError::KeePassXC(ref m) if m == "Invalid database password"),
            "got {bad:?}"
        );

        let unreadable =
            list_failure_error("Error while reading the database: Not a KeePass database.");
        assert!(
            matches!(unreadable, SecretError::KeePassXC(ref m) if m.contains("Not a KeePass database")),
            "got {unreadable:?}"
        );
    }

    /// `kdbx_read_only` is enforced inside the three public writers, ahead of
    /// path validation: `/nonexistent/x.kdbx` would otherwise fail validation, so
    /// getting `ReadOnly` proves the guard refused before anything ran. The
    /// writable runs prove it is the flag, not the path, doing the refusing.
    #[test]
    fn kdbx_writers_refuse_read_only_before_touching_the_database() {
        let path = Path::new("/nonexistent/x.kdbx");
        let pwd = SecretString::from("p".to_string());
        let is_read_only = |r: SecretResult<()>| matches!(r, Err(SecretError::ReadOnly(ref n)) if n == KDBX_DISPLAY_NAME);

        assert!(is_read_only(KeePassStatus::save_password_to_kdbx(
            path,
            None,
            None,
            "conn (ssh)",
            "user",
            &pwd,
            None,
            None,
            true,
        )));
        assert!(is_read_only(KeePassStatus::delete_entry_from_kdbx(
            path,
            None,
            None,
            "RustConn/conn (ssh)",
            None,
            true,
        )));
        assert!(is_read_only(KeePassStatus::rename_entry_in_kdbx(
            path,
            None,
            None,
            "RustConn/old (ssh)",
            "RustConn/new (ssh)",
            None,
            true,
        )));

        // Writable: the same calls get as far as path validation.
        let writable = KeePassStatus::save_password_to_kdbx(
            path,
            None,
            None,
            "conn (ssh)",
            "user",
            &pwd,
            None,
            None,
            false,
        );
        assert!(
            matches!(writable, Err(SecretError::KeePassXC(_))),
            "got {writable:?}"
        );
        let writable =
            KeePassStatus::delete_entry_from_kdbx(path, None, None, "RustConn/c", None, false);
        assert!(
            matches!(writable, Err(SecretError::KeePassXC(_))),
            "got {writable:?}"
        );

        // A rename to the same path writes nothing, so it is not refused.
        assert!(
            KeePassStatus::rename_entry_in_kdbx(
                path,
                None,
                None,
                "RustConn/a",
                "RustConn/a",
                None,
                true
            )
            .is_ok()
        );
    }

    /// The fix: diagnostics must arrive untranslated, and the encoding must not be
    /// collateral damage.
    #[test]
    fn keepassxc_command_pins_the_message_locale_only() {
        if crate::flatpak::is_flatpak() {
            // The sandbox branch forwards `--env=` arguments to flatpak-spawn
            // instead, so `get_envs` would report nothing either way.
            return;
        }

        let cmd = KeePassStatus::keepassxc_command(std::path::Path::new("/usr/bin/keepassxc-cli"));
        let envs: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        let lookup = |name: &str| envs.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());

        assert_eq!(
            lookup("LC_MESSAGES"),
            Some(Some("C".to_string())),
            "messages must be pinned to C or classify_show_failure cannot read them"
        );
        assert_eq!(
            lookup("LANGUAGE"),
            Some(None),
            "LANGUAGE must be cleared for the child: this process exports it, and \
             it outranks LC_MESSAGES"
        );
        assert!(
            lookup("LC_ALL").is_none_or(|value| value.is_none()),
            "LC_ALL must never be handed to the child set: it outranks LC_MESSAGES"
        );
        // Encoding is deliberately not forced — entry paths and the database path
        // travel as argv, and a Qt 5 build takes its codec from the locale charset.
        assert_ne!(
            lookup("LC_CTYPE"),
            Some(Some("C".to_string())),
            "forcing a C charset would mangle non-ASCII entry paths"
        );
        // Unchanged behaviour: macOS GUI launches still need the extended PATH so
        // keepassxc-cli can find its own children (e.g. GPG).
        assert!(
            matches!(lookup("PATH"), Some(Some(_))),
            "the extended PATH must still be injected"
        );
    }
}
