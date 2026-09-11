//! SPICE external-viewer integration boundary.
//!
//! SPICE sessions are handled by launching an external viewer
//! (`remote-viewer`, `virt-viewer`, or `spicy`). This module provides the
//! connection configuration ([`SpiceClientConfig`]), error type
//! ([`SpiceClientError`]), and the helpers that detect a viewer and build its
//! command line ([`detect_spice_viewer`], [`build_spice_viewer_args`]).
//! Callers that need a strict domain-only build should avoid invoking the
//! detection/launch helpers and treat [`SpiceClientConfig`] as data.
//!
//! # History
//!
//! A native embedded SPICE client (behind a `spice-embedded` feature) was
//! removed in 0.18.0: the bundled `spice-client` 0.2 exposes neither an inputs
//! channel nor raw display frames through its public API, so embedded rendering
//! and input forwarding were impossible without forking the crate. The external
//! viewer is the supported path.

mod config;
mod error;

use std::path::Path;

pub use config::{
    SpiceClientConfig, SpiceImageCompression as SpiceCompression, SpiceSecurityProtocol,
    SpiceSharedFolder,
};
pub use error::SpiceClientError;

/// USB auto-redirect filter for `remote-viewer`: auto-redirect HID-class
/// (`0x03`) devices on connect. The value is a `|`-separated list of
/// `class,vendor,product,version,allow` rules.
pub const SPICE_USB_AUTO_REDIRECT_FILTER: &str = "0x03,-1,-1,-1,0|-1,-1,-1,-1,1";

/// Builds the SPICE connection URI for an external viewer.
///
/// Returns `spice+unix://<path>` when `unix_socket_path` is set (host/port are
/// ignored), `spice+tls://host:port` when TLS is enabled, otherwise
/// `spice://host:port`. Shared by [`build_spice_viewer_args`] and the CLI's
/// `SpiceProtocol::build_command` so both paths stay in sync.
#[must_use]
pub fn build_spice_uri(
    unix_socket_path: Option<&Path>,
    tls_enabled: bool,
    host: &str,
    port: u16,
) -> String {
    match unix_socket_path {
        Some(path) => format!("spice+unix://{}", path.display()),
        None if tls_enabled => format!("spice+tls://{host}:{port}"),
        None => format!("spice://{host}:{port}"),
    }
}

/// SPICE viewer binaries, in order of preference.
///
/// `remote-viewer` and `virt-viewer` both ship in the `virt-viewer` package;
/// `remote-viewer` is the one that takes a connection URI, which is why it comes
/// first. `spicy` is spice-gtk's own test client and a last resort.
pub const SPICE_VIEWERS: &[&str] = &["remote-viewer", "virt-viewer", "spicy"];

/// Marker prefix for a viewer that only exists outside the Flatpak sandbox.
///
/// The launcher decodes it into `flatpak-spawn --host`, mirroring the convention
/// the RDP client detection already uses.
pub const HOST_VIEWER_PREFIX: &str = "host:";

/// SPICE viewers installed as macOS `.app` bundles.
///
/// virt-viewer's official macOS build ships `RemoteViewer.app`, whose
/// executable is `RemoteViewer`; some builds use a `virt-viewer.app` wrapper.
/// A bare-name PATH lookup never finds these (see [`crate::which::find_macos_app`]).
#[cfg(target_os = "macos")]
const SPICE_VIEWER_BUNDLES: &[(&str, &str)] = &[
    ("RemoteViewer.app", "RemoteViewer"),
    ("virt-viewer.app", "remote-viewer"),
];

/// Finds an installed SPICE viewer, or `None` when the user has none.
///
/// Returns the binary name for a viewer RustConn can run directly, or
/// `host:<name>` for one found on the host from inside a Flatpak sandbox — see
/// [`HOST_VIEWER_PREFIX`]. Inside Flatpak the host fallback is the case that
/// matters: virt-viewer is a desktop application in its own right and is not
/// bundled in the manifest, so the in-sandbox lookup can only ever fail.
///
/// Until 0.20.11 this spawned `which` and reported "not installed" whenever that
/// binary was missing, which is what issue
/// [#303](https://github.com/totoshko88/RustConn/issues/303) was: *"Install
/// virt-viewer"* on a machine that had it.
#[must_use]
pub fn detect_spice_viewer() -> Option<String> {
    if let Some(viewer) = SPICE_VIEWERS
        .iter()
        .find(|candidate| crate::which::is_available(candidate))
    {
        return Some((*viewer).to_string());
    }

    // macOS: virt-viewer is a GUI `.app`, not a PATH binary. Return the full
    // executable path inside the bundle, which the launcher runs directly.
    #[cfg(target_os = "macos")]
    if let Some(path) = crate::which::find_macos_app(SPICE_VIEWER_BUNDLES) {
        tracing::info!(viewer = %path.display(), "using the macOS SPICE viewer bundle");
        return path.into_os_string().into_string().ok();
    }

    for candidate in SPICE_VIEWERS {
        if crate::which::find_on_host(candidate).is_some() {
            tracing::info!(viewer = candidate, "using the host SPICE viewer");
            return Some(format!("{HOST_VIEWER_PREFIX}{candidate}"));
        }
    }

    tracing::warn!(
        candidates = ?SPICE_VIEWERS,
        "no SPICE viewer found in PATH, in the sandbox, or on the host"
    );
    None
}

/// Builds command-line arguments for virt-viewer/remote-viewer fallback
///
/// This function generates the appropriate command-line arguments for
/// launching an external SPICE viewer when native embedding is not available.
///
/// # Arguments
///
/// * `config` - The SPICE client configuration
///
/// # Returns
///
/// A vector of command-line arguments for the SPICE viewer
#[must_use]
pub fn build_spice_viewer_args(config: &SpiceClientConfig) -> Vec<String> {
    let mut args = Vec::new();

    // Connection URI: spice+unix:///path or spice://host:port
    args.push(build_spice_uri(
        config.unix_socket_path.as_deref(),
        config.tls_enabled,
        &config.host,
        config.port,
    ));

    // Full screen option (not enabled by default for embedded-like behavior)

    // Title
    args.push("--title".to_string());
    if config.unix_socket_path.is_some() {
        args.push(format!(
            "SPICE: {}",
            config
                .unix_socket_path
                .as_ref()
                .map_or("socket", |p| p.to_str().unwrap_or("socket"))
        ));
    } else {
        args.push(format!("SPICE: {}", config.host));
    }

    // USB redirection
    if config.usb_redirection {
        args.push("--spice-usbredir-auto-redirect-filter".to_string());
        args.push(SPICE_USB_AUTO_REDIRECT_FILTER.to_string());
    }

    // Shared folders (webdav)
    for folder in &config.shared_folders {
        args.push("--spice-shared-dir".to_string());
        args.push(folder.local_path.to_string_lossy().to_string());
    }

    // TLS options
    if config.tls_enabled {
        if let Some(ref ca_path) = config.ca_cert_path {
            args.push("--spice-ca-file".to_string());
            args.push(ca_path.to_string_lossy().to_string());
        }

        if config.skip_cert_verify {
            // Note: remote-viewer doesn't have a direct skip-verify flag
            // but we can set host-subject to empty to be more permissive
            args.push("--spice-host-subject".to_string());
            args.push(String::new());
        }
    }

    // Disable audio if not wanted
    if !config.audio_playback {
        args.push("--spice-disable-audio".to_string());
    }

    // ponytail: `image_compression` and `clipboard_enabled` are intentionally
    // not translated to arguments. Spice-GTK's CLI (spice-client(1)) exposes no
    // option to pick an image compressor — it is negotiated with the server —
    // and no option to disable clipboard sharing; copy-paste is governed
    // host-side (libvirt `<clipboard copypaste='no'/>`) or by the guest agent.
    // The fields are kept because they are meaningful for the embedded client
    // and for `.vv`-less future viewers, but remote-viewer cannot honour them.
    // If a viewer ever grows the flags, emit them here.

    // SPICE proxy for tunnelled connections (e.g. Proxmox VE)
    if let Some(ref proxy) = config.proxy {
        args.push("--spice-proxy".to_string());
        args.push(proxy.clone());
    }

    args
}

/// Builds the viewer flags that work alongside a `.vv` connection file.
///
/// A `.vv` file (see [`build_vv_connection_file`]) fully describes the *target*
/// — host, port, TLS, password — but the connection-independent options are
/// still passed as flags. `remote-viewer` accepts these next to a connection
/// file, so the password path can keep USB auto-redirect, WebDAV shared folders
/// and the window title without putting any of them in the file.
///
/// Deliberately omits everything the `.vv` file already carries (`--spice-proxy`,
/// `--spice-ca-file`, `--spice-host-subject`) so the two never state the same
/// thing twice. Audio is *not* one of those: the `.vv` format has no audio field,
/// so `--spice-disable-audio` has to stay on argv or a connection with audio
/// playback switched off would get audio back the moment a password is delivered.
#[must_use]
pub fn build_spice_extra_flags(config: &SpiceClientConfig) -> Vec<String> {
    let mut args = Vec::new();

    args.push("--title".to_string());
    args.push(format!("SPICE: {}", config.host));

    if config.usb_redirection {
        args.push("--spice-usbredir-auto-redirect-filter".to_string());
        args.push(SPICE_USB_AUTO_REDIRECT_FILTER.to_string());
    }
    for folder in &config.shared_folders {
        args.push("--spice-shared-dir".to_string());
        args.push(folder.local_path.to_string_lossy().to_string());
    }
    if !config.audio_playback {
        args.push("--spice-disable-audio".to_string());
    }

    // ponytail: `image_compression` and `clipboard_enabled` are not emitted —
    // spice-client(1) has no flag for either (compression is server-negotiated,
    // clipboard is a host/guest-agent setting). Same rationale as in
    // `build_spice_viewer_args`; the `.vv` format carries neither field.

    args
}

/// Escapes a value for a virt-viewer `.vv` INI line.
///
/// The `.vv` format is a GLib key file and virt-viewer reads it with
/// `g_key_file_get_string`, which un-escapes the value it is given. That makes
/// escaping a correctness requirement and not only an injection guard, because
/// GKeyFile is strict about what it will accept:
///
/// * a backslash followed by anything other than `n`, `r`, `t`, `s` or `\` makes
///   the read fail outright with "value that cannot be interpreted", so a
///   password containing a single backslash would take the whole file down;
/// * `\\` decodes back to one backslash, so a backslash left unescaped is
///   silently altered even when it happens to form a valid sequence;
/// * leading whitespace is stripped, so a password that starts with a space
///   arrives short unless the space is written as `\s`;
/// * a raw newline or carriage return ends the line and would forge or truncate
///   a key.
///
/// Every one of those characters is therefore written in its escaped form, which
/// round-trips exactly. Escaping spaces everywhere rather than only at the front
/// keeps the rule simple; `\s` decodes to a space in any position.
fn escape_vv_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ' ' => escaped.push_str("\\s"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

/// Builds the contents of a virt-viewer `.vv` connection file, or `None`.
///
/// A `.vv` file is the only way to hand `remote-viewer` a password without
/// putting it on argv, where `/proc/<pid>/cmdline` would expose it to every
/// process of the same user (issue [#308]). The file carries `password=` in a
/// `[virt-viewer]` section together with the connection parameters, plus
/// `delete-this-file=1` so the viewer removes it after reading. The caller is
/// responsible for writing it mode-0600 to a user-private directory and for
/// removing it if the viewer does not.
///
/// Returns `None` — and the caller falls back to the plain
/// [`build_spice_viewer_args`] URI, which lets the viewer prompt — when there is
/// no password to deliver, or when the connection uses a unix socket. The `.vv`
/// format addresses the target by `host`/`port`/`tls-port` and has no field for
/// a `spice+unix://` path, so a socket connection cannot be expressed here; a
/// local socket also needs no password in practice.
///
/// [#308]: https://github.com/totoshko88/RustConn/issues/308
#[must_use]
pub fn build_vv_connection_file(config: &SpiceClientConfig) -> Option<String> {
    use secrecy::ExposeSecret;
    use std::fmt::Write as _;

    // No socket support in the .vv format, and a socket needs no password.
    if config.unix_socket_path.is_some() {
        return None;
    }
    let password = config.password.as_ref()?;
    let password = password.expose_secret();
    if password.is_empty() {
        return None;
    }

    let mut file = String::from("[virt-viewer]\ntype=spice\n");
    let _ = writeln!(file, "host={}", escape_vv_value(&config.host));
    if config.tls_enabled {
        // tls-port is the encrypted port; `port` stays unset so the viewer does
        // not also attempt a plaintext channel.
        let _ = writeln!(file, "tls-port={}", config.port);
        if let Some(ref ca_path) = config.ca_cert_path
            && let Ok(ca) = std::fs::read_to_string(ca_path)
        {
            // The CA PEM is a multi-line value; the .vv format joins the lines
            // with literal `\n` sequences, matching oVirt's generated files.
            // Same escaper as every other value, so the PEM survives GKeyFile's
            // un-escaping intact instead of relying on it holding no backslash.
            let _ = writeln!(file, "ca={}", escape_vv_value(&ca));
        }
        if config.skip_cert_verify {
            // An empty host-subject disables subject matching, the closest the
            // format offers to the argv `--spice-host-subject ""` this replaces.
            file.push_str("host-subject=\n");
        }
    } else {
        let _ = writeln!(file, "port={}", config.port);
    }
    if let Some(ref proxy) = config.proxy {
        let _ = writeln!(file, "proxy={}", escape_vv_value(proxy));
    }
    // The deletion flag goes in before the password, so a file truncated by a
    // partial write can hold the flag without the secret but never the secret
    // without the flag that cleans it up.
    file.push_str("delete-this-file=1\n");
    let _ = writeln!(file, "password={}", escape_vv_value(password));

    Some(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_spice_viewer_args_basic() {
        let config = SpiceClientConfig::new("192.168.1.100").with_port(5900);
        let args = build_spice_viewer_args(&config);

        assert!(args.contains(&"spice://192.168.1.100:5900".to_string()));
        assert!(args.contains(&"--title".to_string()));
    }

    #[test]
    fn test_build_spice_viewer_args_with_tls() {
        let config = SpiceClientConfig::new("secure.example.com")
            .with_port(5901)
            .with_tls(true)
            .with_skip_cert_verify(true);
        let args = build_spice_viewer_args(&config);

        assert!(args.contains(&"spice+tls://secure.example.com:5901".to_string()));
    }

    #[test]
    fn test_build_spice_viewer_args_with_usb() {
        let config = SpiceClientConfig::new("localhost").with_usb_redirection(true);
        let args = build_spice_viewer_args(&config);

        assert!(args.contains(&"--spice-usbredir-auto-redirect-filter".to_string()));
    }

    #[test]
    fn test_build_spice_viewer_args_with_shared_folder() {
        let folder = SpiceSharedFolder::new("/home/user/share", "MyShare");
        let config = SpiceClientConfig::new("localhost").with_shared_folder(folder);
        let args = build_spice_viewer_args(&config);

        assert!(args.contains(&"--spice-shared-dir".to_string()));
        assert!(args.contains(&"/home/user/share".to_string()));
    }

    #[test]
    fn test_build_spice_viewer_args_no_audio() {
        let config = SpiceClientConfig::new("localhost").with_audio_playback(false);
        let args = build_spice_viewer_args(&config);

        assert!(args.contains(&"--spice-disable-audio".to_string()));
    }

    #[test]
    fn test_build_spice_viewer_args_with_ca_cert() {
        let config = SpiceClientConfig::new("localhost")
            .with_tls(true)
            .with_ca_cert("/etc/ssl/certs/ca.crt");
        let args = build_spice_viewer_args(&config);

        assert!(args.contains(&"--spice-ca-file".to_string()));
        assert!(args.contains(&"/etc/ssl/certs/ca.crt".to_string()));
    }

    #[test]
    fn test_build_spice_viewer_args_with_proxy() {
        let config = SpiceClientConfig::new("localhost").with_proxy("http://192.168.1.100:3128");
        let args = build_spice_viewer_args(&config);

        assert!(args.contains(&"--spice-proxy".to_string()));
        assert!(args.contains(&"http://192.168.1.100:3128".to_string()));
    }

    #[test]
    fn test_build_spice_viewer_args_unix_socket() {
        // Unix-socket mode uses the spice+unix:// scheme and ignores host:port.
        let config = SpiceClientConfig::new("ignored-host")
            .with_port(5900)
            .with_tls(true) // must not produce spice+tls:// in socket mode
            .with_unix_socket("/run/libvirt/qemu/vm-spice.sock");
        let args = build_spice_viewer_args(&config);

        assert!(args.contains(&"spice+unix:///run/libvirt/qemu/vm-spice.sock".to_string()));
        assert!(!args.iter().any(|a| a.starts_with("spice://")));
        assert!(!args.iter().any(|a| a.starts_with("spice+tls://")));
    }

    #[test]
    fn vv_file_none_without_password() {
        let config = SpiceClientConfig::new("192.168.1.100").with_port(5900);
        assert!(build_vv_connection_file(&config).is_none());
    }

    #[test]
    fn vv_file_none_for_empty_password() {
        let config = SpiceClientConfig::new("192.168.1.100")
            .with_port(5900)
            .with_password("");
        assert!(build_vv_connection_file(&config).is_none());
    }

    #[test]
    fn vv_file_none_for_unix_socket() {
        // The .vv format has no socket field, and a local socket needs no pass.
        let config = SpiceClientConfig::new("ignored")
            .with_password("secret")
            .with_unix_socket("/run/libvirt/qemu/vm-spice.sock");
        assert!(build_vv_connection_file(&config).is_none());
    }

    #[test]
    fn vv_file_plain_tcp_carries_password_and_delete_flag() {
        let config = SpiceClientConfig::new("192.168.1.100")
            .with_port(5900)
            .with_password("s3cret");
        let file = build_vv_connection_file(&config).expect("vv file");

        assert!(file.starts_with("[virt-viewer]\n"));
        assert!(file.contains("type=spice\n"));
        assert!(file.contains("host=192.168.1.100\n"));
        assert!(file.contains("port=5900\n"));
        assert!(!file.contains("tls-port="));
        assert!(file.contains("password=s3cret\n"));
        assert!(file.contains("delete-this-file=1\n"));
    }

    #[test]
    fn vv_file_tls_uses_tls_port_not_port() {
        let config = SpiceClientConfig::new("secure.example.com")
            .with_port(5901)
            .with_tls(true)
            .with_skip_cert_verify(true)
            .with_password("s3cret");
        let file = build_vv_connection_file(&config).expect("vv file");

        assert!(file.contains("tls-port=5901\n"));
        assert!(!file.contains("\nport="));
        // skip_cert_verify maps to an empty host-subject.
        assert!(file.contains("host-subject=\n"));
    }

    #[test]
    fn vv_file_escapes_newlines_in_password() {
        // A raw newline in the value would forge a second .vv key; the escaped
        // form keeps the line intact and still round-trips through GKeyFile.
        let config = SpiceClientConfig::new("192.168.1.100")
            .with_port(5900)
            .with_password("s3cret\nusb-filter=null");
        let file = build_vv_connection_file(&config).expect("vv file");

        assert!(file.contains("password=s3cret\\nusb-filter=null\n"));
        // Exactly one password key, and no injected key on its own line.
        assert_eq!(file.matches("password=").count(), 1);
        assert!(!file.contains("\nusb-filter="));
    }

    #[test]
    fn vv_file_escapes_backslash_and_space_in_password() {
        // GKeyFile refuses a value whose backslash starts no known escape
        // sequence, and strips leading whitespace. Both have to be encoded or
        // the viewer either fails to read the file or reads a shorter password.
        let config = SpiceClientConfig::new("192.168.1.100")
            .with_port(5900)
            .with_password(" pa\\ss word\t");
        let file = build_vv_connection_file(&config).expect("vv file");

        assert!(file.contains("password=\\spa\\\\ss\\sword\\t\n"));
        // The escaped value stays on one line whatever it contained.
        assert_eq!(file.matches("password=").count(), 1);
    }

    #[test]
    fn vv_file_writes_the_delete_flag_before_the_password() {
        // A truncated write must never leave the secret behind without the flag
        // that has the viewer remove the file.
        let config = SpiceClientConfig::new("192.168.1.100")
            .with_port(5900)
            .with_password("s3cret");
        let file = build_vv_connection_file(&config).expect("vv file");

        let flag = file.find("delete-this-file=1").expect("delete flag");
        let password = file.find("password=").expect("password key");
        assert!(flag < password);
    }

    #[test]
    fn vv_file_includes_proxy() {
        let config = SpiceClientConfig::new("192.168.1.100")
            .with_port(5900)
            .with_password("s3cret")
            .with_proxy("http://proxy:3128");
        let file = build_vv_connection_file(&config).expect("vv file");

        assert!(file.contains("proxy=http://proxy:3128\n"));
    }

    #[test]
    fn extra_flags_keep_the_audio_setting() {
        // The .vv format has no audio field, so the flag has to survive on argv
        // or a muted connection unmutes itself as soon as it gets a password.
        let muted = SpiceClientConfig::new("192.168.1.100")
            .with_port(5900)
            .with_audio_playback(false);
        assert!(
            build_spice_extra_flags(&muted).contains(&"--spice-disable-audio".to_string()),
            "audio playback off must still disable audio on the .vv path"
        );

        let audible = SpiceClientConfig::new("192.168.1.100")
            .with_port(5900)
            .with_audio_playback(true);
        assert!(!build_spice_extra_flags(&audible).contains(&"--spice-disable-audio".to_string()));
    }

    #[test]
    fn extra_flags_have_no_uri_and_no_duplicated_vv_fields() {
        let folder = SpiceSharedFolder::new("/home/user/share", "MyShare");
        let config = SpiceClientConfig::new("192.168.1.100")
            .with_port(5900)
            .with_tls(true)
            .with_ca_cert("/etc/ssl/ca.crt")
            .with_proxy("http://proxy:3128")
            .with_usb_redirection(true)
            .with_shared_folder(folder);
        let flags = build_spice_extra_flags(&config);

        // Title and the connection-independent flags are present.
        assert!(flags.contains(&"--title".to_string()));
        assert!(flags.contains(&"SPICE: 192.168.1.100".to_string()));
        assert!(flags.contains(&"--spice-usbredir-auto-redirect-filter".to_string()));
        assert!(flags.contains(&"--spice-shared-dir".to_string()));
        assert!(flags.contains(&"/home/user/share".to_string()));

        // Nothing the .vv file already carries is repeated on argv.
        assert!(!flags.iter().any(|a| a.starts_with("spice://")));
        assert!(!flags.iter().any(|a| a.starts_with("spice+tls://")));
        assert!(!flags.contains(&"--spice-proxy".to_string()));
        assert!(!flags.contains(&"--spice-ca-file".to_string()));
        assert!(!flags.contains(&"--spice-host-subject".to_string()));
    }
}
