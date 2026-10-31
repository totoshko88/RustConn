//! EGFX Graphics Pipeline handler for the RDP client.
//!
//! `ironrdp-egfx` decodes every codec a Windows host sends over the graphics
//! pipeline — AVC420, AVC444, ClearCodec, Planar, RFX Progressive — and
//! composites surfaces, solid fills, surface copies and the bitmap cache
//! itself. `ironrdp-session` then paints each completed frame into the
//! session image and reports it as `ActiveStageOutput::GraphicsUpdate`, the
//! same output bitmap updates produce, so the session loop needs nothing of
//! its own for GFX. [`RustConnGfxHandler`] only tells the GUI what happens on
//! the pipeline: the mode that was negotiated, a resolution reset, and content
//! that could not be decoded.
//!
//! Up to `ironrdp-egfx` 0.3 the handler kept an RGBA copy of every surface and
//! of the bitmap cache, because that version decoded but did not composite, so
//! scrolling, window moves and cache blits painted nothing (issue [#262]). From
//! 0.4 that copy would only repeat IronRDP's own compositor, frame by frame.
//!
//! [#262]: https://github.com/totoshko88/RustConn/issues/262

use std::sync::mpsc;

use ironrdp_egfx::client::{BitmapUpdate, GraphicsPipelineHandler};
use ironrdp_egfx::decode::H264Decoder;
use ironrdp_egfx::pdu::{
    CapabilitiesV10Flags, CapabilitiesV81Flags, CapabilitiesV103Flags, CapabilitiesV104Flags,
    CapabilitiesV107Flags, CapabilitySet, GfxPdu,
};

use super::RdpClientEvent;
use super::graphics::GraphicsMode;

/// Dropped surface updates that must pile up before the GUI is told the server
/// is using a codec we cannot decode.
///
/// Frames arrive in quick succession, so this is well under a second on a live
/// session; it exists only to avoid reacting to a single odd region.
const UNSUPPORTED_CODEC_THRESHOLD: u32 = 5;

/// Consecutive empty bitmap updates that count as a persistent decode failure.
///
/// ponytail: matches Req 3 AC 5; raise it if servers legitimately send empty
/// frames during codec renegotiation.
const EMPTY_UPDATE_THRESHOLD: u32 = 10;

/// Graphics pipeline events for the GUI.
///
/// The capability sets advertised are `ironrdp-egfx`'s default — every version
/// from 10.7 down to 8 — filtered by the client to what its decoder can
/// honour: no AVC without OpenH264, no AVC444 without YUV output.
pub struct RustConnGfxHandler {
    /// Channel sender for delivering client events (errors, status) to the GUI
    event_tx: mpsc::Sender<RdpClientEvent>,
    /// Consecutive empty bitmap updates (potential persistent decode failure)
    consecutive_empty: u32,
    /// Whether [`RdpClientEvent::GfxDecodeFailure`] was already reported for
    /// the current run of empty updates, so the GUI hears about it once
    decode_failure_reported: bool,
    /// Surface updates dropped because their codec has no decoder
    unsupported_codec_frames: u32,
    /// Whether the EGFX pipeline has completed capability negotiation
    active: bool,
}

impl RustConnGfxHandler {
    /// Creates a new GFX handler sending its events through `event_tx`.
    #[must_use]
    pub const fn new(event_tx: mpsc::Sender<RdpClientEvent>) -> Self {
        Self {
            event_tx,
            consecutive_empty: 0,
            decode_failure_reported: false,
            unsupported_codec_frames: 0,
            active: false,
        }
    }

    /// Returns whether the EGFX pipeline is active (capabilities confirmed).
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }

    /// Records surface content the pipeline has no decoder for, and reports it
    /// to the GUI once enough of it has been dropped.
    ///
    /// Such content is lost: nothing reaches `on_bitmap_updated`, so without
    /// this the session would just look frozen — the failure shape behind
    /// issue #262.
    fn note_undecodable_content(&mut self, codec: &str, surface_id: u16) {
        self.unsupported_codec_frames = self.unsupported_codec_frames.saturating_add(1);

        if self.unsupported_codec_frames == 1 {
            tracing::warn!(
                codec,
                surface_id,
                "EGFX surface content arrived in a format the pipeline cannot decode"
            );
        }
        if self.unsupported_codec_frames == UNSUPPORTED_CODEC_THRESHOLD {
            tracing::error!(
                codec,
                dropped_frames = self.unsupported_codec_frames,
                "GFX undecodable content — the session cannot paint through this pipeline"
            );
            let _ = self.event_tx.send(RdpClientEvent::GfxUnsupportedCodec {
                codec: codec.to_owned(),
                dropped_frames: self.unsupported_codec_frames,
            });
        }
    }

    /// Counts an empty bitmap update, reporting a persistent decode failure
    /// once [`EMPTY_UPDATE_THRESHOLD`] of them arrive in a row.
    fn note_empty_update(&mut self, surface_id: u16) {
        self.consecutive_empty = self.consecutive_empty.saturating_add(1);
        if self.consecutive_empty >= EMPTY_UPDATE_THRESHOLD && !self.decode_failure_reported {
            self.decode_failure_reported = true;
            tracing::error!(
                consecutive_empty = self.consecutive_empty,
                surface_id,
                "Persistent decode failure — consecutive empty bitmap updates"
            );
            // Signal the GUI about persistent decode failure so it can
            // display a degraded-quality warning (Req 6 AC 3, Req 10 AC 3).
            let _ = self.event_tx.send(RdpClientEvent::GfxDecodeFailure {
                consecutive_failures: self.consecutive_empty,
            });
        }
    }
}

impl GraphicsPipelineHandler for RustConnGfxHandler {
    fn on_capabilities_confirmed(&mut self, caps: &CapabilitySet) {
        self.active = true;
        let mode = negotiated_graphics_mode(caps);
        tracing::info!(?caps, ?mode, "EGFX capabilities confirmed");
        // Tell the GUI what really negotiated. Until 0.19.15 the status bar
        // reported a compile-time constant, so it claimed "GFX + H.264" even
        // for sessions that never brought the GFX channel up (issue #262).
        let _ = self
            .event_tx
            .send(RdpClientEvent::GraphicsModeActive { mode });
    }

    fn on_reset_graphics(&mut self, width: u32, height: u32) {
        self.consecutive_empty = 0;
        self.decode_failure_reported = false;

        // ironrdp-session resizes the session image itself; the GUI needs to
        // hear about it before the first frame of the new size arrives, and
        // this callback runs while that frame is still being decoded.
        let _ = self.event_tx.send(RdpClientEvent::ResolutionChanged {
            width: u16::try_from(width).unwrap_or(u16::MAX),
            height: u16::try_from(height).unwrap_or(u16::MAX),
        });

        tracing::info!(width, height, "EGFX graphics reset");
    }

    fn on_bitmap_updated(&mut self, update: &BitmapUpdate) {
        // The pixels themselves reach the session image through the
        // compositor; only a run of empty updates matters here.
        if update.data.is_empty() {
            self.note_empty_update(update.surface_id);
        } else {
            self.consecutive_empty = 0;
            self.decode_failure_reported = false;
        }
    }

    fn on_unhandled_pdu(&mut self, pdu: &GfxPdu) {
        let GfxPdu::WireToSurface1(wire) = pdu else {
            tracing::debug!(?pdu, "Unhandled EGFX PDU");
            return;
        };
        self.note_undecodable_content(&format!("{:?}", wire.codec_id), wire.surface_id);
    }

    fn on_frame_complete(&mut self, frame_id: u32) {
        tracing::trace!(frame_id, "EGFX frame complete");
    }

    fn on_close(&mut self) {
        self.active = false;
        tracing::info!("EGFX channel closed");
    }
}

/// Maps a confirmed EGFX capability set onto the graphics mode it represents.
///
/// Version 10 and later carry AVC444 unless the server flags AVC as disabled;
/// 8.1 carries AVC420 when it says so. Anything else is the plain GFX pipeline,
/// so a session is never reported as H.264 when the server turned AVC off.
fn negotiated_graphics_mode(caps: &CapabilitySet) -> GraphicsMode {
    let avc444 = match caps {
        CapabilitySet::V10 { flags } | CapabilitySet::V10_2 { flags } => {
            !flags.contains(CapabilitiesV10Flags::AVC_DISABLED)
        }
        CapabilitySet::V10_1 => true,
        CapabilitySet::V10_3 { flags } => !flags.contains(CapabilitiesV103Flags::AVC_DISABLED),
        CapabilitySet::V10_4 { flags }
        | CapabilitySet::V10_5 { flags }
        | CapabilitySet::V10_6 { flags } => !flags.contains(CapabilitiesV104Flags::AVC_DISABLED),
        CapabilitySet::V10_7 { flags } => !flags.contains(CapabilitiesV107Flags::AVC_DISABLED),
        _ => false,
    };
    match caps {
        _ if avc444 => GraphicsMode::GfxAvc444,
        CapabilitySet::V8_1 { flags } if flags.contains(CapabilitiesV81Flags::AVC420_ENABLED) => {
            GraphicsMode::GfxH264
        }
        _ => GraphicsMode::Gfx,
    }
}

// ============================================================================
// OpenH264 loading
// ============================================================================

/// Standard library search paths for OpenH264.
///
/// Flatpak puts it under `/app/lib/`, native installs use `/usr/lib/` or
/// `/usr/lib64/`. The `openh264` crate's libloading backend handles the
/// actual dlopen; we just need to find the `.so` file.
///
/// These are the *unversioned* names, which a distribution ships only in its
/// `-dev`/`-devel` package. A runtime-only install has none of them, which is
/// why [`OPENH264_SEARCH_DIRS`] is scanned for versioned sonames as well.
#[cfg(not(target_os = "macos"))]
const OPENH264_SEARCH_PATHS: &[&str] = &[
    "/app/lib/libopenh264.so",
    "/app/lib64/libopenh264.so",
    "/usr/lib/libopenh264.so",
    "/usr/lib64/libopenh264.so",
    "/usr/lib/x86_64-linux-gnu/libopenh264.so",
    "/usr/lib/aarch64-linux-gnu/libopenh264.so",
];

/// Standard library search paths for OpenH264 on macOS.
///
/// macOS ships Mach-O `.dylib` files, never ELF `.so`, so the Linux list above
/// can never match. The distributed `.app` carries its own copy (see
/// [`bundled_openh264_path`]); these Homebrew prefixes only cover development
/// runs of the bare `target/` binary on Apple Silicon and Intel.
#[cfg(target_os = "macos")]
const OPENH264_SEARCH_PATHS: &[&str] = &[
    "/opt/homebrew/lib/libopenh264.dylib",
    "/usr/local/lib/libopenh264.dylib",
];

/// Returns the OpenH264 copy shipped inside the macOS application bundle.
///
/// The canonical bundle layout is `RustConn.app/Contents/MacOS/rustconn`, so the
/// relocated library sits at `../Frameworks/libopenh264.dylib` relative to the
/// running executable. Returns `None` outside a bundle-like layout or when the
/// executable path cannot be resolved.
#[cfg(target_os = "macos")]
fn bundled_openh264_path() -> Option<std::path::PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let contents = executable.parent()?.parent()?;
    Some(contents.join("Frameworks").join("libopenh264.dylib"))
}

/// Prefix every versioned OpenH264 soname starts with.
const OPENH264_SONAME_PREFIX: &str = "libopenh264.so.";

/// Directories scanned for a versioned OpenH264 soname.
///
/// Needed because [`OPENH264_SEARCH_PATHS`] names only the unversioned
/// `libopenh264.so`, which lives in the `-dev` package. Debian's
/// `libopenh264-8` installs `libopenh264.so.8` and `libopenh264.so.2.6.0` and
/// nothing else, so probing the unversioned name alone reports "not found" on a
/// machine that has the library — and RDP silently drops to the RemoteFX path.
#[cfg(not(target_os = "macos"))]
const OPENH264_SEARCH_DIRS: &[&str] = &[
    "/app/lib",
    "/app/lib64",
    "/usr/lib",
    "/usr/lib64",
    "/usr/lib/x86_64-linux-gnu",
    "/usr/lib/aarch64-linux-gnu",
];

/// Directories scanned for a versioned OpenH264 soname — none on macOS.
///
/// The bundled copy and the Homebrew prefixes are unversioned `.dylib` files
/// that [`OPENH264_SEARCH_PATHS`] and [`bundled_openh264_path`] already name.
/// Mach-O version naming puts the version before the extension
/// (`libopenh264.2.6.0.dylib`), so it would not match the ELF soname pattern
/// this scan looks for anyway.
#[cfg(target_os = "macos")]
const OPENH264_SEARCH_DIRS: &[&str] = &[];

/// Parses the version segments of a versioned OpenH264 soname.
///
/// Returns the dot-separated numbers after `libopenh264.so.`, or `None` when
/// `name` is not a versioned soname of that library. Every segment must parse
/// as a number, which drops companions like `libopenh264.so.debug`.
fn soname_version(name: &str) -> Option<Vec<u64>> {
    let suffix = name.strip_prefix(OPENH264_SONAME_PREFIX)?;
    if suffix.is_empty() {
        return None;
    }
    suffix
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect()
}

/// Returns the versioned OpenH264 sonames among `names`, newest ABI first.
///
/// Takes names rather than a directory so the ordering is testable without a
/// filesystem. Sorted by major version descending — a numeric compare, so
/// `.so.10` outranks `.so.9` where a string sort would not — then by segment
/// count ascending, which prefers the bare soname `libopenh264.so.8` over the
/// `libopenh264.so.8.0.1` it points at, then by the remaining segments
/// descending so the result does not depend on directory order.
fn sonames_newest_first(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut versioned: Vec<(Vec<u64>, String)> = names
        .into_iter()
        .filter_map(|name| soname_version(&name).map(|version| (version, name)))
        .collect();

    versioned.sort_by(|(a, _), (b, _)| {
        b.first()
            .cmp(&a.first())
            .then_with(|| a.len().cmp(&b.len()))
            .then_with(|| b.cmp(a))
    });

    versioned.into_iter().map(|(_, name)| name).collect()
}

/// Returns versioned OpenH264 sonames found on disk, newest ABI first.
///
/// An unreadable directory is skipped: the list is a set of guesses about where
/// a distribution might have put the library, so most of them are expected to
/// be absent on any given machine.
fn versioned_openh264_candidates() -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();

    for dir in OPENH264_SEARCH_DIRS {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let names: Vec<String> = entries
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .collect();
        found.extend(
            sonames_newest_first(names)
                .into_iter()
                .map(|name| std::path::Path::new(dir).join(name)),
        );
    }

    found
}

/// Environment variable naming an OpenH264 library to try before anything else.
///
/// The loader only accepts Cisco's own published binaries (see
/// [`try_load_openh264`]), and no Linux distribution ships one, so the *only*
/// way to get H.264 on a packaged install is to point RustConn at a blob
/// downloaded from `ciscobinary.openh264.org`. Without this there was nowhere to
/// put it except a system directory, i.e. it needed root.
///
/// Read-only: nothing in RustConn ever sets it.
const OPENH264_PATH_ENV: &str = "RUSTCONN_OPENH264";

/// Returns OpenH264 candidates in priority order.
///
/// An explicit [`OPENH264_PATH_ENV`] wins over everything: it is a deliberate
/// choice, and on Linux it is usually the only candidate that can be loaded at
/// all. The bundled copy comes next so a self-contained macOS `.app` never
/// depends on a Homebrew installation at runtime. Unversioned names come before
/// versioned sonames because an unversioned name is a deliberate pointer at the
/// installation the system considers current.
fn openh264_candidates() -> Vec<std::path::PathBuf> {
    let mut candidates = Vec::new();

    // The explicit env override is a deliberate per-run choice and always
    // applies, even when the stored preference disables OpenH264.
    if let Some(explicit) = std::env::var_os(OPENH264_PATH_ENV)
        .map(std::path::PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
    {
        candidates.push(explicit);
    }

    // The user disabled OpenH264 in Settings → Connection → Media Codecs. The
    // licence requires an enable/disable path, so honour it: offer no further
    // candidate (keeping only any explicit env override above) and let the probe
    // report H.264 unavailable, without deleting the downloaded blob.
    if OPENH264_DISABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return candidates;
    }

    // The on-demand Cisco blob the user downloaded via the consent flow
    // (`openh264_download::download_openh264`). It sits in the user cache and,
    // being Cisco's own published binary, is the one library on a packaged
    // Linux install the loader's SHA-256 allow-list actually accepts. It comes
    // AFTER the explicit env override (which is a deliberate per-run choice) but
    // BEFORE the system search paths (whose distribution builds the loader
    // always rejects). `cached_openh264_path` is a pure read — it never triggers
    // a download — and returns `None` unless the file exists AND its hash
    // matches, so a corrupt/partial cache entry is simply skipped here.
    if let Some(cached) = super::openh264_download::cached_openh264_path() {
        candidates.push(cached);
    }

    #[cfg(target_os = "macos")]
    if let Some(bundled) = bundled_openh264_path() {
        candidates.push(bundled);
    }

    candidates.extend(OPENH264_SEARCH_PATHS.iter().map(std::path::PathBuf::from));
    candidates.extend(versioned_openh264_candidates());
    candidates
}

/// The outcome of the library search, cached so the candidate walk does not run
/// for every RDP connection.
///
/// `Some(Some(path))` is the library that loaded; `Some(None)` means a completed
/// walk found nothing usable; the outer `None` means "not probed yet". The
/// decoder itself cannot be shared — each session needs its own — but the search
/// result can, and so can the explanation. Without this the whole walk ran again
/// for every connection: re-`stat`ing every candidate, re-`dlopen`ing each one,
/// and re-emitting the same warnings — nine identical warning lines from three
/// connections about an unchangeable property of the machine.
///
/// It is a resettable `RwLock` rather than a one-shot `OnceLock` so that
/// [`invalidate_openh264_cache`] can clear it after the user downloads Cisco's
/// blob through the in-app action: the **next** RDP connection then re-probes
/// and finds the freshly cached library, so H.264 works without restarting the
/// app. Already-open sessions keep their negotiated RemoteFX path — only new
/// connections pick up the codec — which is why a reconnect, not a restart, is
/// all that is needed.
/// The three states of the OpenH264 search cache, distinguished so a completed
/// "found nothing" is not re-probed every connection while an invalidation
/// still forces a fresh walk.
enum ProbeState {
    /// Not probed yet (startup, or just invalidated) — the next access probes.
    Unprobed,
    /// A completed probe; `Some(path)` loaded, `None` found nothing usable.
    Probed(Option<std::path::PathBuf>),
}

static USABLE_LIBRARY: std::sync::RwLock<ProbeState> = std::sync::RwLock::new(ProbeState::Unprobed);

/// Set when the user turns OpenH264 off in Settings → Connection → Media
/// Codecs. [`openh264_candidates`] then offers no cached/system candidate (an
/// explicit `RUSTCONN_OPENH264` still wins), so H.264 is disabled without the
/// blob being deleted — the enable/disable/re-enable path the OpenH264 binary
/// licence requires. Defaults to enabled (`false`).
static OPENH264_DISABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Enable or disable use of the downloaded OpenH264 codec at runtime.
///
/// Called by the GUI from the stored `ConnectionSettings::use_openh264` at
/// startup and whenever the Media Codecs switch changes. Invalidates the probe
/// cache so the next RDP connection re-evaluates — turning the codec on or off
/// takes effect on the next connection, no restart.
pub fn set_openh264_enabled(enabled: bool) {
    let was_disabled = OPENH264_DISABLED.swap(!enabled, std::sync::atomic::Ordering::Relaxed);
    if was_disabled == enabled {
        // State actually changed; re-probe on the next connection.
        invalidate_openh264_cache();
    }
}

/// Clears the cached OpenH264 search result so the next connection re-probes.
///
/// Called after the in-app downloader installs Cisco's blob, so a user who
/// enables H.264 can simply open a new RDP session rather than restart the app.
/// Safe to call at any time; the worst case is one extra candidate walk.
pub fn invalidate_openh264_cache() {
    if let Ok(mut guard) = USABLE_LIBRARY.write() {
        *guard = ProbeState::Unprobed;
    }
    // The reason flags describe the last probe; a fresh probe recomputes them,
    // so clear them here or a stale RejectedNonCisco / LoadFailed could outlive
    // the download that fixed it.
    REJECTED_NON_CISCO.store(false, std::sync::atomic::Ordering::Relaxed);
    FOUND_BUT_LOAD_FAILED.store(false, std::sync::atomic::Ordering::Relaxed);
}

/// Returns the cached probe result, running the probe once if not yet done.
///
/// A fast read-locked hit on the common path, falling back to a write-locked
/// probe when the cache is `Unprobed` (after startup or an invalidation).
fn usable_library() -> Option<std::path::PathBuf> {
    if let Ok(guard) = USABLE_LIBRARY.read()
        && let ProbeState::Probed(cached) = &*guard
    {
        return cached.clone();
    }
    // Unprobed (or just invalidated): take the write lock and probe.
    let mut guard = USABLE_LIBRARY.write().ok()?;
    // Another thread may have probed between the read unlock and this write lock.
    if let ProbeState::Probed(cached) = &*guard {
        return cached.clone();
    }
    let result = probe_openh264();
    *guard = ProbeState::Probed(result.clone());
    result
}

/// Attempts to load OpenH264 at runtime via dlopen.
///
/// The search itself happens once per process and is cached in
/// [`USABLE_LIBRARY`]; this returns a fresh decoder built from whatever that
/// search settled on.
///
/// Searches [`OPENH264_PATH_ENV`] and the well-known system paths, returning a
/// decoder suitable for passing to
/// [`GraphicsPipelineClient::new`](ironrdp_egfx::client::GraphicsPipelineClient::new).
///
/// # Why a library that is installed still gets rejected
///
/// `ironrdp-egfx` loads through `openh264::OpenH264API::from_blob_path`, which
/// compares the file's SHA-256 against a list of **Cisco's own published
/// binaries** and refuses anything else with `Invalid hash: <sha>`. That is
/// deliberate, not a bug: Cisco pays the H.264 patent royalties for the binaries
/// it distributes itself, which is why the crate's own documentation says to
/// download the library from Cisco. No distribution build can be on that list —
/// Debian's `libopenh264-8`, Fedora's `libopenh264`, and a local build from the
/// Cisco *source* tarball are all refused — and the unchecked loader is `unsafe`
/// and not re-exported, so there is nothing to opt into.
///
/// The practical consequence is that on a packaged Linux install H.264 requires
/// a blob from `ciscobinary.openh264.org` and [`OPENH264_PATH_ENV`] pointing at
/// it. A rejected hash is therefore reported as the configuration problem it is,
/// rather than as an opaque load failure.
///
/// Returns `None` when nothing loadable is found. The session then uses the
/// RemoteFX path; see the EGFX registration in `client::connection` for why it
/// does not open a GFX channel it cannot paint through (issue #262).
///
/// # Errors
///
/// This function does not return an error — it logs warnings and returns
/// `None` on failure, allowing graceful fallback.
#[must_use]
pub fn try_load_openh264() -> Option<Box<dyn H264Decoder>> {
    use ironrdp_egfx::decode::OpenH264Decoder;

    let path = usable_library()?;

    match OpenH264Decoder::from_library_path(&path) {
        Ok(decoder) => Some(Box::new(decoder)),
        Err(e) => {
            // The probe already loaded this exact file, so a failure here is a
            // second session failing where the first succeeded — worth a warning
            // rather than a silent fallback.
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "OpenH264 loaded during detection but not for this session"
            );
            None
        }
    }
}

/// Walks the candidates once and reports which one is usable, if any.
fn probe_openh264() -> Option<std::path::PathBuf> {
    use ironrdp_egfx::decode::OpenH264Decoder;

    // Each probe recomputes the reason, so start from a clean slate (a prior
    // probe, or a stale value an invalidation did not clear).
    REJECTED_NON_CISCO.store(false, std::sync::atomic::Ordering::Relaxed);
    FOUND_BUT_LOAD_FAILED.store(false, std::sync::atomic::Ordering::Relaxed);

    let mut rejected_hash = false;
    let mut load_failed = false;

    for path in openh264_candidates() {
        let path = path.as_path();
        if !path.exists() {
            continue;
        }

        match OpenH264Decoder::from_library_path(path) {
            Ok(_) => {
                tracing::info!(
                    path = %path.display(),
                    "OpenH264 loaded — H.264 decoding enabled"
                );
                return Some(path.to_path_buf());
            }
            Err(e) => {
                // `Invalid hash` is the signature of the check described above:
                // the file loaded fine, it is simply not one of Cisco's.
                let message = e.to_string();
                if message.contains("Invalid hash") {
                    rejected_hash = true;
                    tracing::warn!(
                        path = %path.display(),
                        reason = "openh264_not_cisco_build",
                        "OpenH264 at this path is not one of Cisco's published binaries, so the \
                         loader refuses it — this is expected for a distribution package. Enable \
                         H.264 from Settings → Connection → Media Codecs → “Download H.264 codec \
                         from Cisco”, or point {} at a library downloaded from \
                         ciscobinary.openh264.org.",
                        OPENH264_PATH_ENV
                    );
                } else {
                    load_failed = true;
                    tracing::warn!(
                        path = %path.display(),
                        error = %message,
                        "OpenH264 found but failed to initialize"
                    );
                }
            }
        }
    }

    if rejected_hash {
        REJECTED_NON_CISCO.store(true, std::sync::atomic::Ordering::Relaxed);
        tracing::warn!(
            reason = "openh264_not_cisco_build",
            "No usable OpenH264 — every library found was a non-Cisco build. GFX pipeline will \
             use non-AVC codecs; enable H.264 from Settings → Connection → Media Codecs (the \
             Download button) (or set {}).",
            OPENH264_PATH_ENV
        );
    } else if load_failed {
        FOUND_BUT_LOAD_FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
        tracing::warn!(
            reason = "openh264_load_failed",
            "OpenH264 found and accepted by the hash check, but the decoder could not initialize \
             from it — the file may be corrupt or incompatible. Re-download it from Settings → \
             Connection → Media Codecs, or replace the library {} points at.",
            OPENH264_PATH_ENV
        );
    } else {
        tracing::warn!(
            reason = "openh264_unavailable",
            "OpenH264 not found — GFX pipeline will use non-AVC codecs"
        );
    }
    None
}

/// Set by [`probe_openh264`] when every OpenH264 it found was a non-Cisco build
/// rejected by the hash check, as opposed to none being found at all. Read via
/// [`openh264_unavailable_reason`] to tell the user which case they are in.
static REJECTED_NON_CISCO: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Set by [`probe_openh264`] when an OpenH264 passed the Cisco hash check but
/// the decoder still could not initialize from it (corrupt file, ABI mismatch).
/// Distinct from both "not found" and "non-Cisco build". Read via
/// [`openh264_unavailable_reason`].
static FOUND_BUT_LOAD_FAILED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Returns why H.264 is unavailable, or `None` when a decoder DID load.
///
/// Runs the one-shot probe (shared with [`try_load_openh264`]) so it reflects
/// the same decision the session made, without re-walking the filesystem.
#[must_use]
pub fn openh264_unavailable_reason() -> Option<super::graphics::H264UnavailableReason> {
    use super::graphics::H264UnavailableReason;
    if usable_library().is_some() {
        return None;
    }
    Some(
        if REJECTED_NON_CISCO.load(std::sync::atomic::Ordering::Relaxed) {
            H264UnavailableReason::RejectedNonCisco
        } else if FOUND_BUT_LOAD_FAILED.load(std::sync::atomic::Ordering::Relaxed) {
            H264UnavailableReason::LoadFailed
        } else {
            H264UnavailableReason::NotFound
        },
    )
}

// ============================================================================
// Error types
// ============================================================================

/// Errors specific to the GFX/H.264 pipeline.
#[derive(Debug, thiserror::Error)]
pub enum GfxError {
    /// OpenH264 library not available at runtime.
    #[error("OpenH264 library not available: {0}")]
    H264Unavailable(String),

    /// Single-frame H.264 decode failure.
    #[error("H.264 decode failed for surface {surface_id}: {reason}")]
    H264DecodeFailed {
        /// Surface that failed to decode
        surface_id: u16,
        /// Human-readable failure reason
        reason: String,
    },

    /// Bitmap update references an unmapped surface.
    #[error("Surface {surface_id} not mapped to output")]
    SurfaceNotMapped {
        /// The unmapped surface ID
        surface_id: u16,
    },

    /// Too many consecutive empty frames indicate a persistent problem.
    #[error("Persistent decode failure: {consecutive_failures} consecutive empty frames")]
    PersistentDecodeFailure {
        /// Number of consecutive failures observed
        consecutive_failures: u32,
    },
}

#[cfg(test)]
mod tests {
    use ironrdp_egfx::pdu::{Codec1Type, PixelFormat, WireToSurface1Pdu};
    use ironrdp_pdu::geometry::ExclusiveRectangle;

    use super::*;

    #[test]
    fn invalidate_resets_the_probe_cache() {
        // Prime the cache with a probe, then invalidate and confirm the slot is
        // empty again so the NEXT caller re-probes (this is what lets a freshly
        // downloaded blob be picked up without an app restart). The probe result
        // itself is machine-dependent, so this asserts the cache STATE, not which
        // library was found.
        let _ = usable_library();
        assert!(
            matches!(&*USABLE_LIBRARY.read().unwrap(), ProbeState::Probed(_)),
            "a completed probe must populate the cache"
        );
        invalidate_openh264_cache();
        assert!(
            matches!(&*USABLE_LIBRARY.read().unwrap(), ProbeState::Unprobed),
            "invalidation must clear the cache so the next connection re-probes"
        );
        // Re-probing repopulates it (idempotent, no panic).
        let _ = usable_library();
        assert!(matches!(
            &*USABLE_LIBRARY.read().unwrap(),
            ProbeState::Probed(_)
        ));
    }

    #[test]
    fn gfx_error_display() {
        let err = GfxError::H264Unavailable("library not found".into());
        assert!(err.to_string().contains("not available"));

        let err = GfxError::H264DecodeFailed {
            surface_id: 5,
            reason: "corrupted NAL".into(),
        };
        assert!(err.to_string().contains("surface 5"));

        let err = GfxError::SurfaceNotMapped { surface_id: 3 };
        assert!(err.to_string().contains("Surface 3"));

        let err = GfxError::PersistentDecodeFailure {
            consecutive_failures: 10,
        };
        assert!(err.to_string().contains("10"));
    }

    fn test_handler() -> (RustConnGfxHandler, mpsc::Receiver<RdpClientEvent>) {
        let (event_tx, event_rx) = mpsc::channel();
        (RustConnGfxHandler::new(event_tx), event_rx)
    }

    #[test]
    fn capabilities_confirm_reports_the_negotiated_mode() {
        let cases = [
            (
                CapabilitySet::V10_7 {
                    flags: CapabilitiesV107Flags::SMALL_CACHE,
                },
                GraphicsMode::GfxAvc444,
            ),
            (
                CapabilitySet::V10_7 {
                    flags: CapabilitiesV107Flags::AVC_DISABLED,
                },
                GraphicsMode::Gfx,
            ),
            (CapabilitySet::V10_1, GraphicsMode::GfxAvc444),
            (
                CapabilitySet::V8_1 {
                    flags: CapabilitiesV81Flags::AVC420_ENABLED | CapabilitiesV81Flags::SMALL_CACHE,
                },
                GraphicsMode::GfxH264,
            ),
            // A server that drops AVC must not be reported as an H.264 session.
            (
                CapabilitySet::V8 {
                    flags: ironrdp_egfx::pdu::CapabilitiesV8Flags::SMALL_CACHE,
                },
                GraphicsMode::Gfx,
            ),
        ];
        for (caps, expected) in cases {
            let (mut handler, event_rx) = test_handler();
            handler.on_capabilities_confirmed(&caps);
            match event_rx.try_recv() {
                Ok(RdpClientEvent::GraphicsModeActive { mode }) => {
                    assert_eq!(mode, expected, "{caps:?}");
                }
                other => panic!("expected GraphicsModeActive for {caps:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn reset_graphics_tells_the_gui_the_new_size() {
        let (mut handler, event_rx) = test_handler();
        handler.consecutive_empty = 7;
        handler.on_reset_graphics(1920, 1200);
        assert_eq!(handler.consecutive_empty, 0);
        assert!(matches!(
            event_rx.try_recv(),
            Ok(RdpClientEvent::ResolutionChanged {
                width: 1920,
                height: 1200
            })
        ));
    }

    /// The unsupported-codec path is the actual issue #262 failure: content
    /// arrives, cannot be decoded, and must be reported rather than dropped.
    #[test]
    fn unsupported_codec_reports_once_past_the_threshold() {
        let (mut handler, event_rx) = test_handler();
        let pdu = GfxPdu::WireToSurface1(WireToSurface1Pdu {
            surface_id: 1,
            codec_id: Codec1Type::Avc444v2,
            pixel_format: PixelFormat::XRgb,
            destination_rectangle: ExclusiveRectangle {
                left: 0,
                top: 0,
                right: 16,
                bottom: 16,
            },
            bitmap_data: vec![0; 8],
        });

        for _ in 0..UNSUPPORTED_CODEC_THRESHOLD - 1 {
            handler.on_unhandled_pdu(&pdu);
            assert!(event_rx.try_recv().is_err(), "must not fire early");
        }

        handler.on_unhandled_pdu(&pdu);
        match event_rx.try_recv().expect("threshold must report") {
            RdpClientEvent::GfxUnsupportedCodec {
                codec,
                dropped_frames,
            } => {
                assert!(codec.contains("Avc444"), "codec was: {codec}");
                assert_eq!(dropped_frames, UNSUPPORTED_CODEC_THRESHOLD);
            }
            other => panic!("expected GfxUnsupportedCodec, got: {other:?}"),
        }

        // Reported once, not on every subsequent frame.
        handler.on_unhandled_pdu(&pdu);
        assert!(event_rx.try_recv().is_err(), "must report only once");
    }

    #[test]
    fn handler_activation() {
        let (mut handler, _event_rx) = test_handler();
        assert!(!handler.is_active());

        handler.on_capabilities_confirmed(&CapabilitySet::V8 {
            flags: ironrdp_egfx::pdu::CapabilitiesV8Flags::SMALL_CACHE,
        });
        assert!(handler.is_active());

        handler.on_close();
        assert!(!handler.is_active());
    }

    /// A run of empty bitmap updates reports a persistent decode failure once.
    ///
    /// `BitmapUpdate` is `#[non_exhaustive]` and cannot be built here, so the
    /// counter behind `on_bitmap_updated` is driven directly.
    #[test]
    fn handler_sends_decode_failure_event_once() {
        let (mut handler, event_rx) = test_handler();

        for _ in 0..EMPTY_UPDATE_THRESHOLD - 1 {
            handler.note_empty_update(1);
        }
        assert!(event_rx.try_recv().is_err(), "No event before threshold");

        handler.note_empty_update(1);
        assert!(matches!(
            event_rx.try_recv(),
            Ok(RdpClientEvent::GfxDecodeFailure {
                consecutive_failures: EMPTY_UPDATE_THRESHOLD
            })
        ));

        handler.note_empty_update(1);
        assert!(event_rx.try_recv().is_err(), "reported only once per run");
    }

    /// Verifies that all [`GfxError`] variants produce meaningful Display output
    /// suitable for structured logging and user-facing error messages.
    ///
    /// Covers: Req 10 AC 1, AC 5
    #[test]
    fn gfx_error_variants_display_coverage() {
        // Each variant produces a non-empty, descriptive string
        let variants: Vec<(GfxError, &str)> = vec![
            (
                GfxError::H264Unavailable("libpath not found".into()),
                "not available",
            ),
            (
                GfxError::H264DecodeFailed {
                    surface_id: 42,
                    reason: "invalid NAL unit".into(),
                },
                "surface 42",
            ),
            (GfxError::SurfaceNotMapped { surface_id: 7 }, "Surface 7"),
            (
                GfxError::PersistentDecodeFailure {
                    consecutive_failures: 15,
                },
                "15",
            ),
        ];

        for (err, expected_substr) in variants {
            let display = err.to_string();
            assert!(
                !display.is_empty(),
                "GfxError Display should not be empty: {err:?}"
            );
            assert!(
                display.contains(expected_substr),
                "Expected '{expected_substr}' in '{display}'"
            );
        }
    }

    #[test]
    fn soname_version_accepts_only_numeric_segments() {
        assert_eq!(soname_version("libopenh264.so.8"), Some(vec![8]));
        assert_eq!(soname_version("libopenh264.so.2.6.0"), Some(vec![2, 6, 0]));

        // The unversioned name is handled by OPENH264_SEARCH_PATHS, not here.
        assert_eq!(soname_version("libopenh264.so"), None);
        assert_eq!(soname_version("libopenh264.so."), None);
        assert_eq!(soname_version("libopenh264.so.debug"), None);
        assert_eq!(soname_version("libopenh264.so.8.debug"), None);
        assert_eq!(soname_version("libavcodec.so.60"), None);
    }

    #[test]
    fn sonames_sort_newest_abi_first() {
        // Deliberately not in the answer's order, and not in an order a string
        // sort would fix: "10" < "9" lexicographically.
        let names = vec![
            "libopenh264.so.2.6.0".to_string(),
            "libopenh264.so.9".to_string(),
            "libopenh264.so.10".to_string(),
            "libopenh264.so.7".to_string(),
        ];

        assert_eq!(
            sonames_newest_first(names),
            vec![
                "libopenh264.so.10",
                "libopenh264.so.9",
                "libopenh264.so.7",
                "libopenh264.so.2.6.0",
            ]
        );
    }

    #[test]
    fn bare_soname_wins_over_the_file_it_points_at() {
        let names = vec![
            "libopenh264.so.8.0.1".to_string(),
            "libopenh264.so.8".to_string(),
        ];

        assert_eq!(
            sonames_newest_first(names),
            vec!["libopenh264.so.8", "libopenh264.so.8.0.1"]
        );
    }

    #[test]
    fn sonames_drop_unrelated_entries() {
        let names = vec![
            "libopenh264.so".to_string(),
            "libopenh264.so.8".to_string(),
            "libx264.so.164".to_string(),
            "pkgconfig".to_string(),
        ];

        assert_eq!(sonames_newest_first(names), vec!["libopenh264.so.8"]);
    }

    /// The Debian layout that made H.264 unreachable before the scan existed:
    /// a runtime-only install carries the soname symlink and the real file, and
    /// never the unversioned name the old candidate list looked for.
    #[test]
    fn debian_runtime_only_layout_resolves() {
        let names = vec![
            "libopenh264.so.2.6.0".to_string(),
            "libopenh264.so.8".to_string(),
        ];

        let ordered = sonames_newest_first(names);
        assert_eq!(
            ordered.first().map(String::as_str),
            Some("libopenh264.so.8")
        );
        assert_eq!(ordered.len(), 2, "the real file stays as a fallback");
    }
}
