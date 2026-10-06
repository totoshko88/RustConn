//! FreeRDP detection utilities.
//!
//! Every choice of FreeRDP client below goes through
//! [`rustconn_core::protocol::select_freerdp_client`], with the cached version
//! probe in this module as its oracle. Debian and Ubuntu install FreeRDP 2 under
//! the names FreeRDP 3 uses elsewhere, and FreeRDP 2 cannot run the
//! `/args-from:` command line RustConn hands it, so a name alone is not enough
//! to launch a client (issue #351).

use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use rustconn_core::protocol::{
    FreeRdpProbe, FreeRdpSelection, FreeRdpVersion, parse_freerdp_version, select_freerdp_client,
};

/// Maximum time allowed for a FreeRDP `--version` process.
///
/// Raised from 2s: under load, or when the probe is relayed out of a Flatpak
/// sandbox through `flatpak-spawn --host`, `--version` can take several seconds
/// to print its banner. A probe that times out returns no version, and a
/// client whose version is unknown is then trusted or refused by name alone —
/// the gap that launched a FreeRDP 2 with a command line it rejects (exit 255).
/// 5s keeps the probe bounded while giving a slow but healthy client room.
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Maximum time allowed for one `which` process during binary detection.
const BINARY_DETECTION_TIMEOUT: Duration = Duration::from_millis(500);
/// Maximum time allowed to reap a probe after sending it a kill request.
const PROBE_REAP_TIMEOUT: Duration = Duration::from_millis(500);
/// Poll interval avoids busy-waiting while keeping probes and cancellation responsive.
const PROBE_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// The FreeRDP clients the embedded mode can launch, preferred first.
///
/// Only a Wayland-native client embeds as a subsurface; the SDL3 client that
/// external launches prefer (issue #340) cannot. `wlfreerdp` qualifies only
/// when its probe reports FreeRDP 3 — on Debian and Ubuntu it is FreeRDP 2.
const EMBEDDED_FREERDP_CLIENTS: &[&str] = &["wlfreerdp3", "wlfreerdp"];

/// The FreeRDP clients that can host a RemoteApp (RAIL) session, preferred first.
const REMOTEAPP_FREERDP_CLIENTS: &[&str] = &["xfreerdp3", "xfreerdp"];

/// Only successfully-read versions are cached; a failed probe is left uncached
/// so a later attempt can re-probe. Exact keys distinguish host and sandbox targets.
static VERSION_CACHE: OnceLock<Mutex<HashMap<String, Option<FreeRdpVersion>>>> = OnceLock::new();

fn is_cancelled(cancellation: Option<&AtomicBool>) -> bool {
    cancellation.is_some_and(|flag| flag.load(Ordering::Acquire))
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

const fn args_from_form_for_version(version: Option<FreeRdpVersion>) -> ArgsFromForm {
    match version {
        Some(FreeRdpVersion { major, minor, .. })
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
    // Cache a version that was actually READ, never a `None`. A `None` means the
    // probe timed out, could not start, or printed no parseable banner — all
    // transient or load-sensitive conditions. Caching it would poison every
    // later probe of this binary for the whole process lifetime, so a client
    // whose first probe was starved stays "version unknown" forever and is then
    // trusted/refused purely by name. Leaving `None` uncached lets the next
    // attempt re-probe, which is cheap relative to launching the wrong FreeRDP
    // (issue: external FreeRDP exit 255 after an IronRDP hand-off).
    if let Some(read) = version {
        cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(binary.to_string(), Some(read));
    }
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

/// Whether `binary` is installed, and if so which FreeRDP it is.
///
/// A `host:` candidate is looked up on the Flatpak host. Only an installed
/// client is version-probed, and the probe is cached, so asking again costs a
/// lookup rather than a spawn.
fn probe_freerdp_candidate(binary: &str, cancellation: Option<&AtomicBool>) -> FreeRdpProbe {
    if is_cancelled(cancellation) {
        return FreeRdpProbe::Missing;
    }
    let installed = match binary.strip_prefix("host:") {
        Some(host_binary) => host_binary_exists(host_binary, cancellation),
        None => binary_exists(binary),
    };
    if installed {
        FreeRdpProbe::Installed(freerdp_version_with_cancel(binary, cancellation))
    } else {
        FreeRdpProbe::Missing
    }
}

/// The clients tried for an external launch, in this session's order.
///
/// The order lives in rustconn-core, shared with the client detection that
/// reports which FreeRDP is installed (issue #340). macOS: a FreeRDP shipped as
/// an `.app` bundle is not on `PATH`, so its in-bundle executable comes last;
/// the external launcher runs that path directly.
fn best_freerdp_candidates() -> Vec<String> {
    const MACOS_BUNDLES: &[(&str, &str)] = &[
        ("FreeRDP.app", "freerdp"),
        ("SDL-freerdp.app", "sdl-freerdp"),
        ("wlfreerdp.app", "wlfreerdp"),
    ];
    let mut candidates: Vec<String> = rustconn_core::protocol::freerdp_launch_order()
        .iter()
        .map(|candidate| (*candidate).to_string())
        .collect();
    if let Some(path) = rustconn_core::which::find_macos_app(MACOS_BUNDLES)
        .and_then(|path| path.into_os_string().into_string().ok())
    {
        candidates.push(path);
    }
    candidates
}

/// The clients tried for a RemoteApp launch: the X11 clients in the sandbox
/// first, then — under Flatpak, whose bundled FreeRDP has no X11 client — the
/// same clients on the host.
fn remoteapp_freerdp_candidates(flatpak: bool) -> Vec<String> {
    let mut candidates: Vec<String> = REMOTEAPP_FREERDP_CLIENTS
        .iter()
        .map(|candidate| (*candidate).to_string())
        .collect();
    if flatpak {
        candidates.extend(
            REMOTEAPP_FREERDP_CLIENTS
                .iter()
                .map(|candidate| format!("host:{candidate}")),
        );
    }
    candidates
}

/// Detects the best available FreeRDP binary — a FreeRDP 3 client.
#[must_use]
pub fn detect_best_freerdp() -> Option<String> {
    let selection = select_freerdp_client(best_freerdp_candidates(), |binary| {
        probe_freerdp_candidate(binary, None)
    });
    if selection == FreeRdpSelection::NotInstalled {
        tracing::warn!(
            protocol = "rdp",
            wayland = rustconn_core::protocol::is_wayland_session(),
            "No FreeRDP client found on PATH"
        );
    }
    selection.into_supported()
}

/// Chooses the client the embedded mode launches, asking `probe` about each.
fn select_embedded_wlfreerdp_with(probe: impl FnMut(&str) -> FreeRdpProbe) -> FreeRdpSelection {
    select_freerdp_client(EMBEDDED_FREERDP_CLIENTS, probe)
}

/// Chooses the Wayland client the embedded mode launches: `wlfreerdp3`, or a
/// `wlfreerdp` that reports FreeRDP 3 (issue #351).
///
/// [`detect_wlfreerdp`] and the embedded launch both ask this, so the mode is
/// only attempted when the launch will find a client it can run. Only the
/// sandbox's own `PATH` counts: the embedded client is never spawned on a
/// Flatpak host.
#[must_use]
pub fn select_embedded_wlfreerdp() -> FreeRdpSelection {
    select_embedded_wlfreerdp_with(|binary| probe_freerdp_candidate(binary, None))
}

/// Detects if a Wayland-native FreeRDP 3 client is available for embedded mode.
#[must_use]
pub fn detect_wlfreerdp() -> bool {
    rustconn_core::protocol::is_wayland_session()
        && matches!(select_embedded_wlfreerdp(), FreeRdpSelection::Supported(_))
}

/// Detects the best FreeRDP binary for RemoteApp sessions.
#[must_use]
pub fn detect_best_freerdp_for_remoteapp() -> Option<String> {
    let candidates = remoteapp_freerdp_candidates(rustconn_core::flatpak::is_flatpak());
    select_freerdp_client(candidates, |binary| probe_freerdp_candidate(binary, None))
        .into_supported()
}

/// Every FreeRDP client binary RustConn knows how to launch, newest-first.
///
/// The superset of the session-ordered launch lists that
/// `rustconn_core::protocol::freerdp_launch_order` returns, used to populate
/// the connection editor's "FreeRDP client" dropdown. `wlfreerdp`/`wlfreerdp3`
/// are deprecated upstream (issue #340) but still offered, since some setups
/// only ship them.
pub const KNOWN_FREERDP_CLIENTS: &[&str] = &[
    "sdl-freerdp3",
    "sdl-freerdp",
    "wlfreerdp3",
    "wlfreerdp",
    "xfreerdp3",
    "xfreerdp",
    "freerdp",
];

/// Returns the known FreeRDP clients RustConn can launch, for the UI.
///
/// Probes the local `PATH` and, under Flatpak, the host. Order follows
/// [`KNOWN_FREERDP_CLIENTS`] (newest-first). The result seeds the connection
/// editor's client dropdown, so neither an unavailable client nor a FreeRDP 2
/// one the launcher would refuse is offered (issue #351).
#[must_use]
pub fn available_freerdp_clients() -> Vec<String> {
    let flatpak = rustconn_core::flatpak::is_flatpak();
    rustconn_core::protocol::launchable_freerdp_clients(KNOWN_FREERDP_CLIENTS, |name| {
        if binary_exists(name) {
            FreeRdpProbe::Installed(freerdp_version(name))
        } else if flatpak && host_binary_exists(name, None) {
            FreeRdpProbe::Installed(freerdp_version(&format!("host:{name}")))
        } else {
            FreeRdpProbe::Missing
        }
    })
}

/// Resolves which FreeRDP binary to launch, honouring an explicit override.
///
/// When `override_name` is set and the named client is available (on the local
/// `PATH`, or on the Flatpak host, returned then as a `host:` form) and is
/// FreeRDP 3, it wins. An override that is not installed, that names a
/// `wl*`/`sdl*` client for a RemoteApp (RAIL) session, which those clients
/// cannot host (issue #340), or that turns out to be FreeRDP 2 (issue #351) is
/// dropped with a warning and the usual auto-detection takes over.
///
/// [`FreeRdpSelection::Unsupported`] means FreeRDP is installed but no
/// candidate is FreeRDP 3; it carries the version the caller tells the user
/// about.
#[must_use]
pub fn resolve_freerdp_binary(
    override_name: Option<&str>,
    is_remote_app: bool,
    cancellation: Option<&AtomicBool>,
) -> FreeRdpSelection {
    let pinned = override_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .and_then(|name| pinned_freerdp_candidate(name, is_remote_app, cancellation));
    let automatic = if is_remote_app {
        remoteapp_freerdp_candidates(rustconn_core::flatpak::is_flatpak())
    } else {
        best_freerdp_candidates()
    };

    // The pin is simply the most preferred candidate: a FreeRDP 2 one is
    // skipped like any other, and auto-detection carries on behind it.
    let candidates = pinned.iter().cloned().chain(automatic);
    let selection = select_freerdp_client(candidates, |binary| {
        if pinned.as_deref() == Some(binary) {
            // Found when the pin was resolved; only its version is left to learn.
            FreeRdpProbe::Installed(freerdp_version_with_cancel(binary, cancellation))
        } else {
            probe_freerdp_candidate(binary, cancellation)
        }
    });

    if let Some(ref pinned) = pinned
        && !is_cancelled(cancellation)
        && !matches!(&selection, FreeRdpSelection::Supported(chosen) if chosen == pinned)
    {
        tracing::warn!(
            protocol = "rdp",
            client = %pinned,
            "Configured FreeRDP client is not FreeRDP 3, which RustConn needs — falling back to auto-detection"
        );
    }
    selection
}

/// The form a configured client is launched in, or `None` when it cannot be.
///
/// Local first, then — under Flatpak — on the host, returned as `host:<name>`.
/// Only a plain program name is looked up on the host, because the name comes
/// from the connection's configuration and the host lookup runs a shell.
fn pinned_freerdp_candidate(
    name: &str,
    is_remote_app: bool,
    cancellation: Option<&AtomicBool>,
) -> Option<String> {
    if is_remote_app && !is_remoteapp_capable_client(name) {
        tracing::warn!(
            protocol = "rdp",
            client = %name,
            "Ignoring FreeRDP client override for a RemoteApp session — wl/sdl clients cannot host RAIL; auto-detecting"
        );
        return None;
    }
    if binary_exists(name) {
        return Some(name.to_string());
    }
    if rustconn_core::flatpak::is_flatpak()
        && is_plain_binary_name(name)
        && host_binary_exists(name, cancellation)
    {
        return Some(format!("host:{name}"));
    }
    tracing::warn!(
        protocol = "rdp",
        client = %name,
        "Configured FreeRDP client is not available — falling back to auto-detection"
    );
    None
}

/// Whether `name` is a bare program name, safe to place on a shell command line.
fn is_plain_binary_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
}

/// Whether a FreeRDP client can host a RemoteApp (RAIL) session.
///
/// `wl*` and `sdl*` clients render a full desktop into one surface and cannot
/// create the individual application windows RAIL needs, so only the X11 and
/// generic clients qualify. Matches on the binary's file name so an absolute
/// path or a `host:`-free name both classify correctly.
fn is_remoteapp_capable_client(name: &str) -> bool {
    let stem = std::path::Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    !(stem.starts_with("wl") || stem.starts_with("sdl"))
}

/// Whether the *host* has `name`, asked from inside a Flatpak sandbox.
///
/// `sh -lc 'command -v …'` rather than `which`: a login shell honours the user's
/// own `PATH`, and `command -v` is a shell builtin, so the answer no longer
/// depends on the host having a `which` binary installed (#303). Deliberately not
/// delegated to `rustconn_core::which::find_on_host`, which is otherwise the same
/// probe — this one keeps the cancellation token that lets an abandoned session
/// stop the detection thread. `name` is one of the hardcoded FreeRDP candidates
/// or a configured client that passed [`is_plain_binary_name`], so nothing that
/// could change the command reaches the shell.
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

    const FREERDP_2: Option<FreeRdpVersion> = Some(FreeRdpVersion::new(2, 11, 5));
    const FREERDP_3: Option<FreeRdpVersion> = Some(FreeRdpVersion::new(3, 32, 1));

    /// A probe that knows only the listed clients, each with its version.
    fn installed<'a>(
        clients: &'a [(&'a str, Option<FreeRdpVersion>)],
    ) -> impl FnMut(&str) -> FreeRdpProbe + 'a {
        move |binary: &str| {
            clients
                .iter()
                .find(|client| client.0 == binary)
                .map_or(FreeRdpProbe::Missing, |client| {
                    FreeRdpProbe::Installed(client.1)
                })
        }
    }

    /// The embedded launch takes `wlfreerdp3`, then a `wlfreerdp` that is
    /// FreeRDP 3, and nothing else — so Ubuntu 24.04's FreeRDP 2 `wlfreerdp`
    /// is never handed an `/args-from:` command line it rejects (issue #351).
    #[test]
    fn embedded_mode_takes_only_a_freerdp_3_wayland_client() {
        let both = [("wlfreerdp3", None), ("wlfreerdp", FREERDP_2)];
        assert_eq!(
            select_embedded_wlfreerdp_with(installed(&both)),
            FreeRdpSelection::Supported("wlfreerdp3".to_string())
        );

        // The Flatpak's bundled client, and Arch's: FreeRDP 3 without the suffix.
        let unsuffixed_3 = [("wlfreerdp", FREERDP_3)];
        assert_eq!(
            select_embedded_wlfreerdp_with(installed(&unsuffixed_3)),
            FreeRdpSelection::Supported("wlfreerdp".to_string())
        );

        let unsuffixed_2 = [("wlfreerdp", FREERDP_2)];
        let selection = select_embedded_wlfreerdp_with(installed(&unsuffixed_2));
        assert_eq!(selection.unsupported_version(), FREERDP_2);
        assert_eq!(selection.into_supported(), None);

        // A `wlfreerdp` whose version could not be read is not trusted.
        let unknown = [("wlfreerdp", None)];
        assert!(matches!(
            select_embedded_wlfreerdp_with(installed(&unknown)),
            FreeRdpSelection::Unsupported(_)
        ));

        assert_eq!(
            select_embedded_wlfreerdp_with(installed(&[])),
            FreeRdpSelection::NotInstalled
        );
    }

    #[test]
    fn remoteapp_looks_in_the_sandbox_before_the_host() {
        assert_eq!(
            remoteapp_freerdp_candidates(false),
            ["xfreerdp3", "xfreerdp"]
        );
        assert_eq!(
            remoteapp_freerdp_candidates(true),
            ["xfreerdp3", "xfreerdp", "host:xfreerdp3", "host:xfreerdp"]
        );
    }

    /// A FreeRDP 2 X11 client in the sandbox does not stop a FreeRDP 3 one on
    /// the Flatpak host from hosting the RemoteApp session.
    #[test]
    fn remoteapp_skips_freerdp_2_for_a_freerdp_3_host_client() {
        let clients = [("xfreerdp", FREERDP_2), ("host:xfreerdp3", FREERDP_3)];
        assert_eq!(
            select_freerdp_client(remoteapp_freerdp_candidates(true), installed(&clients)),
            FreeRdpSelection::Supported("host:xfreerdp3".to_string())
        );
    }

    /// A configured client name reaches a host shell only when it is a bare
    /// program name.
    #[test]
    fn only_a_plain_binary_name_is_looked_up_on_the_host() {
        for plain in ["xfreerdp3", "sdl-freerdp", "wlfreerdp3", "freerdp_3.5+git"] {
            assert!(is_plain_binary_name(plain), "{plain}");
        }
        for unsafe_name in [
            "",
            "x; rm -rf ~",
            "$(id)",
            "a b",
            "`id`",
            "/usr/bin/xfreerdp",
        ] {
            assert!(!is_plain_binary_name(unsafe_name), "{unsafe_name:?}");
        }
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
        // the first call resolved, the second must equal it.
        let first = freerdp_version(&binary);
        let second = freerdp_version(&binary);
        assert_eq!(first, second, "the second lookup must reuse the cache");

        // The counter has to tolerate that same starvation, and for a while it
        // did not: it read the file with `.expect("read probe count")`, so a
        // probe killed before the shell reached its first `printf` left no file
        // at all and failed the test on a machine's load. Absent means "the
        // script never ran", which is not a cache defect.
        let probes = std::fs::read_to_string(&counter).unwrap_or_default();
        match first {
            // The banner parsed, so the script ran to completion and its mark is
            // on disk. Exactly one is the whole claim.
            Some(version) => {
                assert_eq!(version, FreeRdpVersion::new(3, 26, 1));
                assert_eq!(
                    probes, "x",
                    "the binary must be probed exactly once, then served from cache"
                );
            }
            // The probe lost its race, so the script may have run partly or not
            // at all. What survives of the claim is that it did not run twice.
            None => assert!(
                probes.len() <= 1,
                "a starved probe may leave no mark, but the cache must stop a second spawn; got {probes:?}"
            ),
        }
    }

    #[test]
    fn version_selects_compatible_args_from_form() {
        assert_eq!(
            args_from_form_for_version(Some(FreeRdpVersion::new(3, 25, 0))),
            ArgsFromForm::BarePath
        );
        assert_eq!(
            args_from_form_for_version(Some(FreeRdpVersion::new(3, 26, 0))),
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
