//! FreeRDP detection utilities.

use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const WAYLAND_FIRST_CANDIDATES: &[&str] = &[
    "sdl-freerdp3",
    "sdl-freerdp",
    "wlfreerdp3",
    "wlfreerdp",
    "xfreerdp3",
    "xfreerdp",
];
const X11_FIRST_CANDIDATES: &[&str] = &[
    "xfreerdp3",
    "xfreerdp",
    "sdl-freerdp3",
    "sdl-freerdp",
    "wlfreerdp3",
    "wlfreerdp",
];

/// Maximum time allowed for a FreeRDP `--version` process.
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// Maximum time allowed for one `which` process during binary detection.
const BINARY_DETECTION_TIMEOUT: Duration = Duration::from_millis(500);
/// Maximum time allowed to reap a probe after sending it a kill request.
const PROBE_REAP_TIMEOUT: Duration = Duration::from_millis(500);
/// Poll interval avoids busy-waiting while keeping probes and cancellation responsive.
const PROBE_POLL_INTERVAL: Duration = Duration::from_millis(10);

type FreeRdpVersion = (u32, u32, u32);

/// Includes failed probes. Exact keys distinguish host and sandbox targets.
static VERSION_CACHE: OnceLock<Mutex<HashMap<String, Option<FreeRdpVersion>>>> = OnceLock::new();

fn is_cancelled(cancellation: Option<&AtomicBool>) -> bool {
    cancellation.is_some_and(|flag| flag.load(Ordering::Acquire))
}

fn is_wayland_session() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|v| v == "wayland")
        .unwrap_or(false)
        || std::env::var("WAYLAND_DISPLAY").is_ok()
}

/// Syntax accepted by the installed FreeRDP for `/args-from:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgsFromForm {
    /// `/args-from:<path>` — accepted by every FreeRDP 3.x release.
    BarePath,
    /// `/args-from:file:<path>` — FreeRDP 3.26 and newer only.
    FilePrefix,
}

const ARGS_FROM_FILE_PREFIX_MIN_MINOR: u32 = 26;

fn parse_freerdp_version(output: &str) -> Option<FreeRdpVersion> {
    let lower = output.to_ascii_lowercase();
    let freerdp_offset = lower.find("freerdp")?;
    output[freerdp_offset..]
        .split_whitespace()
        .find_map(parse_version_token)
}

fn parse_version_token(token: &str) -> Option<FreeRdpVersion> {
    let start = token.find(|c: char| c.is_ascii_digit())?;
    let numeric: String = token[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let mut parts = numeric.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts
        .next()
        .filter(|part| !part.is_empty())
        .map_or(Some(0), |part| part.parse().ok())?;
    Some((major, minor, patch))
}

const fn args_from_form_for_version(version: Option<FreeRdpVersion>) -> ArgsFromForm {
    match version {
        Some((major, minor, _))
            if major > 3 || (major == 3 && minor >= ARGS_FROM_FILE_PREFIX_MIN_MINOR) =>
        {
            ArgsFromForm::FilePrefix
        }
        _ => ArgsFromForm::BarePath,
    }
}

fn version_probe_command(binary: &str) -> (String, Vec<String>) {
    if let Some(host_binary) = binary.strip_prefix("host:") {
        (
            "flatpak-spawn".to_string(),
            vec![
                "--host".to_string(),
                "--watch-bus".to_string(),
                host_binary.to_string(),
                "--version".to_string(),
            ],
        )
    } else {
        (binary.to_string(), vec!["--version".to_string()])
    }
}

fn terminate_and_reap_probe(mut child: std::process::Child, target: &str, operation: &str) {
    if let Err(error) = child.kill() {
        tracing::debug!(protocol = "rdp", target, operation, %error, "Probe exited before kill");
    }
    let deadline = Instant::now() + PROBE_REAP_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(PROBE_POLL_INTERVAL),
            Ok(None) => {
                tracing::warn!(
                    protocol = "rdp",
                    target,
                    operation,
                    "Probe did not exit promptly after kill; reaping in background"
                );
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return;
            }
            Err(error) => {
                tracing::warn!(protocol = "rdp", target, operation, %error, "Failed to poll probe");
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return;
            }
        }
    }
}

fn probe_freerdp_version_with_timeout(
    binary: &str,
    timeout: Duration,
    cancellation: Option<&AtomicBool>,
) -> Option<FreeRdpVersion> {
    if is_cancelled(cancellation) {
        return None;
    }
    let (program, args) = version_probe_command(binary);
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        if is_cancelled(cancellation) {
            terminate_and_reap_probe(child, binary, "version");
            return None;
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(PROBE_POLL_INTERVAL),
            Ok(None) => {
                tracing::warn!(
                    protocol = "rdp",
                    binary,
                    timeout_ms = timeout.as_millis(),
                    "FreeRDP version probe timed out"
                );
                terminate_and_reap_probe(child, binary, "version");
                return None;
            }
            Err(error) => {
                tracing::debug!(protocol = "rdp", binary, %error, "Version probe failed");
                terminate_and_reap_probe(child, binary, "version");
                return None;
            }
        }
    }

    let mut stdout = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_end(&mut stdout);
    }
    let mut stderr = Vec::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_end(&mut stderr);
    }
    if is_cancelled(cancellation) {
        return None;
    }
    parse_freerdp_version(&String::from_utf8_lossy(&stdout))
        .or_else(|| parse_freerdp_version(&String::from_utf8_lossy(&stderr)))
}

fn freerdp_version_with_cancel(
    binary: &str,
    cancellation: Option<&AtomicBool>,
) -> Option<FreeRdpVersion> {
    if is_cancelled(cancellation) {
        return None;
    }
    let cache = VERSION_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let cached = {
        let guard = cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.get(binary).copied()
    };
    if let Some(version) = cached {
        return version;
    }
    let version = probe_freerdp_version_with_timeout(binary, VERSION_PROBE_TIMEOUT, cancellation);
    if is_cancelled(cancellation) {
        return None;
    }
    cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(binary.to_string(), version);
    tracing::debug!(
        protocol = "rdp",
        binary,
        ?version,
        "Detected FreeRDP version"
    );
    version
}

/// Queries `binary --version` with a bounded probe and caches the result.
///
/// The original target, including a `host:` marker, is the cache key.
#[must_use]
pub fn freerdp_version(binary: &str) -> Option<FreeRdpVersion> {
    freerdp_version_with_cancel(binary, None)
}

/// Resolves the args-file syntax before a credential file is created.
#[must_use]
pub fn resolve_args_from_form(binary: &str) -> ArgsFromForm {
    resolve_args_from_form_with_cancel(binary, None)
}

pub(crate) fn resolve_args_from_form_with_cancel(
    binary: &str,
    cancellation: Option<&AtomicBool>,
) -> ArgsFromForm {
    args_from_form_for_version(freerdp_version_with_cancel(binary, cancellation))
}

/// Builds an args-file switch from a previously resolved form.
#[must_use]
pub fn args_from_argument_for_form(form: ArgsFromForm, path: &std::path::Path) -> String {
    match form {
        ArgsFromForm::FilePrefix => format!("/args-from:file:{}", path.display()),
        ArgsFromForm::BarePath => format!("/args-from:{}", path.display()),
    }
}

/// Builds an args-file switch for callers that did not pre-resolve the form.
#[must_use]
pub fn args_from_argument(binary: &str, path: &std::path::Path) -> String {
    args_from_argument_for_form(resolve_args_from_form(binary), path)
}

fn command_succeeds_with_timeout(
    program: &str,
    args: &[&str],
    target: &str,
    timeout: Duration,
    cancellation: Option<&AtomicBool>,
) -> bool {
    if is_cancelled(cancellation) {
        return false;
    }
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + timeout;
    loop {
        if is_cancelled(cancellation) {
            terminate_and_reap_probe(child, target, "binary detection");
            return false;
        }
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(PROBE_POLL_INTERVAL),
            Ok(None) => {
                tracing::debug!(
                    protocol = "rdp",
                    target,
                    timeout_ms = timeout.as_millis(),
                    "FreeRDP binary detection timed out"
                );
                terminate_and_reap_probe(child, target, "binary detection");
                return false;
            }
            Err(error) => {
                tracing::debug!(protocol = "rdp", target, %error, "Binary detection failed");
                terminate_and_reap_probe(child, target, "binary detection");
                return false;
            }
        }
    }
}

/// Whether a FreeRDP binary is installed in the sandbox or on `PATH`.
///
/// Resolved in process by the shared lookup, so there is no child to time out or
/// cancel — hence no `cancellation` parameter, unlike the host probe below, which
/// really does spawn `flatpak-spawn`. Spawning `which` here meant a missing
/// `which` reported every FreeRDP client as absent (#303).
fn binary_exists(name: &str) -> bool {
    rustconn_core::which::is_available(name)
}

pub(crate) fn detect_best_freerdp_with_cancel(cancellation: Option<&AtomicBool>) -> Option<String> {
    // macOS: a FreeRDP shipped as an `.app` bundle is not on PATH; the in-bundle
    // executable path is used as a fallback below.
    const MACOS_BUNDLES: &[(&str, &str)] = &[
        ("FreeRDP.app", "freerdp"),
        ("SDL-freerdp.app", "sdl-freerdp"),
        ("wlfreerdp.app", "wlfreerdp"),
    ];
    let wayland = is_wayland_session();
    let candidates = if wayland {
        WAYLAND_FIRST_CANDIDATES
    } else {
        X11_FIRST_CANDIDATES
    };
    for candidate in candidates {
        if is_cancelled(cancellation) {
            return None;
        }
        if binary_exists(candidate) {
            return Some((*candidate).to_string());
        }
    }
    if is_cancelled(cancellation) {
        return None;
    }
    // macOS: a FreeRDP shipped as an `.app` bundle is not on PATH. Return the
    // in-bundle executable path, which the external launcher runs directly.
    if let Some(path) = rustconn_core::which::find_macos_app(MACOS_BUNDLES)
        .and_then(|p| p.into_os_string().into_string().ok())
    {
        return Some(path);
    }
    tracing::warn!(protocol = "rdp", wayland, "No FreeRDP client found on PATH");
    None
}

/// Detects the best available FreeRDP binary.
#[must_use]
pub fn detect_best_freerdp() -> Option<String> {
    detect_best_freerdp_with_cancel(None)
}

/// Detects if a Wayland-native FreeRDP variant is available for embedded mode.
#[must_use]
pub fn detect_wlfreerdp() -> bool {
    is_wayland_session() && (binary_exists("wlfreerdp3") || binary_exists("wlfreerdp"))
}

pub(crate) fn detect_best_freerdp_for_remoteapp_with_cancel(
    cancellation: Option<&AtomicBool>,
) -> Option<String> {
    const REMOTEAPP_CANDIDATES: &[&str] = &["xfreerdp3", "xfreerdp"];
    for candidate in REMOTEAPP_CANDIDATES {
        if is_cancelled(cancellation) {
            return None;
        }
        if binary_exists(candidate) {
            return Some((*candidate).to_string());
        }
    }
    if rustconn_core::flatpak::is_flatpak() {
        for candidate in REMOTEAPP_CANDIDATES {
            if is_cancelled(cancellation) {
                return None;
            }
            if host_binary_exists(candidate, cancellation) {
                return Some(format!("host:{candidate}"));
            }
        }
    }
    None
}

/// Detects the best FreeRDP binary for RemoteApp sessions.
#[must_use]
pub fn detect_best_freerdp_for_remoteapp() -> Option<String> {
    detect_best_freerdp_for_remoteapp_with_cancel(None)
}

/// Whether the *host* has `name`, asked from inside a Flatpak sandbox.
///
/// `sh -lc 'command -v …'` rather than `which`: a login shell honours the user's
/// own `PATH`, and `command -v` is a shell builtin, so the answer no longer
/// depends on the host having a `which` binary installed (#303). Deliberately not
/// delegated to `rustconn_core::which::find_on_host`, which is otherwise the same
/// probe — this one keeps the cancellation token that lets an abandoned session
/// stop the detection thread. `name` is always one of the hardcoded FreeRDP
/// candidates, so nothing user-supplied reaches the shell.
fn host_binary_exists(name: &str, cancellation: Option<&AtomicBool>) -> bool {
    command_succeeds_with_timeout(
        "flatpak-spawn",
        &[
            "--host",
            "--watch-bus",
            "sh",
            "-lc",
            &format!("command -v {name}"),
        ],
        name,
        BINARY_DETECTION_TIMEOUT,
        cancellation,
    )
}

/// Detects any FreeRDP client available for external mode.
#[must_use]
pub fn detect_xfreerdp() -> Option<String> {
    detect_best_freerdp()
}

/// Checks if the native IronRDP client is compiled in.
#[must_use]
pub fn is_ironrdp_available() -> bool {
    rustconn_core::is_embedded_rdp_available()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;

    use super::*;

    fn executable_script(body: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().expect("create temporary directory");
        let path = dir.path().join("probe.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write probe script");
        let mut permissions = std::fs::metadata(&path)
            .expect("read probe script metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&path, permissions).expect("make probe script executable");
        (dir, path.to_string_lossy().into_owned())
    }

    #[test]
    fn candidates_have_expected_precedence() {
        let sdl = WAYLAND_FIRST_CANDIDATES
            .iter()
            .position(|c| *c == "sdl-freerdp3")
            .expect("SDL candidate must exist");
        let wl = WAYLAND_FIRST_CANDIDATES
            .iter()
            .position(|c| *c == "wlfreerdp3")
            .expect("Wayland candidate must exist");
        let x11 = WAYLAND_FIRST_CANDIDATES
            .iter()
            .position(|c| *c == "xfreerdp3")
            .expect("X11 candidate must exist");
        assert!(sdl < wl && wl < x11);
        assert_eq!(X11_FIRST_CANDIDATES.first(), Some(&"xfreerdp3"));
    }

    #[test]
    fn parses_robust_freerdp_version_banners() {
        for (banner, expected) in [
            ("This is FreeRDP version 3.24.2 (3.24.2)", (3, 24, 2)),
            ("THIS IS FREERDP VERSION v3.26.0", (3, 26, 0)),
            ("FreeRDP build: 3.27.0-rc1", (3, 27, 0)),
            ("FreeRDP version 4.0", (4, 0, 0)),
        ] {
            assert_eq!(parse_freerdp_version(banner), Some(expected));
        }
    }

    #[test]
    fn rejects_unparseable_version_output() {
        assert_eq!(parse_freerdp_version(""), None);
        assert_eq!(parse_freerdp_version("FreeRDP version unknown"), None);
        assert_eq!(parse_freerdp_version("version 3.26.0"), None);
    }

    #[test]
    fn host_probe_preserves_original_target_construction() {
        assert_eq!(
            version_probe_command("host:xfreerdp3"),
            (
                "flatpak-spawn".to_string(),
                vec![
                    "--host".into(),
                    "--watch-bus".into(),
                    "xfreerdp3".into(),
                    "--version".into()
                ]
            )
        );
    }

    #[test]
    fn version_probe_is_bounded_and_kills_hung_process() {
        let (_dir, binary) = executable_script("exec sleep 5");
        let started = Instant::now();
        assert_eq!(
            probe_freerdp_version_with_timeout(&binary, Duration::from_millis(50), None),
            None
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn version_probe_honors_cancellation() {
        let (_dir, binary) = executable_script("exec sleep 5");
        let cancellation = Arc::new(AtomicBool::new(false));
        let trigger = Arc::clone(&cancellation);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            trigger.store(true, Ordering::Release);
        });
        let started = Instant::now();
        assert_eq!(
            probe_freerdp_version_with_timeout(
                &binary,
                Duration::from_secs(5),
                Some(&cancellation)
            ),
            None
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn binary_detection_is_bounded() {
        let (_dir, binary) = executable_script("exec sleep 5");
        let started = Instant::now();
        assert!(!command_succeeds_with_timeout(
            &binary,
            &[],
            "test-probe",
            Duration::from_millis(50),
            None
        ));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn binary_detection_honors_cancellation() {
        let (_dir, binary) = executable_script("exec sleep 5");
        let cancellation = Arc::new(AtomicBool::new(false));
        let trigger = Arc::clone(&cancellation);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            trigger.store(true, Ordering::Release);
        });
        let started = Instant::now();
        assert!(!command_succeeds_with_timeout(
            &binary,
            &[],
            "test-probe",
            Duration::from_secs(5),
            Some(&cancellation)
        ));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn version_is_cached_per_original_target() {
        let counter_dir = tempfile::tempdir().expect("create counter directory");
        let counter = counter_dir.path().join("count");
        let body = format!(
            "printf x >> '{}'; printf 'This is FreeRDP version 3.26.1\\n'",
            counter.display()
        );
        let (_script_dir, binary) = executable_script(&body);

        // What this test proves is the *cache*, not the probe: the binary is
        // spawned once and the second lookup reuses the stored answer. It must
        // not assert the parsed version directly, because the probe races a
        // wall-clock timeout — under a saturated `cargo test --workspace` the
        // spawned shell can miss the deadline and cache `None`, which is a
        // property of the machine's load, not of the code under test. Whatever
        // the first call resolved, the second must equal it, and the script must
        // have run exactly once.
        let first = freerdp_version(&binary);
        let second = freerdp_version(&binary);
        assert_eq!(first, second, "the second lookup must reuse the cache");
        assert_eq!(
            std::fs::read_to_string(&counter).expect("read probe count"),
            "x",
            "the binary must be probed exactly once, then served from cache"
        );

        // When the probe did win its race, confirm the banner parsed correctly —
        // this keeps the version-parsing assertion whenever the machine allowed
        // it, without turning a starved subprocess into a test failure.
        if let Some(version) = first {
            assert_eq!(version, (3, 26, 1));
        }
    }

    #[test]
    fn version_selects_compatible_args_from_form() {
        assert_eq!(
            args_from_form_for_version(Some((3, 25, 0))),
            ArgsFromForm::BarePath
        );
        assert_eq!(
            args_from_form_for_version(Some((3, 26, 0))),
            ArgsFromForm::FilePrefix
        );
        assert_eq!(args_from_form_for_version(None), ArgsFromForm::BarePath);
    }

    #[test]
    fn formats_pre_resolved_args_from_form() {
        let path = std::path::Path::new("/run/user/1000/rustconn-rdp.args");
        assert_eq!(
            args_from_argument_for_form(ArgsFromForm::BarePath, path),
            "/args-from:/run/user/1000/rustconn-rdp.args"
        );
        assert_eq!(
            args_from_argument_for_form(ArgsFromForm::FilePrefix, path),
            "/args-from:file:/run/user/1000/rustconn-rdp.args"
        );
    }
}
