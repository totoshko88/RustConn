//! Background CLI detection for secret backends.
//!
//! All functions in this module are `Send` and perform no GTK calls,
//! making them safe to run on a background thread.

use crate::i18n::{i18n, i18n_f};

/// Results of background CLI detection for all secret backends
#[derive(Clone)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "settings/flags struct mirrors persisted config 1:1; bools represent independent toggles, not a state machine"
)]
pub(super) struct SecretCliDetection {
    pub keepassxc_version: Option<String>,
    pub bitwarden_installed: bool,
    pub bitwarden_cmd: String,
    pub bitwarden_version: Option<String>,
    pub bitwarden_status: Option<(String, &'static str)>,
    pub onepassword_installed: bool,
    pub onepassword_cmd: String,
    pub onepassword_version: Option<String>,
    pub onepassword_status: Option<(String, &'static str)>,
    pub passbolt_installed: bool,
    pub passbolt_version: Option<String>,
    pub passbolt_status: Option<(String, &'static str)>,
    pub passbolt_server_url: Option<String>,
    pub pass_version: Option<String>,
    pub pass_status: Option<(String, &'static str)>,
    /// Whether `secret-tool` binary is available (for keyring operations)
    pub secret_tool_available: bool,
    /// Fine-grained availability of the platform system-keyring backend
    /// (libsecret/Secret Service on Linux/BSD, Keychain on macOS). Lets the
    /// Secrets tab show whether the keyring is genuinely usable, not just
    /// whether the client binary exists (#201).
    pub system_keyring_availability: rustconn_core::secret::BackendAvailability,
}

/// Cached detection result: probing spawns ~10 child processes, so reuse
/// the result when the settings dialog is reopened shortly after.
/// Vault lock/unlock actions in the dialog refresh their status labels
/// directly (not through this cache), so staleness is bounded to reopen.
static DETECTION_CACHE: std::sync::Mutex<Option<(std::time::Instant, SecretCliDetection)>> =
    std::sync::Mutex::new(None);

/// 30s keeps reopen instant while bounding stale backend status.
const DETECTION_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// Runs all secret backend CLI detection on a background thread.
/// This function is `Send` and performs no GTK calls.
///
/// Results are cached for [`DETECTION_CACHE_TTL`]; independent backends are
/// probed in parallel so total latency equals the slowest probe, not the sum.
pub(super) fn detect_secret_backends() -> SecretCliDetection {
    if let Ok(guard) = DETECTION_CACHE.lock()
        && let Some((detected_at, cached)) = guard.as_ref()
        && detected_at.elapsed() < DETECTION_CACHE_TTL
    {
        return cached.clone();
    }

    let detection = run_detection();

    if let Ok(mut guard) = DETECTION_CACHE.lock() {
        *guard = Some((std::time::Instant::now(), detection.clone()));
    }
    detection
}

/// Probes every backend in parallel scoped threads.
///
/// Each probe only spawns short-lived child processes (`--version`,
/// `status`), so a panic is a programming bug; in that case the backend is
/// reported as not installed rather than poisoning the whole detection.
fn run_detection() -> SecretCliDetection {
    std::thread::scope(|scope| {
        let keepassxc = scope.spawn(detect_keepassxc);
        let bitwarden = scope.spawn(detect_bitwarden);
        let onepassword = scope.spawn(detect_onepassword);
        let passbolt = scope.spawn(detect_passbolt);
        let pass = scope.spawn(detect_pass);
        let secret_tool = scope.spawn(detect_secret_tool);
        let keyring_avail = scope.spawn(detect_system_keyring_availability);

        let keepassxc_version = keepassxc.join().unwrap_or_default();
        let (bitwarden_installed, bitwarden_cmd, bitwarden_version, bitwarden_status) = bitwarden
            .join()
            .unwrap_or_else(|_| (false, "bw".to_string(), None, None));
        let (onepassword_installed, onepassword_cmd, onepassword_version, onepassword_status) =
            onepassword
                .join()
                .unwrap_or_else(|_| (false, "op".to_string(), None, None));
        let (passbolt_installed, passbolt_version, passbolt_status, passbolt_server_url) =
            passbolt.join().unwrap_or_default();
        let (pass_version, pass_status) = pass.join().unwrap_or_default();
        let secret_tool_available = secret_tool.join().unwrap_or_default();
        let system_keyring_availability = keyring_avail
            .join()
            .unwrap_or(rustconn_core::secret::BackendAvailability::ServiceUnavailable);

        SecretCliDetection {
            keepassxc_version,
            bitwarden_installed,
            bitwarden_cmd,
            bitwarden_version,
            bitwarden_status,
            onepassword_installed,
            onepassword_cmd,
            onepassword_version,
            onepassword_status,
            passbolt_installed,
            passbolt_version,
            passbolt_status,
            passbolt_server_url,
            pass_version,
            pass_status,
            secret_tool_available,
            system_keyring_availability,
        }
    })
}

/// Detects the KeePassXC CLI version.
///
/// Delegates to the core detector, which resolves `keepassxc-cli` on the host
/// via `flatpak-spawn --host` when running inside a Flatpak sandbox (#182).
fn detect_keepassxc() -> Option<String> {
    rustconn_core::secret::KeePassStatus::detect().keepassxc_version
}

/// Detects the Bitwarden CLI: `(installed, cmd, version, status)`
fn detect_bitwarden() -> (bool, String, Option<String>, Option<(String, &'static str)>) {
    let mut bw_paths: Vec<String> = vec!["bw".to_string()];
    if !rustconn_core::flatpak::is_flatpak() {
        bw_paths.extend(["/snap/bin/bw".to_string(), "/usr/local/bin/bw".to_string()]);
    }
    if let Some(cli_dir) = rustconn_core::cli_download::get_cli_install_dir() {
        let flatpak_bw = cli_dir.join("bitwarden").join("bw");
        if flatpak_bw.exists() {
            bw_paths.push(flatpak_bw.to_string_lossy().to_string());
        }
    }
    let mut bitwarden_installed = false;
    let mut bitwarden_cmd = "bw".to_string();
    for path in &bw_paths {
        if std::process::Command::new(path)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            bitwarden_installed = true;
            bitwarden_cmd = path.clone();
            break;
        }
    }
    if !bitwarden_installed
        && let Ok(output) = std::process::Command::new("which").arg("bw").output()
        && output.status.success()
    {
        bitwarden_installed = true;
        bitwarden_cmd = String::from_utf8_lossy(&output.stdout).trim().to_string();
    }
    let bitwarden_version = if bitwarden_installed {
        get_cli_version(&bitwarden_cmd, &["--version"])
    } else {
        None
    };
    let bitwarden_status = if bitwarden_installed {
        Some(check_bitwarden_status_sync(&bitwarden_cmd))
    } else {
        None
    };

    (
        bitwarden_installed,
        bitwarden_cmd,
        bitwarden_version,
        bitwarden_status,
    )
}

/// Detects the 1Password CLI: `(installed, cmd, version, status)`
fn detect_onepassword() -> (bool, String, Option<String>, Option<(String, &'static str)>) {
    let mut op_paths: Vec<String> = vec!["op".to_string()];
    if !rustconn_core::flatpak::is_flatpak() {
        op_paths.push("/usr/local/bin/op".to_string());
    }
    if let Some(cli_dir) = rustconn_core::cli_download::get_cli_install_dir() {
        let flatpak_op = cli_dir.join("1password").join("op");
        if flatpak_op.exists() {
            op_paths.push(flatpak_op.to_string_lossy().to_string());
        }
    }
    let mut onepassword_installed = false;
    let mut onepassword_cmd = "op".to_string();
    for path in &op_paths {
        if std::process::Command::new(path)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            onepassword_installed = true;
            onepassword_cmd = path.clone();
            break;
        }
    }
    if !onepassword_installed
        && let Ok(output) = std::process::Command::new("which").arg("op").output()
        && output.status.success()
    {
        onepassword_installed = true;
        onepassword_cmd = String::from_utf8_lossy(&output.stdout).trim().to_string();
    }
    let onepassword_version = if onepassword_installed {
        get_cli_version(&onepassword_cmd, &["--version"])
    } else {
        None
    };
    let onepassword_status = if onepassword_installed {
        Some(check_onepassword_status_sync(&onepassword_cmd))
    } else {
        None
    };

    (
        onepassword_installed,
        onepassword_cmd,
        onepassword_version,
        onepassword_status,
    )
}

/// Detects the Passbolt CLI: `(installed, version, status, server_url)`
fn detect_passbolt() -> (
    bool,
    Option<String>,
    Option<(String, &'static str)>,
    Option<String>,
) {
    let mut passbolt_paths: Vec<String> = vec!["passbolt".to_string()];
    if !rustconn_core::flatpak::is_flatpak() {
        passbolt_paths.push("/usr/local/bin/passbolt".to_string());
    }
    if let Some(cli_dir) = rustconn_core::cli_download::get_cli_install_dir() {
        let flatpak_pb = cli_dir.join("passbolt").join("passbolt");
        if flatpak_pb.exists() {
            passbolt_paths.push(flatpak_pb.to_string_lossy().to_string());
        }
    }
    let mut passbolt_installed = false;
    for path in &passbolt_paths {
        if std::process::Command::new(path)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            passbolt_installed = true;
            break;
        }
    }
    if !passbolt_installed
        && let Ok(output) = std::process::Command::new("which").arg("passbolt").output()
        && output.status.success()
    {
        passbolt_installed = true;
    }
    let passbolt_version = if passbolt_installed {
        get_cli_version("passbolt", &["--version"])
    } else {
        None
    };
    let passbolt_status = if passbolt_installed {
        Some(check_passbolt_status_sync())
    } else {
        None
    };
    let passbolt_server_url = read_passbolt_server_url_sync();

    (
        passbolt_installed,
        passbolt_version,
        passbolt_status,
        passbolt_server_url,
    )
}

/// Detects the `pass` password store: `(version, status)`
fn detect_pass() -> (Option<String>, Option<(String, &'static str)>) {
    let pass_version = if let Ok(output) = std::process::Command::new("pass")
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
    {
        if output.status.success() {
            let version_str = String::from_utf8_lossy(&output.stdout);
            // Extract version number from output like "v1.7.4"
            // Find the line containing 'v' followed by digits
            version_str
                .lines()
                .find(|line| line.contains('v') && line.chars().any(|c| c.is_ascii_digit()))
                .and_then(|line| {
                    // Extract just the version part: find 'v' and capture digits/dots after it
                    line.split_whitespace()
                        .find(|word| {
                            word.starts_with('v')
                                && word[1..].chars().next().is_some_and(|c| c.is_ascii_digit())
                        })
                        .map(|v| v.trim_start_matches('v').to_string())
                })
        } else {
            None
        }
    } else {
        None
    };

    let pass_status = if pass_version.is_some() {
        // Check if password store is initialized
        let store_dir = std::env::var("PASSWORD_STORE_DIR").ok().or_else(|| {
            dirs::home_dir().map(|h| h.join(".password-store").to_string_lossy().to_string())
        });

        if let Some(dir) = store_dir {
            let store_path = std::path::PathBuf::from(&dir);
            if store_path.exists() && store_path.join(".gpg-id").exists() {
                Some((
                    i18n_f("Initialized at {}", &[&store_path.display().to_string()]),
                    "success",
                ))
            } else {
                Some((
                    i18n("Not initialized (run 'pass init &lt;gpg-id&gt;')"),
                    "warning",
                ))
            }
        } else {
            Some((i18n("Cannot determine store directory"), "error"))
        }
    } else {
        None
    };

    (pass_version, pass_status)
}

/// Checks `secret-tool` availability (for system keyring operations)
fn detect_secret_tool() -> bool {
    std::process::Command::new("which")
        .arg("secret-tool")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Probes the platform system-keyring backend for fine-grained availability.
///
/// Runs the same read-only probe the keyring backend uses (`availability()`),
/// so the Secrets tab can show whether the keyring is genuinely usable —
/// distinguishing a missing client from an unresponsive Secret Service (#201) —
/// rather than only whether the client binary exists. Bounded by the same 5s
/// budget as the startup `has_secret_backend` check.
fn detect_system_keyring_availability() -> rustconn_core::secret::BackendAvailability {
    use rustconn_core::secret::{BackendAvailability, SecretBackend};

    // 5s mirrors the startup availability budget (R2.4).
    const KEYRING_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    crate::async_utils::with_runtime(|rt| {
        rt.block_on(async {
            #[cfg(target_os = "macos")]
            let probe = {
                let backend = rustconn_core::secret::MacOsKeychainBackend::new();
                tokio::time::timeout(KEYRING_PROBE_TIMEOUT, async move {
                    backend.availability().await
                })
                .await
            };
            #[cfg(not(target_os = "macos"))]
            let probe = {
                let backend = rustconn_core::secret::LibSecretBackend::new("rustconn");
                tokio::time::timeout(KEYRING_PROBE_TIMEOUT, async move {
                    backend.availability().await
                })
                .await
            };
            probe.unwrap_or(BackendAvailability::ServiceUnavailable)
        })
    })
    .unwrap_or(BackendAvailability::ServiceUnavailable)
}

/// Gets CLI version from command output
fn get_cli_version(command: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(command)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            let output = String::from_utf8_lossy(&o.stdout);
            parse_version(&output)
        })
}

/// Parses version from output string
fn parse_version(output: &str) -> Option<String> {
    rustconn_core::secret::VERSION_REGEX
        .captures(output)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Checks Bitwarden vault status synchronously
pub(super) fn check_bitwarden_status_sync(bw_cmd: &str) -> (String, &'static str) {
    let output = std::process::Command::new(bw_cmd).arg("status").output();

    match output {
        Ok(o) if o.status.success() => {
            let status_str = String::from_utf8_lossy(&o.stdout);
            if let Ok(status) = serde_json::from_str::<serde_json::Value>(&status_str)
                && let Some(status_val) = status.get("status").and_then(|v| v.as_str())
            {
                return match status_val {
                    "unlocked" => (i18n("Unlocked"), "success"),
                    "locked" => (i18n("Locked"), "warning"),
                    "unauthenticated" => (i18n("Not logged in"), "error"),
                    _ => (i18n_f("Status: {}", &[status_val]), "dim-label"),
                };
            }
            (i18n("Unknown"), "dim-label")
        }
        _ => (i18n("Error checking status"), "error"),
    }
}

/// Checks 1Password account status synchronously
fn check_onepassword_status_sync(op_cmd: &str) -> (String, &'static str) {
    let output = std::process::Command::new(op_cmd)
        .args(["whoami", "--format", "json"])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            if let Ok(whoami) = serde_json::from_str::<serde_json::Value>(&stdout)
                && let Some(email) = whoami.get("email").and_then(|v| v.as_str())
            {
                return (i18n_f("Signed in: {}", &[email]), "success");
            }
            (i18n("Signed in"), "success")
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            if stderr.contains("not signed in") || stderr.contains("sign in") {
                (i18n("Not signed in"), "error")
            } else if stderr.contains("session expired") {
                (i18n("Session expired"), "warning")
            } else {
                (i18n("Not signed in"), "error")
            }
        }
        Err(_) => (i18n("Error checking status"), "error"),
    }
}

/// Checks Passbolt CLI configuration status synchronously
fn check_passbolt_status_sync() -> (String, &'static str) {
    let output = std::process::Command::new("passbolt")
        .args(["list", "user", "--json"])
        .output();

    match output {
        Ok(o) if o.status.success() => (i18n("Configured"), "success"),
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            if stderr.contains("no configuration") {
                (i18n("Not configured"), "error")
            } else if stderr.contains("authentication") || stderr.contains("passphrase") {
                (i18n("Authentication failed"), "warning")
            } else {
                (i18n("Not configured"), "error")
            }
        }
        Err(_) => (i18n("Error checking status"), "error"),
    }
}

/// Reads the Passbolt server URL from the CLI configuration file (sync)
pub(super) fn read_passbolt_server_url_sync() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let config_path = std::path::PathBuf::from(home)
        .join(".config")
        .join("go-passbolt-cli")
        .join("config.json");

    let content = std::fs::read_to_string(config_path).ok()?;
    let config: serde_json::Value = serde_json::from_str(&content).ok()?;
    config
        .get("serverAddress")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Extracts session key from `bw unlock` output
pub(super) fn extract_session_key(output: &str) -> Option<String> {
    // Output format: export BW_SESSION="<session_key>"
    // or: $ export BW_SESSION="<session_key>"
    for line in output.lines() {
        if line.contains("BW_SESSION=") {
            // Extract the value between quotes
            if let Some(start) = line.find('"')
                && let Some(end) = line.rfind('"')
                && end > start
            {
                return Some(line[start + 1..end].to_string());
            }
            // Try without quotes (BW_SESSION=value)
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
