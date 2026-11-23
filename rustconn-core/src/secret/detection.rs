//! Secret backend detection and version checking
//!
//! This module provides utilities for detecting installed password managers
//! and their versions, useful for UI display and backend selection.

use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use tokio::process::Command;

/// Creates a [`Command`] with the extended PATH that includes Homebrew,
/// Flatpak CLI directories, and other platform-specific tool locations.
///
/// On macOS, GUI apps launched via `.app` bundle inherit a minimal PATH
/// (`/usr/bin:/bin:/usr/sbin:/sbin`), so tools installed via Homebrew or
/// in custom locations are invisible to bare `Command::new("tool")`.
/// This helper ensures all detection commands can find CLI tools regardless
/// of how the application was launched.
fn detection_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.env("PATH", crate::cli_download::get_extended_path());
    // Required for [`probe`] to mean anything: dropping a `tokio::process::Child`
    // does not kill it, so a probe abandoned at its deadline would leave the very
    // unresponsive process it gave up on still running, once per refresh of the
    // Secrets tab.
    cmd.kill_on_drop(true);
    cmd
}

/// How long any single detection probe is given before it is abandoned.
///
/// None of these probes had a deadline, and they are not independent:
/// [`detect_password_managers`] runs all eight detectors under one
/// `tokio::join!`, which completes only when the last of them does, so one
/// unresponsive CLI stalls the whole call. `bw status` reaches the network once
/// its session has expired, `op whoami` can sit on a biometric prompt and
/// `passbolt list` talks to a server — the three most likely to stall are exactly
/// the three that do I/O.
///
/// Worth knowing before assuming this fixed the Secrets page: **nothing in this
/// workspace calls these detectors.** They are public and re-exported from
/// `secret::mod`, and the GUI has its own synchronous copies in
/// `rustconn/src/dialogs/settings/secrets_tab/detection.rs`, driven from a
/// `std::thread::scope`, and those are where the page actually stalled. They were
/// bounded in the same change. Two parallel detector implementations is the real
/// defect here and neither this constant nor that one fixes it.
///
/// The one argument for keeping the async set was that an external consumer of
/// the crate might depend on it. Checked on the Linux validation pass for 0.21.0
/// and it does not hold: neither `rustconn-core` nor `rustconn` is published —
/// `https://crates.io/api/v1/crates/rustconn-core` and the same URL for
/// `rustconn` both answer 404 — and no `publish` key appears in any manifest, so
/// there is no released library surface for anything to have been written
/// against. Together with having no in-workspace caller, that makes this module
/// dead code rather than an API, and the duplication resolves by deleting it
/// and keeping the synchronous copies the GUI actually runs.
///
/// Not deleted in 0.21.0 on purpose: it is roughly six hundred lines plus their
/// tests, and 0.21.0 was already prepared and awaiting a tag when the evidence
/// was gathered. Doing it here would put an unreviewed deletion of that size
/// into a release the maintainer is about to publish. The decision is made and
/// the evidence is above; the removal is a 0.22 change. What must *not* happen
/// is a later session re-deriving the crates.io question — that part is settled.
///
/// Five seconds rather than the ten a vault operation gets: this is a `--version`
/// or a status check, nobody is waiting on a credential, and the answer is only
/// used to populate a row. A probe that gives up reads as "not installed", which
/// is already what an errored probe reads as and is the honest answer when a CLI
/// will not say otherwise.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs a detection probe, giving up rather than blocking the caller.
///
/// Whether `flatpak info <app_id>` reports the application as installed.
///
/// Synchronous, because its two callers are: they build a launch command from a
/// GTK action, where an unbounded `flatpak` would freeze the window. Bounded
/// through [`crate::proc::wait_bounded`] rather than `tokio::time::timeout`,
/// which needs a runtime and an async caller.
///
/// Output is discarded rather than piped. Only the exit status is read, and a
/// child whose pipe nobody drains can block before it exits, which would spend
/// the budget for no reason.
fn flatpak_app_installed(app_id: &str, extended_path: &str, what: &'static str) -> bool {
    let child = std::process::Command::new("flatpak")
        .env("PATH", extended_path)
        .args(["info", app_id])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    match child {
        Ok(child) => crate::proc::wait_bounded(child, PROBE_TIMEOUT, what)
            .is_ok_and(|waited| waited.succeeded()),
        Err(e) => {
            tracing::debug!(probe = what, %e, "flatpak probe could not run");
            false
        }
    }
}

/// Runs a detection probe, giving up rather than blocking the caller.
///
/// `what` names the probe in the log line a timeout produces, and is
/// `&'static str` so a caller cannot interpolate a path or a credential into it.
async fn probe(cmd: &mut Command, what: &'static str) -> Option<std::process::Output> {
    match tokio::time::timeout(PROBE_TIMEOUT, cmd.output()).await {
        Ok(Ok(output)) => Some(output),
        Ok(Err(e)) => {
            tracing::debug!(probe = what, %e, "detection probe could not run");
            None
        }
        Err(_) => {
            tracing::warn!(
                probe = what,
                timeout_secs = PROBE_TIMEOUT.as_secs(),
                "detection probe timed out; reporting the manager as unavailable"
            );
            None
        }
    }
}

/// Cached regex for version parsing: matches patterns like "1.2.3" or "v1.2.3"
pub static VERSION_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"v?(\d+\.\d+(?:\.\d+)?)").expect("VERSION_REGEX is a valid regex pattern")
});

/// Information about an installed password manager
#[derive(Debug, Clone)]
pub struct PasswordManagerInfo {
    /// Unique identifier
    pub id: &'static str,
    /// Display name
    pub name: &'static str,
    /// Version string (if detected)
    pub version: Option<String>,
    /// Whether the manager is installed/available
    pub installed: bool,
    /// Whether it's currently running (for socket-based backends)
    pub running: bool,
    /// Path to executable or database
    pub path: Option<PathBuf>,
    /// Additional status message
    pub status_message: Option<String>,
    /// Supported formats (e.g., "KDBX 4", "Secret Service API")
    pub formats: Vec<&'static str>,
}

/// Detects all available password managers on the system
pub async fn detect_password_managers() -> Vec<PasswordManagerInfo> {
    let (keepassxc, gnome_secrets, libsecret, bitwarden, onepassword, keepass, passbolt, pass) = tokio::join!(
        detect_keepassxc(),
        detect_gnome_secrets(),
        detect_libsecret(),
        detect_bitwarden(),
        detect_onepassword(),
        detect_keepass(),
        detect_passbolt(),
        detect_pass(),
    );

    vec![
        keepassxc,
        gnome_secrets,
        libsecret,
        bitwarden,
        onepassword,
        keepass,
        passbolt,
        pass,
    ]
}

/// Detects KeePassXC installation and status
pub async fn detect_keepassxc() -> PasswordManagerInfo {
    let mut info = PasswordManagerInfo {
        id: "keepassxc",
        name: "KeePassXC",
        version: None,
        installed: false,
        running: false,
        path: None,
        status_message: None,
        formats: vec!["KDBX 3", "KDBX 4"],
    };

    // Check keepassxc-cli availability (extended PATH covers Homebrew, .app bundle, etc.)
    if let Some(output) = probe(
        detection_command("keepassxc-cli").arg("--version"),
        "keepassxc-cli --version",
    )
    .await
        && output.status.success()
    {
        let version_str = String::from_utf8_lossy(&output.stdout);
        info.version = parse_version_line(&version_str);
        info.installed = true;
    }

    // Check if KeePassXC is running (socket exists)
    let socket_path = std::env::var("XDG_RUNTIME_DIR")
        .map(|dir| PathBuf::from(dir).join("kpxc_server"))
        .unwrap_or_else(|_| PathBuf::from("/tmp/kpxc_server"));

    if socket_path.exists() {
        info.running = true;
        info.status_message = Some("Browser integration active".to_string());
    } else if info.installed {
        info.status_message = Some("Not running or browser integration disabled".to_string());
    }

    // Find executable path
    if let Some(path) = crate::which::find_in_path("keepassxc") {
        info.path = Some(path);
    }

    info
}

/// Detects GNOME Secrets (Password Safe) installation
pub async fn detect_gnome_secrets() -> PasswordManagerInfo {
    let mut info = PasswordManagerInfo {
        id: "gnome-secrets",
        name: "GNOME Secrets",
        version: None,
        installed: false,
        running: false,
        path: None,
        status_message: None,
        formats: vec!["KDBX 4"],
    };

    // Check for flatpak installation
    if let Some(output) = probe(
        detection_command("flatpak").args(["info", "org.gnome.World.Secrets"]),
        "flatpak info (GNOME Secrets)",
    )
    .await
        && output.status.success()
    {
        let output_str = String::from_utf8_lossy(&output.stdout);
        info.version = parse_flatpak_version(&output_str);
        info.installed = true;
        info.path = Some(PathBuf::from("flatpak:org.gnome.World.Secrets"));
    }

    // Check for native installation
    if !info.installed
        && let Some(path) = crate::which::find_in_path("gnome-secrets")
    {
        info.installed = true;
        info.path = Some(path);
    }

    // Also check for old name (gnome-passwordsafe)
    if !info.installed
        && let Some(path) = crate::which::find_in_path("gnome-passwordsafe")
    {
        info.installed = true;
        info.path = Some(path);
    }

    if info.installed {
        info.status_message = Some("Uses KDBX format (compatible with KeePass)".to_string());
    }

    info
}

/// Detects libsecret/secret-tool availability
pub async fn detect_libsecret() -> PasswordManagerInfo {
    let mut info = PasswordManagerInfo {
        id: "libsecret",
        name: "GNOME Keyring / KDE Wallet",
        version: None,
        installed: false,
        running: false,
        path: None,
        status_message: None,
        formats: vec!["Secret Service API"],
    };

    // Check secret-tool
    if let Some(output) = probe(
        detection_command("secret-tool").arg("--version"),
        "secret-tool --version",
    )
    .await
        && output.status.success()
    {
        let version_str = String::from_utf8_lossy(&output.stdout);
        info.version = parse_version_line(&version_str);
        info.installed = true;
    }

    // Check if gnome-keyring-daemon is running
    if let Some(output) = probe(
        detection_command("pgrep").arg("gnome-keyring-d"),
        "pgrep gnome-keyring-daemon",
    )
    .await
        && output.status.success()
    {
        info.running = true;
        info.status_message = Some("GNOME Keyring daemon running".to_string());
    }

    // Check if kwalletd is running (KDE)
    if !info.running
        && let Some(output) =
            probe(detection_command("pgrep").arg("kwalletd"), "pgrep kwalletd").await
        && output.status.success()
    {
        info.running = true;
        info.status_message = Some("KDE Wallet daemon running".to_string());
    }

    if info.installed && !info.running {
        info.status_message = Some("No keyring daemon detected".to_string());
    }

    info
}

/// Detects Bitwarden CLI installation
pub async fn detect_bitwarden() -> PasswordManagerInfo {
    let mut info = PasswordManagerInfo {
        id: "bitwarden",
        name: "Bitwarden CLI",
        version: None,
        installed: false,
        running: false,
        path: None,
        status_message: None,
        formats: vec!["Cloud or self-hosted vault"],
    };

    // Try common paths for bw CLI
    let bw_paths = ["bw", "/usr/bin/bw", "/usr/local/bin/bw", "/snap/bin/bw"];

    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let extra_paths = [
        format!("{home}/.local/bin/bw"),
        format!("{home}/.npm-global/bin/bw"),
        format!("{home}/bin/bw"),
        format!("{home}/.nvm/versions/node/*/bin/bw"),
    ];

    let mut bw_cmd: Option<String> = None;

    // Try standard paths first
    for path in &bw_paths {
        if let Some(output) = probe(detection_command(path).arg("--version"), "cli --version").await
            && output.status.success()
        {
            let version_str = String::from_utf8_lossy(&output.stdout);
            info.version = Some(version_str.trim().to_string());
            info.installed = true;
            bw_cmd = Some((*path).to_string());
            break;
        }
    }

    // Try home-relative paths
    if !info.installed {
        for path in &extra_paths {
            // Skip glob patterns
            if path.contains('*') {
                continue;
            }
            if let Some(output) =
                probe(detection_command(path).arg("--version"), "cli --version").await
                && output.status.success()
            {
                let version_str = String::from_utf8_lossy(&output.stdout);
                info.version = Some(version_str.trim().to_string());
                info.installed = true;
                bw_cmd = Some(path.clone());
                break;
            }
        }
    }

    // Check login status
    if let Some(ref cmd) = bw_cmd {
        if let Some(output) = probe(detection_command(cmd).arg("status"), "bw status").await
            && output.status.success()
        {
            let status_str = String::from_utf8_lossy(&output.stdout);
            if let Ok(status) = serde_json::from_str::<serde_json::Value>(&status_str)
                && let Some(status_val) = status.get("status").and_then(|v| v.as_str())
            {
                match status_val {
                    "unlocked" => {
                        info.running = true;
                        info.status_message = Some("Vault unlocked".to_string());
                    }
                    "locked" => {
                        info.status_message = Some("Vault locked".to_string());
                    }
                    "unauthenticated" => {
                        info.status_message = Some("Not logged in".to_string());
                    }
                    _ => {
                        info.status_message = Some(format!("Status: {status_val}"));
                    }
                }
            }
        }
        info.path = Some(PathBuf::from(cmd));
    }

    // If still not found, look it up on PATH
    if !info.installed
        && let Some(path) = crate::which::find_in_path("bw")
    {
        info.path = Some(path.clone());
        // Try to get version from found path
        if let Some(ver_output) = probe(
            detection_command(&path.to_string_lossy()).arg("--version"),
            "resolved CLI --version",
        )
        .await
            && ver_output.status.success()
        {
            let version_str = String::from_utf8_lossy(&ver_output.stdout);
            info.version = Some(version_str.trim().to_string());
            info.installed = true;
        }
    }

    if !info.installed {
        info.status_message = Some("Login with 'bw login' in terminal first".to_string());
    }

    info
}

/// Detects original KeePass (via kpcli or keepass2)
pub async fn detect_keepass() -> PasswordManagerInfo {
    let mut info = PasswordManagerInfo {
        id: "keepass",
        name: "KeePass",
        version: None,
        installed: false,
        running: false,
        path: None,
        status_message: None,
        formats: vec!["KDBX 3", "KDBX 4", "KDB"],
    };

    // Check kpcli (Perl CLI for KeePass)
    if let Some(output) = probe(
        detection_command("kpcli").arg("--version"),
        "kpcli --version",
    )
    .await
        && output.status.success()
    {
        let version_str = String::from_utf8_lossy(&output.stdout);
        info.version = parse_version_line(&version_str);
        info.installed = true;
        info.status_message = Some("kpcli available".to_string());
    }

    // Check keepass2 (Mono/.NET version)
    if !info.installed
        && let Some(path) = crate::which::find_in_path("keepass2")
    {
        info.installed = true;
        info.path = Some(path);
        info.status_message = Some("KeePass 2 (Mono) available".to_string());
    }

    info
}

/// Detects 1Password CLI installation and status
pub async fn detect_onepassword() -> PasswordManagerInfo {
    let mut info = PasswordManagerInfo {
        id: "onepassword",
        name: "1Password CLI",
        version: None,
        installed: false,
        running: false,
        path: None,
        status_message: None,
        formats: vec!["Cloud or self-hosted vault"],
    };

    // Try common paths for op CLI
    let op_paths = ["op", "/usr/bin/op", "/usr/local/bin/op"];

    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let extra_paths = [format!("{home}/.local/bin/op"), format!("{home}/bin/op")];

    let mut op_cmd: Option<String> = None;

    // Try standard paths first
    for path in &op_paths {
        if let Some(output) = probe(detection_command(path).arg("--version"), "cli --version").await
            && output.status.success()
        {
            let version_str = String::from_utf8_lossy(&output.stdout);
            info.version = Some(version_str.trim().to_string());
            info.installed = true;
            op_cmd = Some((*path).to_string());
            break;
        }
    }

    // Try home-relative paths
    if !info.installed {
        for path in &extra_paths {
            if let Some(output) =
                probe(detection_command(path).arg("--version"), "cli --version").await
                && output.status.success()
            {
                let version_str = String::from_utf8_lossy(&output.stdout);
                info.version = Some(version_str.trim().to_string());
                info.installed = true;
                op_cmd = Some(path.clone());
                break;
            }
        }
    }

    // Check signin status using whoami
    if let Some(ref cmd) = op_cmd {
        if let Some(output) = probe(
            detection_command(cmd).args(["whoami", "--format", "json"]),
            "op whoami",
        )
        .await
        {
            if output.status.success() {
                info.running = true;
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Ok(whoami) = serde_json::from_str::<serde_json::Value>(&stdout) {
                    if let Some(email) = whoami.get("email").and_then(|v| v.as_str()) {
                        info.status_message = Some(format!("Signed in as {email}"));
                    } else {
                        info.status_message = Some("Signed in".to_string());
                    }
                } else {
                    info.status_message = Some("Signed in".to_string());
                }
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("not signed in") || stderr.contains("sign in") {
                    info.status_message = Some("Not signed in".to_string());
                } else if stderr.contains("session expired") {
                    info.status_message = Some("Session expired".to_string());
                } else {
                    info.status_message = Some("Not signed in".to_string());
                }
            }
        }
        info.path = Some(PathBuf::from(cmd));
    }

    // If still not found, look it up on PATH
    if !info.installed
        && let Some(path) = crate::which::find_in_path("op")
    {
        info.path = Some(path.clone());
        // Try to get version from found path
        if let Some(ver_output) = probe(
            detection_command(&path.to_string_lossy()).arg("--version"),
            "resolved CLI --version",
        )
        .await
            && ver_output.status.success()
        {
            let version_str = String::from_utf8_lossy(&ver_output.stdout);
            info.version = Some(version_str.trim().to_string());
            info.installed = true;
        }
    }

    if !info.installed {
        info.status_message =
            Some("Install from https://1password.com/downloads/command-line".to_string());
    }

    info
}

/// Detects Passbolt CLI installation and status
pub async fn detect_passbolt() -> PasswordManagerInfo {
    let mut info = PasswordManagerInfo {
        id: "passbolt",
        name: "Passbolt CLI",
        version: None,
        installed: false,
        running: false,
        path: None,
        status_message: None,
        formats: vec!["Server-based team vault"],
    };

    let passbolt_paths = ["passbolt", "/usr/bin/passbolt", "/usr/local/bin/passbolt"];

    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let extra_paths = [
        format!("{home}/.local/bin/passbolt"),
        format!("{home}/go/bin/passbolt"),
        format!("{home}/go/bin/go-passbolt-cli"),
    ];

    let mut pb_cmd: Option<String> = None;

    for path in &passbolt_paths {
        if let Some(output) = probe(detection_command(path).arg("--version"), "cli --version").await
            && output.status.success()
        {
            let version_str = String::from_utf8_lossy(&output.stdout);
            info.version = Some(version_str.trim().to_string());
            info.installed = true;
            pb_cmd = Some((*path).to_string());
            break;
        }
    }

    if !info.installed {
        for path in &extra_paths {
            if let Some(output) =
                probe(detection_command(path).arg("--version"), "cli --version").await
                && output.status.success()
            {
                let version_str = String::from_utf8_lossy(&output.stdout);
                info.version = Some(version_str.trim().to_string());
                info.installed = true;
                pb_cmd = Some(path.clone());
                break;
            }
        }
    }

    // Check if configured by listing users
    if let Some(ref cmd) = pb_cmd {
        if let Some(output) = probe(
            detection_command(cmd).args(["list", "user", "--json"]),
            "passbolt list user",
        )
        .await
        {
            if output.status.success() {
                info.running = true;
                info.status_message = Some("Configured".to_string());
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("no configuration") {
                    info.status_message = Some("Not configured".to_string());
                } else if stderr.contains("authentication") || stderr.contains("passphrase") {
                    info.status_message = Some("Authentication failed".to_string());
                } else {
                    info.status_message = Some("Not configured".to_string());
                }
            }
        }
        info.path = Some(PathBuf::from(cmd));
    }

    // Fall back to a PATH lookup
    if !info.installed
        && let Some(path) = crate::which::find_in_path("passbolt")
    {
        info.path = Some(path.clone());
        if let Some(ver_output) = probe(
            detection_command(&path.to_string_lossy()).arg("--version"),
            "resolved CLI --version",
        )
        .await
            && ver_output.status.success()
        {
            let version_str = String::from_utf8_lossy(&ver_output.stdout);
            info.version = Some(version_str.trim().to_string());
            info.installed = true;
        }
    }

    if !info.installed {
        info.status_message = Some(
            "Install from \
             https://github.com/passbolt/go-passbolt-cli"
                .to_string(),
        );
    }

    info
}

/// Detects Pass (Unix password manager) installation
pub async fn detect_pass() -> PasswordManagerInfo {
    let mut info = PasswordManagerInfo {
        id: "pass",
        name: "Pass (passwordstore)",
        version: None,
        installed: false,
        running: false,
        path: None,
        status_message: None,
        formats: vec!["GPG-encrypted files"],
    };

    let pass_paths = ["pass", "/usr/bin/pass", "/usr/local/bin/pass"];

    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let extra_paths = [format!("{home}/.local/bin/pass")];

    let mut pass_cmd: Option<String> = None;

    // Try all paths (standard paths + extra paths) in a single iterator chain
    for path in pass_paths
        .iter()
        .map(std::string::ToString::to_string)
        .chain(extra_paths.iter().cloned())
    {
        if let Some(output) =
            probe(detection_command(&path).arg("--version"), "cli --version").await
            && output.status.success()
        {
            let version_str = String::from_utf8_lossy(&output.stdout);
            // Pass --version outputs a banner with version in the middle
            // Look for a line containing "v" followed by version numbers
            for line in version_str.lines() {
                if let Some(version) = parse_version_line(line) {
                    info.version = Some(version);
                    break;
                }
            }
            info.installed = true;
            pass_cmd = Some(path);
            break;
        }
    }

    // Check if password store is initialized
    if let Some(ref cmd) = pass_cmd {
        let store_dir = std::env::var("PASSWORD_STORE_DIR")
            .unwrap_or_else(|_| format!("{home}/.password-store"));

        let store_path = PathBuf::from(&store_dir);
        if store_path.exists() && store_path.join(".gpg-id").exists() {
            info.running = true;
            info.status_message = Some(format!("Initialized at {}", store_path.display()));
        } else {
            info.status_message =
                Some("Not initialized (run 'pass init &lt;gpg-id&gt;')".to_string());
        }
        info.path = Some(PathBuf::from(cmd));
    }

    // Fall back to a PATH lookup
    if !info.installed
        && let Some(path) = crate::which::find_in_path("pass")
    {
        info.path = Some(path.clone());
        if let Some(ver_output) = probe(
            detection_command(&path.to_string_lossy()).arg("--version"),
            "resolved CLI --version",
        )
        .await
            && ver_output.status.success()
        {
            let version_str = String::from_utf8_lossy(&ver_output.stdout);
            // Look for a line containing version numbers
            for line in version_str.lines() {
                if let Some(version) = parse_version_line(line) {
                    info.version = Some(version);
                    break;
                }
            }
            info.installed = true;
        }
    }

    if !info.installed {
        info.status_message = Some("Install from https://www.passwordstore.org/".to_string());
    }

    info
}

/// Parses version from a typical version output line
fn parse_version_line(output: &str) -> Option<String> {
    VERSION_REGEX
        .captures(output)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Parses version from flatpak info output
fn parse_flatpak_version(output: &str) -> Option<String> {
    for line in output.lines() {
        if line.trim().starts_with("Version:") {
            return Some(line.trim().strip_prefix("Version:")?.trim().to_string());
        }
    }
    None
}

/// Returns the platform-specific command to open a URL or file.
/// On macOS this is `open`, on Linux/other — `xdg-open`.
pub fn url_open_command() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "open"
    }
    #[cfg(not(target_os = "macos"))]
    {
        "xdg-open"
    }
}

/// Returns the command to open the password manager application
///
/// # Arguments
/// * `backend` - The secret backend type
/// * `passbolt_server_url` - Optional Passbolt server URL from settings
///
/// # Returns
/// A tuple of (command, args) to launch the password manager, or None
pub fn get_password_manager_launch_command(
    backend: &crate::config::SecretBackendType,
    passbolt_server_url: Option<&str>,
) -> Option<(String, Vec<String>)> {
    let extended_path = crate::cli_download::get_extended_path();

    match backend {
        crate::config::SecretBackendType::KeePassXc
        | crate::config::SecretBackendType::KdbxFile => {
            // Try KeePassXC first
            if crate::which::is_available("keepassxc") {
                return Some(("keepassxc".to_string(), vec![]));
            }
            // Try GNOME Secrets (flatpak)
            if flatpak_app_installed(
                "org.gnome.World.Secrets",
                &extended_path,
                "flatpak info (GNOME Secrets launch)",
            ) {
                return Some((
                    "flatpak".to_string(),
                    vec!["run".to_string(), "org.gnome.World.Secrets".to_string()],
                ));
            }
            // Try gnome-secrets native
            if crate::which::is_available("gnome-secrets") {
                return Some(("gnome-secrets".to_string(), vec![]));
            }
            // Try KeePass 2
            if crate::which::is_available("keepass2") {
                return Some(("keepass2".to_string(), vec![]));
            }
            None
        }
        crate::config::SecretBackendType::LibSecret => {
            // Open Seahorse (GNOME Passwords and Keys)
            if crate::which::is_available("seahorse") {
                return Some(("seahorse".to_string(), vec![]));
            }
            // Try GNOME Settings privacy section
            if crate::which::is_available("gnome-control-center") {
                return Some((
                    "gnome-control-center".to_string(),
                    vec!["privacy".to_string()],
                ));
            }
            // Try KDE Wallet Manager
            if crate::which::is_available("kwalletmanager5") {
                return Some(("kwalletmanager5".to_string(), vec![]));
            }
            None
        }
        crate::config::SecretBackendType::Bitwarden => {
            // Open Bitwarden web vault in default browser
            Some((
                url_open_command().to_string(),
                vec!["https://vault.bitwarden.com".to_string()],
            ))
        }
        crate::config::SecretBackendType::OnePassword => {
            // Try 1Password desktop app first
            if crate::which::is_available("1password") {
                return Some(("1password".to_string(), vec![]));
            }
            // Try flatpak version
            if flatpak_app_installed(
                "com.onepassword.OnePassword",
                &extended_path,
                "flatpak info (1Password launch)",
            ) {
                return Some((
                    "flatpak".to_string(),
                    vec!["run".to_string(), "com.onepassword.OnePassword".to_string()],
                ));
            }
            // Fallback to web vault
            Some((
                url_open_command().to_string(),
                vec!["https://my.1password.com".to_string()],
            ))
        }
        crate::config::SecretBackendType::Passbolt => {
            // Passbolt is web-based, open configured server URL in browser
            let url = passbolt_server_url
                .filter(|u| !u.is_empty())
                .unwrap_or("https://passbolt.local");
            Some((url_open_command().to_string(), vec![url.to_string()]))
        }
        crate::config::SecretBackendType::Pass => {
            // Try qtpass first (popular GUI for pass)
            if crate::which::is_available("qtpass") {
                return Some(("qtpass".to_string(), vec![]));
            }
            // Fallback: open store directory in file manager
            let home = dirs::home_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let store_dir = std::env::var("PASSWORD_STORE_DIR")
                .unwrap_or_else(|_| format!("{home}/.password-store"));
            Some((url_open_command().to_string(), vec![store_dir]))
        }
        crate::config::SecretBackendType::ProtonPass => {
            Some(proton_pass_launch_command(&extended_path))
        }
        crate::config::SecretBackendType::MacOsKeychain => {
            // Open Keychain Access on macOS
            Some((
                "open".to_string(),
                vec!["-a".to_string(), "Keychain Access".to_string()],
            ))
        }
        crate::config::SecretBackendType::EncryptedFile => {
            // Application-managed file: no external password-manager app to
            // launch. (Correct as-is; not a 2.5 placeholder.)
            None
        }
        crate::config::SecretBackendType::PortableEncryptedFile => {
            // Portable encrypted file: no external app to launch.
            None
        }
    }
}

/// The Proton Pass desktop app (native package), then the Flathub one, then
/// the web vault.
fn proton_pass_launch_command(extended_path: &str) -> (String, Vec<String>) {
    if crate::which::is_available("proton-pass") {
        return ("proton-pass".to_string(), vec![]);
    }
    if flatpak_app_installed(
        "me.proton.Pass",
        extended_path,
        "flatpak info (Proton Pass launch)",
    ) {
        return (
            "flatpak".to_string(),
            vec!["run".to_string(), "me.proton.Pass".to_string()],
        );
    }
    (
        url_open_command().to_string(),
        vec!["https://pass.proton.me".to_string()],
    )
}

/// Opens the password manager application for the given backend
///
/// # Arguments
/// * `backend` - The secret backend type
/// * `passbolt_server_url` - Optional Passbolt server URL from settings
///
/// # Returns
/// Ok(()) if launched successfully
///
/// # Errors
/// Returns error message if no password manager is found or launch fails
pub fn open_password_manager(
    backend: &crate::config::SecretBackendType,
    passbolt_server_url: Option<&str>,
) -> Result<(), String> {
    let Some((cmd, args)) = get_password_manager_launch_command(backend, passbolt_server_url)
    else {
        return Err("No password manager application found".to_string());
    };

    std::process::Command::new(&cmd)
        .env("PATH", crate::cli_download::get_extended_path())
        .args(&args)
        .spawn()
        .map_err(|e| format!("Failed to launch {cmd}: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_version_line() {
        assert_eq!(
            parse_version_line("KeePassXC 2.7.6"),
            Some("2.7.6".to_string())
        );
        assert_eq!(
            parse_version_line("secret-tool 0.19.1"),
            Some("0.19.1".to_string())
        );
        assert_eq!(parse_version_line("v1.2.3"), Some("1.2.3".to_string()));
        assert_eq!(parse_version_line("no version"), None);
    }

    #[test]
    fn test_parse_flatpak_version() {
        let output = "ID: org.gnome.World.Secrets\nVersion: 9.0\nBranch: stable";
        assert_eq!(parse_flatpak_version(output), Some("9.0".to_string()));
    }
}
