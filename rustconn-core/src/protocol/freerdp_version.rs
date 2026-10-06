//! FreeRDP client versions and the FreeRDP 3 requirement (issue #351).
//!
//! Every RustConn launch of FreeRDP — the external client, the fallback from
//! the embedded IronRDP client and the embedded `wlfreerdp` — hands the whole
//! command line over in one `/args-from:` file, so the password never reaches
//! the process's argument vector. That switch, like `/sec:nla:off`, only exists
//! in FreeRDP 3. FreeRDP 2 rejects the command line and prints its usage banner
//! instead of connecting, so a FreeRDP 2 client is never launched.
//!
//! The binary name cannot tell the two apart. Debian and Ubuntu ship FreeRDP 2
//! as `xfreerdp` and `wlfreerdp`, while Arch, Fedora and the Flatpak ship
//! FreeRDP 3 under those same names, so the decision is made from the version
//! the client reports. This module holds the part of that decision that needs
//! no process: the version type and its parser, the rule for a client whose
//! version could not be read, and the choice among candidates given a probe.
//! The GUI supplies the probe, which also has to look on a Flatpak host.

use std::fmt;
use std::path::Path;

/// The oldest FreeRDP major release RustConn launches.
pub const MIN_SUPPORTED_FREERDP_MAJOR: u32 = 3;

/// A FreeRDP release number, as `<client> --version` reports it.
///
/// Ordered by `major`, then `minor`, then `patch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FreeRdpVersion {
    /// Major release number.
    pub major: u32,
    /// Minor release number.
    pub minor: u32,
    /// Patch level; `0` when the banner gave only two components.
    pub patch: u32,
}

impl FreeRdpVersion {
    /// Creates a version from its three components.
    #[must_use]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Whether RustConn can launch a client of this version: FreeRDP 3 or newer.
    #[must_use]
    pub const fn is_supported(self) -> bool {
        self.major >= MIN_SUPPORTED_FREERDP_MAJOR
    }
}

impl fmt::Display for FreeRdpVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Reads the FreeRDP version out of a client's `--version` output.
///
/// FreeRDP 2 and 3 both print `This is FreeRDP version 3.5.1 (n/a)`. A line
/// naming FreeRDP and a version is preferred over any other line that mentions
/// FreeRDP, so a log line printed ahead of the banner cannot supply a stray
/// number. Pre-release suffixes (`3.0.0-dev`, `-rc1`) are ignored and a missing
/// patch level reads as `0`. Returns `None` when the output carries no such
/// version — which is what FreeRDP 2 prints when it rejects a command line
/// instead of reporting one.
#[must_use]
pub fn parse_freerdp_version(output: &str) -> Option<FreeRdpVersion> {
    output
        .lines()
        .filter(|line| line.to_ascii_lowercase().contains("version"))
        .find_map(version_after_freerdp)
        .or_else(|| output.lines().find_map(version_after_freerdp))
}

/// The first version-shaped token after the word "FreeRDP" on `line`.
fn version_after_freerdp(line: &str) -> Option<FreeRdpVersion> {
    // `to_ascii_lowercase` keeps every byte offset, so the match indexes `line`.
    let offset = line.to_ascii_lowercase().find("freerdp")?;
    line[offset..]
        .split_whitespace()
        .find_map(parse_version_token)
}

/// Parses `3.5.1`, `v3.26.0`, `(2.11.5)` or `3.0.0-dev`; a minor is required.
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
    Some(FreeRdpVersion::new(major, minor, patch))
}

/// Whether a FreeRDP client may be launched, given what its version probe found.
///
/// A version that was read decides on its own. Without one — a probe that
/// timed out, could not start, or printed no banner — only the name is left,
/// and only the `3`-suffixed names (`sdl-freerdp3`, `wlfreerdp3`, `xfreerdp3`)
/// are FreeRDP 3 by construction. An unsuffixed `xfreerdp` or `wlfreerdp` is
/// FreeRDP 2 on Debian and Ubuntu, so without a version it is refused rather
/// than handed a command line it may reject, which is the failure this rule
/// exists to stop.
///
/// Trusting a `3`-suffixed name without a version is NOT free of risk: on
/// Debian and Ubuntu `xfreerdp3`/`wlfreerdp3` are alternatives that can point
/// at a FreeRDP **2** build, which then rejects the `/args-from:` command line
/// and exits 255. We keep the trust (refusing it would turn a slow-but-healthy
/// FreeRDP 3 probe into a spurious "install FreeRDP 3" error) but the caller
/// should make the probe reliable first — the real guard is not caching a
/// failed probe and giving `--version` enough time — and log the name-only
/// launch so this path is visible in a bug report. `binary` may carry a
/// `host:` marker or be a full path; only its file name counts.
#[must_use]
pub fn is_supported_freerdp_client(binary: &str, version: Option<FreeRdpVersion>) -> bool {
    version.map_or_else(
        || {
            let trusted_by_name = freerdp_file_name(binary).ends_with('3');
            if trusted_by_name {
                tracing::warn!(
                    protocol = "rdp",
                    binary,
                    "FreeRDP version could not be read; trusting the '3'-suffixed \
                     name as FreeRDP 3. If this is a FreeRDP 2 behind the name \
                     (Debian/Ubuntu alternatives), the external client will exit \
                     255 — install a real FreeRDP 3 or set an explicit client."
                );
            }
            trusted_by_name
        },
        FreeRdpVersion::is_supported,
    )
}

/// The file name of a FreeRDP candidate, without a `host:` marker or a directory.
fn freerdp_file_name(binary: &str) -> &str {
    let binary = binary.strip_prefix("host:").unwrap_or(binary);
    Path::new(binary)
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(binary)
}

/// What looking for one FreeRDP candidate found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreeRdpProbe {
    /// The candidate is not installed.
    Missing,
    /// The candidate is installed; this is its version, when it could be read.
    Installed(Option<FreeRdpVersion>),
}

/// An installed FreeRDP client that RustConn will not launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedFreeRdp {
    /// The candidate as it would have been launched, `host:` marker included.
    pub binary: String,
    /// Its version, when the probe could read one.
    pub version: Option<FreeRdpVersion>,
}

/// The outcome of choosing a FreeRDP client from a preference-ordered list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FreeRdpSelection {
    /// The first candidate RustConn can launch.
    Supported(String),
    /// FreeRDP is installed, but no candidate is FreeRDP 3.
    Unsupported(UnsupportedFreeRdp),
    /// No candidate is installed at all.
    NotInstalled,
}

impl FreeRdpSelection {
    /// The client to launch, if one was chosen.
    #[must_use]
    pub fn into_supported(self) -> Option<String> {
        match self {
            Self::Supported(binary) => Some(binary),
            Self::Unsupported(_) | Self::NotInstalled => None,
        }
    }

    /// The version of the refused FreeRDP, when that is why nothing was chosen.
    #[must_use]
    pub fn unsupported_version(&self) -> Option<FreeRdpVersion> {
        match self {
            Self::Unsupported(unsupported) => unsupported.version,
            Self::Supported(_) | Self::NotInstalled => None,
        }
    }
}

/// Picks the first candidate RustConn can launch, asking `probe` about each in
/// order.
///
/// Candidates after the chosen one are never probed, so a version probe is paid
/// for only down to the first usable client, and the preference order of the
/// list — SDL3 first in a Wayland session (issue #340) — is kept among the
/// clients that qualify. When nothing qualifies, the result tells a missing
/// FreeRDP apart from one RustConn refuses, and names the first refused client,
/// preferring one whose version is known so the user can be told which FreeRDP
/// they have.
#[must_use]
pub fn select_freerdp_client<I, S, F>(candidates: I, mut probe: F) -> FreeRdpSelection
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    F: FnMut(&str) -> FreeRdpProbe,
{
    let mut refused: Option<UnsupportedFreeRdp> = None;
    for candidate in candidates {
        let binary = candidate.as_ref();
        let FreeRdpProbe::Installed(version) = probe(binary) else {
            continue;
        };
        if is_supported_freerdp_client(binary, version) {
            return FreeRdpSelection::Supported(binary.to_string());
        }
        tracing::debug!(
            protocol = "rdp",
            binary,
            version = ?version,
            "Skipping FreeRDP client: RustConn needs FreeRDP 3"
        );
        if refused
            .as_ref()
            .is_none_or(|first| first.version.is_none() && version.is_some())
        {
            refused = Some(UnsupportedFreeRdp {
                binary: binary.to_string(),
                version,
            });
        }
    }
    match refused {
        Some(refused) => FreeRdpSelection::Unsupported(refused),
        None => FreeRdpSelection::NotInstalled,
    }
}

/// Keeps the candidates RustConn can launch, in their original order.
///
/// Seeds the connection editor's FreeRDP client list, which must not offer a
/// client the launcher would then refuse. Each candidate is decided by the same
/// rule as in [`select_freerdp_client`].
#[must_use]
pub fn launchable_freerdp_clients<I, S, F>(candidates: I, mut probe: F) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    F: FnMut(&str) -> FreeRdpProbe,
{
    candidates
        .into_iter()
        .filter_map(|candidate| {
            let binary = candidate.as_ref();
            match probe(binary) {
                FreeRdpProbe::Installed(version)
                    if is_supported_freerdp_client(binary, version) =>
                {
                    Some(binary.to_string())
                }
                FreeRdpProbe::Installed(_) | FreeRdpProbe::Missing => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{FREERDP_WAYLAND_FIRST, FREERDP_X11_FIRST};

    const V2: Option<FreeRdpVersion> = Some(FreeRdpVersion::new(2, 11, 5));
    const V3: Option<FreeRdpVersion> = Some(FreeRdpVersion::new(3, 32, 1));

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

    fn unsupported(binary: &str, version: Option<FreeRdpVersion>) -> FreeRdpSelection {
        FreeRdpSelection::Unsupported(UnsupportedFreeRdp {
            binary: binary.to_string(),
            version,
        })
    }

    #[test]
    fn parses_freerdp_version_banners() {
        for (output, expected) in [
            // FreeRDP 2 on Ubuntu 24.04, the reporter's `wlfreerdp` (#351).
            (
                "This is FreeRDP version 2.11.5 (2.11.5)",
                FreeRdpVersion::new(2, 11, 5),
            ),
            // Ubuntu 24.04's FreeRDP 3 build has no git revision.
            (
                "This is FreeRDP version 3.5.1 (n/a)",
                FreeRdpVersion::new(3, 5, 1),
            ),
            // The Flatpak's bundled client, followed by build information.
            (
                "This is FreeRDP version 3.32.1 (3.32.1)\nBuild configuration: \
                 BUILD_TESTING=OFF WITH_X11=OFF\nBuild type: Release",
                FreeRdpVersion::new(3, 32, 1),
            ),
            (
                "This is FreeRDP version 3.0.0-dev (a1b2c3d)",
                FreeRdpVersion::new(3, 0, 0),
            ),
            (
                "This is FreeRDP version 3.24.2 (3.24.2)",
                FreeRdpVersion::new(3, 24, 2),
            ),
            (
                "THIS IS FREERDP VERSION v3.26.0",
                FreeRdpVersion::new(3, 26, 0),
            ),
            ("FreeRDP build: 3.27.0-rc1", FreeRdpVersion::new(3, 27, 0)),
            ("FreeRDP version 4.0", FreeRdpVersion::new(4, 0, 0)),
        ] {
            assert_eq!(parse_freerdp_version(output), Some(expected), "{output:?}");
        }
    }

    /// A log line printed before the banner must not supply the version.
    #[test]
    fn the_banner_line_wins_over_an_earlier_log_line() {
        let output = "[13:22:50:123] [4711:4712] [WARN][com.freerdp.client.common] - \
                      option 1.2.3 is deprecated\nThis is FreeRDP version 3.5.1 (n/a)";
        assert_eq!(
            parse_freerdp_version(output),
            Some(FreeRdpVersion::new(3, 5, 1))
        );
    }

    #[test]
    fn rejects_output_without_a_freerdp_version() {
        for output in [
            "",
            "FreeRDP version unknown",
            "version 3.26.0",
            "garbage 1.2.3",
            "FreeRDP 3",
            // What FreeRDP 2 prints for a command line it does not understand,
            // such as `/args-from:` — the black-screen report in #351.
            "wlfreerdp - A Free Remote Desktop Protocol Implementation\n\
             To show full command line help type\nwlfreerdp /?",
        ] {
            assert_eq!(parse_freerdp_version(output), None, "{output:?}");
        }
    }

    #[test]
    fn only_freerdp_3_and_newer_is_supported() {
        assert!(!FreeRdpVersion::new(2, 11, 5).is_supported());
        assert!(!FreeRdpVersion::new(2, 99, 99).is_supported());
        assert!(FreeRdpVersion::new(3, 0, 0).is_supported());
        assert!(FreeRdpVersion::new(4, 0, 0).is_supported());
    }

    #[test]
    fn versions_order_and_display_as_released() {
        let legacy = FreeRdpVersion::new(2, 11, 5);
        let current = FreeRdpVersion::new(3, 5, 1);
        assert!(current > legacy);
        assert!(FreeRdpVersion::new(3, 26, 0) > current);
        assert_eq!(FreeRdpVersion::new(3, 32, 1).to_string(), "3.32.1");
    }

    #[test]
    fn a_read_version_decides_over_the_name() {
        assert!(is_supported_freerdp_client("xfreerdp", V3));
        assert!(is_supported_freerdp_client("wlfreerdp", V3));
        assert!(!is_supported_freerdp_client("wlfreerdp", V2));
        assert!(!is_supported_freerdp_client("xfreerdp3", V2));
    }

    /// The documented choice for a client whose version could not be read:
    /// only the `3`-suffixed names are FreeRDP 3 by construction.
    #[test]
    fn an_unknown_version_keeps_only_3_suffixed_names() {
        for kept in [
            "sdl-freerdp3",
            "wlfreerdp3",
            "xfreerdp3",
            "host:xfreerdp3",
            "/usr/local/bin/sdl-freerdp3",
        ] {
            assert!(is_supported_freerdp_client(kept, None), "{kept}");
        }
        for refused in [
            "sdl-freerdp",
            "wlfreerdp",
            "xfreerdp",
            "freerdp",
            "host:xfreerdp",
        ] {
            assert!(!is_supported_freerdp_client(refused, None), "{refused}");
        }
    }

    /// The reporter's Ubuntu 24.04 with `freerdp3-x11` added: the FreeRDP 2
    /// `wlfreerdp` comes first in a Wayland session and must be passed over.
    #[test]
    fn freerdp_2_is_skipped_and_freerdp_3_kept() {
        let clients = [("wlfreerdp", V2), ("xfreerdp3", None)];
        assert_eq!(
            select_freerdp_client(FREERDP_WAYLAND_FIRST, installed(&clients)),
            FreeRdpSelection::Supported("xfreerdp3".to_string())
        );
    }

    /// Supported clients keep today's preference: SDL3 first on Wayland, the
    /// X11 client first on X11 (issue #340).
    #[test]
    fn the_session_order_is_kept_among_supported_clients() {
        let clients = [("sdl-freerdp3", V3), ("wlfreerdp3", V3), ("xfreerdp3", V3)];
        assert_eq!(
            select_freerdp_client(FREERDP_WAYLAND_FIRST, installed(&clients)),
            FreeRdpSelection::Supported("sdl-freerdp3".to_string())
        );
        assert_eq!(
            select_freerdp_client(FREERDP_X11_FIRST, installed(&clients)),
            FreeRdpSelection::Supported("xfreerdp3".to_string())
        );
    }

    /// Arch, Fedora and the Flatpak ship FreeRDP 3 without the suffix; their
    /// probe reports 3.x, so those clients are still chosen.
    #[test]
    fn unsuffixed_freerdp_3_clients_are_chosen() {
        let clients = [("sdl-freerdp", V3), ("wlfreerdp", V3)];
        assert_eq!(
            select_freerdp_client(FREERDP_WAYLAND_FIRST, installed(&clients)),
            FreeRdpSelection::Supported("sdl-freerdp".to_string())
        );
    }

    #[test]
    fn only_freerdp_2_installed_names_the_refused_version() {
        let clients = [("wlfreerdp", V2), ("xfreerdp", V2)];
        let selection = select_freerdp_client(FREERDP_WAYLAND_FIRST, installed(&clients));
        assert_eq!(selection, unsupported("wlfreerdp", V2));
        assert_eq!(selection.unsupported_version(), V2);
        assert_eq!(selection.into_supported(), None);
    }

    /// The refused client reported is one whose version is known, so the
    /// notification can say which FreeRDP is installed.
    #[test]
    fn a_refused_client_with_a_known_version_is_preferred_for_the_report() {
        let clients = [("wlfreerdp", None), ("xfreerdp", V2)];
        assert_eq!(
            select_freerdp_client(FREERDP_WAYLAND_FIRST, installed(&clients)),
            unsupported("xfreerdp", V2)
        );

        let unknown_only = [("xfreerdp", None)];
        let selection = select_freerdp_client(FREERDP_X11_FIRST, installed(&unknown_only));
        assert_eq!(selection, unsupported("xfreerdp", None));
        assert_eq!(selection.unsupported_version(), None);
    }

    #[test]
    fn nothing_installed_is_not_installed() {
        let selection = select_freerdp_client(FREERDP_WAYLAND_FIRST, installed(&[]));
        assert_eq!(selection, FreeRdpSelection::NotInstalled);
        assert_eq!(selection.unsupported_version(), None);
    }

    /// A connection pinned to a FreeRDP 2 client: the pin goes first in the
    /// candidate list, is refused, and auto-detection takes over.
    #[test]
    fn a_pinned_freerdp_2_client_falls_back_to_auto_detection() {
        let pinned = std::iter::once("xfreerdp");
        let candidates = pinned.chain(FREERDP_WAYLAND_FIRST.iter().copied());
        let clients = [("xfreerdp", V2), ("sdl-freerdp3", None)];
        assert_eq!(
            select_freerdp_client(candidates, installed(&clients)),
            FreeRdpSelection::Supported("sdl-freerdp3".to_string())
        );
    }

    #[test]
    fn candidates_after_the_chosen_one_are_not_probed() {
        let mut probed = Vec::new();
        let selection = select_freerdp_client(FREERDP_WAYLAND_FIRST, |binary: &str| {
            probed.push(binary.to_string());
            if binary == "sdl-freerdp" {
                FreeRdpProbe::Installed(V3)
            } else {
                FreeRdpProbe::Missing
            }
        });
        assert_eq!(
            selection,
            FreeRdpSelection::Supported("sdl-freerdp".to_string())
        );
        assert_eq!(probed, ["sdl-freerdp3", "sdl-freerdp"]);
    }

    #[test]
    fn the_editor_list_leaves_out_freerdp_2() {
        let candidates = [
            "sdl-freerdp3",
            "sdl-freerdp",
            "wlfreerdp3",
            "wlfreerdp",
            "xfreerdp3",
            "xfreerdp",
        ];
        let clients = [
            ("sdl-freerdp3", V3),
            ("wlfreerdp", V2),
            ("xfreerdp3", None),
            ("xfreerdp", V2),
        ];
        assert_eq!(
            launchable_freerdp_clients(candidates, installed(&clients)),
            ["sdl-freerdp3", "xfreerdp3"]
        );
    }
}
