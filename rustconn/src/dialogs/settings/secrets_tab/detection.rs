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
pub(crate) struct SecretCliDetection {
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

/// Whether the selected backend can actually store and read a password.
///
/// The Secrets page already showed a version number and, for some backends, a
/// status line — but the version row answered "is the client installed", which is
/// the least interesting of the prerequisites, and the status line existed for
/// four backends out of eight. Nothing anywhere answered the question the user is
/// really asking when they pick from that list. This is that answer, in one shape
/// for every row, so the page can show one line per backend instead of a
/// different arrangement per backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BackendReadiness {
    /// Usable right now.
    ///
    /// Carries a detail when the probe learned something worth repeating — which
    /// account is signed in, where the store lives — and an empty string when
    /// there is nothing to add beyond "ready".
    Ready(String),
    /// The client is there, but something has to happen before it will work —
    /// logging in, unlocking, initialising a store, choosing a file.
    ///
    /// This is the state the three-variant `BackendAvailability` cannot express,
    /// and the reason a logged-out Bitwarden looks the same to the startup check
    /// as a working one: `is_available()` is a bool, and a CLI that runs at all
    /// answers `true`.
    NeedsAction(String),
    /// The client program is not installed, so nothing can be done in RustConn.
    NotInstalled,
    /// Detection has not finished yet.
    Unknown,
}

impl BackendReadiness {
    /// The line to show the user.
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Ready(detail) if detail.is_empty() => i18n("Ready"),
            Self::Ready(detail) | Self::NeedsAction(detail) => detail.clone(),
            Self::NotInstalled => i18n("Not installed"),
            Self::Unknown => i18n("Checking…"),
        }
    }

    /// The css class to render this verdict with.
    ///
    /// The label always names the state as well, so status is never carried by
    /// colour alone (GNOME HIG / WCAG).
    pub(crate) const fn css_class(&self) -> &'static str {
        match self {
            Self::Ready(_) => "success",
            Self::NeedsAction(_) => "warning",
            Self::NotInstalled => "error",
            Self::Unknown => "dim-label",
        }
    }

    /// Whether saving a password to this backend can be expected to work.
    ///
    /// `Unknown` counts as usable: an unfinished probe is not evidence of a
    /// problem, and treating a `--version` call that has not returned yet as a
    /// fault would make the page argue with the user on a slow machine.
    pub(crate) const fn is_usable(&self) -> bool {
        matches!(self, Self::Ready(_) | Self::Unknown)
    }
}

/// The parts of a backend's readiness that live in the dialog, not in a probe.
///
/// Read from the widgets rather than from the saved `SecretSettings`, and that is
/// the point: someone who has just chosen a database file has not saved it yet,
/// so a verdict computed from the stored configuration would report the state
/// they are in the middle of leaving. Three fields because these are the only
/// prerequisites a probe cannot see.
pub(crate) struct LocalBackendState {
    /// The "Use KeePass integration" switch.
    pub kdbx_enabled: bool,
    /// The database path currently in the entry, expanded.
    pub kdbx_path: Option<std::path::PathBuf>,
    /// Whether the portable file's passphrase field has anything in it.
    pub portable_passphrase_entered: bool,
}

impl LocalBackendState {
    /// Reads the same three prerequisites out of saved settings.
    ///
    /// For callers outside the dialog — the startup check and the post-save
    /// re-check — where there are no widgets to read and the saved configuration
    /// *is* the state in force.
    pub(crate) fn from_settings(secrets: &rustconn_core::config::SecretSettings) -> Self {
        Self {
            kdbx_enabled: secrets.kdbx_enabled,
            kdbx_path: secrets.kdbx_path.clone(),
            portable_passphrase_entered: secrets.portable_passphrase.is_some(),
        }
    }
}

/// Renders the readiness of `backend` from a finished detection pass.
///
/// `detection` is `None` while the background probe is still running, which is
/// the only source of [`BackendReadiness::Unknown`].
///
/// The per-backend status strings this reuses are the ones the page already
/// computed and displayed; what is new is that every backend produces a verdict,
/// including the four that previously had no status line at all — the two file
/// backends, KeePassXC and the system keyring, whose row existed but was shown
/// for one selection only.
pub(crate) fn backend_readiness(
    detection: Option<&SecretCliDetection>,
    backend: rustconn_core::config::SecretBackendType,
    local: &LocalBackendState,
) -> BackendReadiness {
    use rustconn_core::config::SecretBackendType;
    use rustconn_core::secret::BackendAvailability;

    // `detection` is consulted per arm rather than unwrapped up here, and that is
    // load-bearing: the two file backends answer from `local` alone, so a single
    // early return would report "Checking..." for a verdict no probe can change.
    // It is also what lets [`backend_needs_probe`] be checkable against this match
    // rather than a second, driftable copy of it.
    let from_status = |status: Option<&(String, &'static str)>| match status {
        // Keep the detail — "Signed in: someone@example.com" and
        // "Initialized at /home/…/.password-store" tell the user which account
        // and which store, which is worth more than a bare "Ready".
        //
        // The status pairs carry a css class alongside the text, and the class is
        // already the backend's own verdict: "success" means usable, "warning"
        // means a step is missing, "error" means it cannot work. Reading it keeps
        // this function from re-deriving conclusions the probes already reached.
        Some((text, "success")) => BackendReadiness::Ready(text.clone()),
        Some((text, _)) => BackendReadiness::NeedsAction(text.clone()),
        None => BackendReadiness::NotInstalled,
    };
    let probed = |pick: fn(&SecretCliDetection) -> Option<&(String, &'static str)>| {
        detection.map_or(BackendReadiness::Unknown, |det| from_status(pick(det)))
    };

    match backend {
        SecretBackendType::Bitwarden => probed(|det| det.bitwarden_status.as_ref()),
        SecretBackendType::OnePassword => probed(|det| det.onepassword_status.as_ref()),
        SecretBackendType::Passbolt => probed(|det| det.passbolt_status.as_ref()),
        SecretBackendType::Pass => probed(|det| det.pass_status.as_ref()),

        SecretBackendType::LibSecret | SecretBackendType::MacOsKeychain => {
            let Some(det) = detection else {
                return BackendReadiness::Unknown;
            };
            match det.system_keyring_availability {
                BackendAvailability::Available => BackendReadiness::Ready(String::new()),
                BackendAvailability::ServiceUnavailable => {
                    BackendReadiness::NeedsAction(i18n("No keyring service responding"))
                }
                BackendAvailability::ClientMissing => BackendReadiness::NotInstalled,
            }
        }

        // KeePassXC needs three things and the page only ever reported the first.
        // A database that is not configured is the common case for someone who
        // has just selected the backend, and it is not "not installed".
        SecretBackendType::KeePassXc | SecretBackendType::KdbxFile => {
            let Some(det) = detection else {
                return BackendReadiness::Unknown;
            };
            if det.keepassxc_version.is_none() {
                return BackendReadiness::NotInstalled;
            }
            if !local.kdbx_enabled {
                return BackendReadiness::NeedsAction(i18n("Turn on KeePass integration below"));
            }
            match local.kdbx_path.as_ref() {
                None => BackendReadiness::NeedsAction(i18n("Choose a database file below")),
                Some(path) if !path.exists() => BackendReadiness::NeedsAction(i18n_f(
                    "Database file not found: {}",
                    &[&path.display().to_string()],
                )),
                Some(_) => BackendReadiness::Ready(String::new()),
            }
        }

        // Needs nothing outside RustConn — the key is derived from the machine.
        SecretBackendType::EncryptedFile => BackendReadiness::Ready(String::new()),

        // Usable once a passphrase has been supplied this session. Whether one
        // has is session state the page does not hold, so this reports the part
        // it can see: a passphrase is configured or it is not.
        SecretBackendType::PortableEncryptedFile => {
            if local.portable_passphrase_entered {
                BackendReadiness::Ready(String::new())
            } else {
                BackendReadiness::NeedsAction(i18n("Enter the file's passphrase below"))
            }
        }
    }
}

/// Whether `backend`'s readiness verdict depends on [`detect_secret_backends`].
///
/// False for the two file backends: their prerequisites are all local, so no
/// probe can change the answer. The startup check in `app.rs` uses this to skip
/// the probe entirely, which matters because the probe is seven short-lived child
/// processes with a 5-second ceiling and it now runs on every launch — paying that
/// for a verdict that is a constant is pure cost. The Secrets page still probes
/// unconditionally, because it has eight rows to fill and the user is looking at
/// them.
///
/// This is a second reading of the same variants [`backend_readiness`] matches on,
/// so `a_backend_that_needs_no_probe_answers_without_one` pins the two together: a
/// backend named here that does consult the detection would report "Checking..."
/// forever, which is the bug the Passbolt and Pass status labels already had.
pub(crate) const fn backend_needs_probe(backend: rustconn_core::config::SecretBackendType) -> bool {
    use rustconn_core::config::SecretBackendType;

    !matches!(
        backend,
        SecretBackendType::EncryptedFile | SecretBackendType::PortableEncryptedFile
    )
}

/// Cached detection result: probing spawns ~10 child processes, so reuse
/// the result when the settings dialog is reopened shortly after.
/// Vault lock/unlock actions in the dialog refresh their status labels
/// directly (not through this cache), so staleness is bounded to reopen.
static DETECTION_CACHE: std::sync::Mutex<
    Option<(std::time::Instant, Option<String>, SecretCliDetection)>,
> = std::sync::Mutex::new(None);

/// 30s keeps reopen instant while bounding stale backend status.
const DETECTION_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// Runs all secret backend CLI detection on a background thread.
/// This function is `Send` and performs no GTK calls.
///
/// Results are cached for [`DETECTION_CACHE_TTL`]; independent backends are
/// probed in parallel so total latency equals the slowest probe, not the sum.
pub(crate) fn detect_secret_backends(pass_store_dir: Option<String>) -> SecretCliDetection {
    // The `pass` store directory is part of the cache key, not just an argument.
    // The cache is one slot, so without this a probe made with the configured
    // directory would be served to a caller asking about a different one — and the
    // 30-second window is exactly long enough to cover someone changing the
    // directory and looking at the Status row.
    if let Ok(guard) = DETECTION_CACHE.lock()
        && let Some((detected_at, probed_store_dir, cached)) = guard.as_ref()
        && detected_at.elapsed() < DETECTION_CACHE_TTL
        && *probed_store_dir == pass_store_dir
    {
        return cached.clone();
    }

    let detection = run_detection(pass_store_dir.as_deref());

    if let Ok(mut guard) = DETECTION_CACHE.lock() {
        *guard = Some((std::time::Instant::now(), pass_store_dir, detection.clone()));
    }
    detection
}

/// Probes every backend in parallel scoped threads.
///
/// Each probe only spawns short-lived child processes (`--version`,
/// `status`), so a panic is a programming bug; in that case the backend is
/// reported as not installed rather than poisoning the whole detection.
fn run_detection(pass_store_dir: Option<&str>) -> SecretCliDetection {
    std::thread::scope(|scope| {
        let keepassxc = scope.spawn(detect_keepassxc);
        let bitwarden = scope.spawn(detect_bitwarden);
        let onepassword = scope.spawn(detect_onepassword);
        let passbolt = scope.spawn(detect_passbolt);
        let pass = scope.spawn(move || detect_pass(pass_store_dir));
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

/// How long any single CLI probe on this page is given before it is killed.
///
/// None of them had a deadline, and the shape of `run_detection` is what made
/// that expensive: the probes are scoped threads and the scope is not joined
/// until the last of them returns, so one unresponsive CLI kept the whole
/// Secrets page empty rather than costing it one row. The three most likely to
/// stall are the three that do I/O — `bw status` reaches the network once its
/// session has expired, `op whoami` can sit on a biometric prompt, and
/// `passbolt list user` talks to a server.
///
/// Five seconds: nobody is waiting on a credential here, the answers only
/// populate rows, and a probe that gives up reads as "not installed", which is
/// already what a failed probe reads as.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Runs one CLI probe, returning `None` rather than blocking the page.
///
/// `what` goes into a log line, so it is `&'static str`: a `&str` would let a
/// caller interpolate a resolved path — and these paths include `$HOME` — or a
/// credential into it.
fn probe(cmd: &mut std::process::Command, what: &'static str) -> Option<std::process::Output> {
    let child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    rustconn_core::proc::wait_bounded(child, PROBE_TIMEOUT, what)
        .ok()?
        .output()
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
        if probe(
            std::process::Command::new(path).arg("--version"),
            "bw --version",
        )
        .is_some_and(|output| output.status.success())
        {
            bitwarden_installed = true;
            bitwarden_cmd = path.clone();
            break;
        }
    }
    if !bitwarden_installed && let Some(path) = rustconn_core::which::find_in_path("bw") {
        bitwarden_installed = true;
        bitwarden_cmd = path.display().to_string();
    }
    let bitwarden_version = if bitwarden_installed {
        get_cli_version(&bitwarden_cmd, &["--version"])
    } else {
        None
    };
    let bitwarden_status = if bitwarden_installed {
        Some(check_bitwarden_status_sync(&bitwarden_cmd).to_status_pair())
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
        if probe(
            std::process::Command::new(path).arg("--version"),
            "op --version",
        )
        .is_some_and(|output| output.status.success())
        {
            onepassword_installed = true;
            onepassword_cmd = path.clone();
            break;
        }
    }
    if !onepassword_installed && let Some(path) = rustconn_core::which::find_in_path("op") {
        onepassword_installed = true;
        onepassword_cmd = path.display().to_string();
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
        if probe(
            std::process::Command::new(path).arg("--version"),
            "passbolt --version",
        )
        .is_some_and(|output| output.status.success())
        {
            passbolt_installed = true;
            break;
        }
    }
    if !passbolt_installed && rustconn_core::which::is_available("passbolt") {
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
///
/// `configured_store_dir` is the directory the user has set for this backend, if
/// any — it has to be passed in because the probe runs on a background thread and
/// the value lives in a GTK entry.
fn detect_pass(
    configured_store_dir: Option<&str>,
) -> (Option<String>, Option<(String, &'static str)>) {
    let pass_version = if let Some(output) = probe(
        std::process::Command::new("pass").arg("--version"),
        "pass --version",
    ) {
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
        // Which store to look at, in the same order the backend resolves it:
        // the directory configured in this very dialog first, then
        // `$PASSWORD_STORE_DIR`, then pass's own default.
        //
        // The configured value was missing from this list, so the probe read the
        // *ambient* environment while `PassBackend::setup_command` puts the
        // configured directory into the child's environment. A user with a custom
        // store was therefore told "Not initialized (run 'pass init <gpg-id>')"
        // about a healthy store, or "Initialized at ~/.password-store" while the
        // backend read somewhere else entirely. That verdict is not cosmetic — it
        // feeds `BackendReadiness::is_usable`.
        let store_dir = configured_store_dir
            .map(|dir| dir.to_string())
            .or_else(|| std::env::var("PASSWORD_STORE_DIR").ok())
            .or_else(|| {
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
    rustconn_core::which::is_available("secret-tool")
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
    probe(
        std::process::Command::new(command).args(args),
        "cli --version",
    )
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

/// What `bw status` reported about the vault.
///
/// A typed verdict, because the display string this used to return was the thing
/// two callers made a decision on: both compared it against the literal
/// `"Locked"` to decide whether to attempt an unlock, and the string had already
/// been through [`i18n`]. In Italian it is `Bloccato`, in Ukrainian
/// `Заблоковано`, and neither equals `"Locked"` — so outside an English locale the
/// comparison said "not locked", the unlock was skipped, and the Secrets page kept
/// reporting a locked vault it was holding the master password for
/// (issue [#312](https://github.com/totoshko88/RustConn/issues/312)). The verdict
/// and its presentation are now separate things: this enum decides, and
/// [`Self::to_status_pair`] renders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum BwVaultStatus {
    /// Unlocked, and the CLI has the vault key it needs to answer lookups.
    Unlocked,
    /// Logged in, but locked. The one state an unlock attempt can change.
    Locked,
    /// No account is logged in, so unlocking cannot help — `bw login` has to
    /// happen first.
    Unauthenticated,
    /// `bw status` named a state this build does not know. The raw value is kept
    /// so the page can show what was actually said.
    Other(String),
    /// `bw status` ran but produced nothing this could read as a status.
    Unparseable,
    /// The probe did not run, or exceeded [`PROBE_TIMEOUT`].
    ProbeFailed,
}

impl BwVaultStatus {
    /// Renders the translated label and css class the Secrets page shows.
    pub(super) fn to_status_pair(&self) -> (String, &'static str) {
        match self {
            Self::Unlocked => (i18n("Unlocked"), "success"),
            Self::Locked => (i18n("Locked"), "warning"),
            Self::Unauthenticated => (i18n("Not logged in"), "error"),
            Self::Other(raw) => (i18n_f("Status: {}", &[raw.as_str()]), "dim-label"),
            Self::Unparseable => (i18n("Unknown"), "dim-label"),
            Self::ProbeFailed => (i18n("Error checking status"), "error"),
        }
    }

    /// Whether to attempt an unlock, given that a master password is in hand.
    ///
    /// [`Self::Locked`] is the obvious yes. The two inconclusive verdicts are
    /// also yes, and that is the deliberate part: [`Self::ProbeFailed`] means
    /// `bw status` did not answer within [`PROBE_TIMEOUT`], which is five seconds
    /// against a call measured at 2.5 s on the reporter's machine in issue
    /// [#312](https://github.com/totoshko88/RustConn/issues/312) — a slower link
    /// turns a locked vault into "probe failed", and treating that as "nothing to
    /// do" reproduces the same silent skip through a different door.
    /// [`Self::Unparseable`] is the same argument: `bw` ran and said something
    /// unreadable, which is not evidence that the vault is fine.
    ///
    /// An attempt against an already-unlocked vault costs one `bw unlock` and
    /// returns a fresh session key, so guessing wrong here is cheap. The two
    /// verdicts that stay `false` are the ones where it cannot help:
    /// [`Self::Unlocked`] needs nothing, and [`Self::Unauthenticated`] needs
    /// `bw login` — no password will unlock a CLI with no account.
    /// [`Self::Other`] is a state this build does not recognise, so it is left
    /// alone rather than acted on.
    pub(super) const fn should_try_unlock(&self) -> bool {
        matches!(self, Self::Locked | Self::ProbeFailed | Self::Unparseable)
    }
}

/// Asks `bw status` what state the vault is in.
///
/// The command is assembled the way [`BitwardenBackend::build_command`] assembles
/// it, and the part that was missing is `BW_SESSION`. RustConn keeps the session
/// key in process memory rather than in its own environment, and `bw status` reads
/// the session from the environment of the child — so a probe that did not pass it
/// reported `locked` for a vault RustConn had unlocked seconds earlier. That
/// verdict is what [`backend_readiness`] turns into
/// `BackendReadiness::NeedsAction`, which is what put "cannot store passwords yet:
/// Locked" on the startup banner of a working vault
/// (issue [#312](https://github.com/totoshko88/RustConn/issues/312)).
///
/// The other two additions are not new bugs but the same omissions: the extended
/// `PATH` is how a sandboxed `bw` finds the tools it shells out to, and
/// `--nointeraction` stops it prompting or reaching the network on a call that has
/// five seconds to answer.
///
/// The key travels as an environment variable, not as `--session`, for the reason
/// the backend does the same: argv is world-readable through `/proc/PID/cmdline`.
///
/// [`BitwardenBackend::build_command`]: rustconn_core::secret::BitwardenBackend
pub(super) fn check_bitwarden_status_sync(bw_cmd: &str) -> BwVaultStatus {
    use secrecy::ExposeSecret;

    let mut cmd = std::process::Command::new(bw_cmd);
    cmd.env("PATH", rustconn_core::cli_download::get_extended_path());
    cmd.args(["--nointeraction", "status"]);
    if let Some(session) = rustconn_core::secret::get_session_key() {
        cmd.env("BW_SESSION", session.expose_secret());
    }

    let Some(output) = probe(&mut cmd, "bw status") else {
        return BwVaultStatus::ProbeFailed;
    };
    if !output.status.success() {
        return BwVaultStatus::ProbeFailed;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&stdout) else {
        return BwVaultStatus::Unparseable;
    };
    match parsed.get("status").and_then(|v| v.as_str()) {
        Some("unlocked") => BwVaultStatus::Unlocked,
        Some("locked") => BwVaultStatus::Locked,
        Some("unauthenticated") => BwVaultStatus::Unauthenticated,
        Some(other) => BwVaultStatus::Other(other.to_string()),
        None => BwVaultStatus::Unparseable,
    }
}

/// Checks 1Password account status synchronously
fn check_onepassword_status_sync(op_cmd: &str) -> (String, &'static str) {
    let output = probe(
        std::process::Command::new(op_cmd).args(["whoami", "--format", "json"]),
        "op whoami",
    );

    match output {
        Some(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            if let Ok(whoami) = serde_json::from_str::<serde_json::Value>(&stdout)
                && let Some(email) = whoami.get("email").and_then(|v| v.as_str())
            {
                return (i18n_f("Signed in: {}", &[email]), "success");
            }
            (i18n("Signed in"), "success")
        }
        Some(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            if stderr.contains("not signed in") || stderr.contains("sign in") {
                (i18n("Not signed in"), "error")
            } else if stderr.contains("session expired") {
                (i18n("Session expired"), "warning")
            } else {
                (i18n("Not signed in"), "error")
            }
        }
        None => (i18n("Error checking status"), "error"),
    }
}

/// Checks Passbolt CLI configuration status synchronously
fn check_passbolt_status_sync() -> (String, &'static str) {
    let output = probe(
        std::process::Command::new("passbolt").args(["list", "user", "--json"]),
        "passbolt list user",
    );

    match output {
        Some(o) if o.status.success() => (i18n("Configured"), "success"),
        Some(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            if stderr.contains("no configuration") {
                (i18n("Not configured"), "error")
            } else if stderr.contains("authentication") || stderr.contains("passphrase") {
                (i18n("Authentication failed"), "warning")
            } else {
                (i18n("Not configured"), "error")
            }
        }
        None => (i18n("Error checking status"), "error"),
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

// `extract_session_key` used to live here, a second copy of the parser in
// `rustconn_core::secret::bitwarden`. Both existed because this page built its own
// `bw unlock` commands; now that all four of those call sites go through
// `unlock_vault_blocking`, core parses its own output and the GUI never sees a
// session key as text.

#[cfg(test)]
mod readiness_tests {
    use rustconn_core::config::SecretBackendType;

    use super::{BackendReadiness, LocalBackendState, backend_needs_probe, backend_readiness};

    /// Written out rather than derived, because `SecretBackendType` has no
    /// iterator and the point of these two tests is to notice a variant that was
    /// added to the enum and to only one of the two functions below.
    const ALL_BACKENDS: [SecretBackendType; 10] = [
        SecretBackendType::KeePassXc,
        SecretBackendType::KdbxFile,
        SecretBackendType::LibSecret,
        SecretBackendType::Bitwarden,
        SecretBackendType::OnePassword,
        SecretBackendType::Passbolt,
        SecretBackendType::Pass,
        SecretBackendType::MacOsKeychain,
        SecretBackendType::EncryptedFile,
        SecretBackendType::PortableEncryptedFile,
    ];

    fn nothing_configured() -> LocalBackendState {
        LocalBackendState {
            kdbx_enabled: false,
            kdbx_path: None,
            portable_passphrase_entered: false,
        }
    }

    /// The contract `backend_needs_probe` promises: a backend it exempts must
    /// produce a real verdict from `None` detection, or the startup check would
    /// skip the probe and then show "Checking..." with nothing coming to replace
    /// it.
    #[test]
    fn a_backend_that_needs_no_probe_answers_without_one() {
        let local = nothing_configured();
        for backend in ALL_BACKENDS {
            if backend_needs_probe(backend) {
                continue;
            }
            assert_ne!(
                backend_readiness(None, backend, &local),
                BackendReadiness::Unknown,
                "{backend:?} is exempt from the probe but cannot answer without one"
            );
        }
    }

    /// The other half: a backend that does read the detection must not be exempt,
    /// or the startup banner would render a verdict built from nothing.
    #[test]
    fn a_backend_that_needs_a_probe_is_not_exempt() {
        let local = nothing_configured();
        for backend in ALL_BACKENDS {
            if backend_readiness(None, backend, &local) == BackendReadiness::Unknown {
                assert!(
                    backend_needs_probe(backend),
                    "{backend:?} answers Unknown without a probe but is exempt from running one"
                );
            }
        }
    }
}

#[cfg(test)]
mod bw_vault_status_tests {
    use super::BwVaultStatus;

    /// Written out rather than derived, for the same reason `ALL_BACKENDS` above
    /// is: the enum has no iterator, and the point is to notice a variant that
    /// was added without a decision being made about it.
    fn all_variants() -> Vec<BwVaultStatus> {
        vec![
            BwVaultStatus::Unlocked,
            BwVaultStatus::Locked,
            BwVaultStatus::Unauthenticated,
            BwVaultStatus::Other("pendingApproval".to_string()),
            BwVaultStatus::Unparseable,
            BwVaultStatus::ProbeFailed,
        ]
    }

    /// The regression test for issue #312. The bug was that the unlock decision
    /// was made by comparing a *translated* display string against the literal
    /// `"Locked"`, so it could only ever be true in English. Pinning the decision
    /// to the variant is the fix, and this asserts the decision directly —
    /// nothing here goes near a rendered label.
    #[test]
    fn a_locked_vault_is_the_case_an_unlock_exists_for() {
        assert!(BwVaultStatus::Locked.should_try_unlock());
    }

    #[test]
    fn an_unlocked_vault_and_a_missing_login_are_both_left_alone() {
        // Nothing to do.
        assert!(!BwVaultStatus::Unlocked.should_try_unlock());
        // `bw unlock` cannot help a CLI with no account; it needs `bw login`.
        assert!(!BwVaultStatus::Unauthenticated.should_try_unlock());
        // A state this build does not recognise is not acted on.
        assert!(!BwVaultStatus::Other("pendingApproval".to_string()).should_try_unlock());
    }

    /// An inconclusive probe must not be read as "the vault is fine". `bw status`
    /// gets five seconds and was measured at 2.5 s on the reporter's machine, so a
    /// slower link turns a locked vault into `ProbeFailed` — and skipping the
    /// unlock there is the same silent no-op the issue was about, reached through a
    /// different door.
    #[test]
    fn an_inconclusive_probe_still_attempts_the_unlock() {
        assert!(BwVaultStatus::ProbeFailed.should_try_unlock());
        assert!(BwVaultStatus::Unparseable.should_try_unlock());
    }

    /// The separation the fix introduced: deciding and rendering are different
    /// jobs. Every variant must produce a non-empty label and a css class, and
    /// none of that may feed back into the decision.
    #[test]
    fn every_variant_renders_a_label_without_affecting_the_decision() {
        for status in all_variants() {
            let (text, css) = status.to_status_pair();
            assert!(!text.is_empty(), "{status:?} rendered an empty label");
            assert!(!css.is_empty(), "{status:?} rendered no css class");
            // Rendering is pure: asking twice gives the same answer, and asking at
            // all does not change what the decision would be.
            let (text_again, css_again) = status.to_status_pair();
            assert_eq!(text, text_again);
            assert_eq!(css, css_again);
        }
    }

    /// `Other` exists to show what `bw` actually said, so the raw value has to
    /// reach the label rather than being flattened to "Unknown".
    #[test]
    fn an_unrecognised_state_shows_what_bw_reported() {
        let (text, _) = BwVaultStatus::Other("pendingApproval".to_string()).to_status_pair();
        assert!(
            text.contains("pendingApproval"),
            "raw state was dropped from the label: {text}"
        );
    }

    /// The shape of the original bug, pinned so it cannot come back: a decision
    /// taken from the rendered text is wrong even in English, because the text is
    /// what a translator is free to change.
    #[test]
    fn the_decision_does_not_depend_on_the_rendered_text() {
        for status in all_variants() {
            let (text, _) = status.to_status_pair();
            let by_text = text == "Locked";
            if matches!(
                status,
                BwVaultStatus::ProbeFailed | BwVaultStatus::Unparseable
            ) {
                // These two are exactly where the two answers legitimately differ:
                // an inconclusive probe never renders as "Locked" but is still
                // worth an attempt.
                assert!(status.should_try_unlock() && !by_text);
            } else {
                assert_eq!(
                    status.should_try_unlock(),
                    by_text,
                    "{status:?} would have been decided differently by its label"
                );
            }
        }
    }
}
