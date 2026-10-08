//! Opt-in, explicit-consent downloader for Cisco's official OpenH264 binary.
//!
//! # Why this exists
//!
//! `ironrdp-egfx` loads H.264 through `openh264`'s
//! `OpenH264Decoder::from_library_path`, which SHA-256-checks the file against a
//! built-in allow-list of **Cisco's own published binaries** and refuses
//! anything else with `Invalid hash`. No Linux distribution ships one of those
//! binaries — Debian's `libopenh264-8`, Fedora's `libopenh264`, and a local
//! build from the Cisco *source* tarball are all built from source and rejected
//! — so on a packaged install there is nowhere to point
//! [`OPENH264_PATH_ENV`](super::gfx_handler) at.
//!
//! Cisco pays the MPEG-LA H.264 patent royalties, but that coverage applies
//! **only to the blobs an end user downloads on demand from Cisco's CDN**, not
//! to blobs redistributed inside another package. Firefox solves this the same
//! way: it ships no codec and fetches Cisco's binary at the user's request. This
//! module is RustConn's version of that — an explicit, user-initiated download
//! into the user's cache, which the existing loader then accepts because its
//! SHA-256 matches Cisco's published value.
//!
//! # Guarantees
//!
//! * The download **never** runs without an explicit `consent == true`.
//! * It is **never** triggered at startup or during the loader probe. The probe
//!   only ever *reads* the cache ([`cached_openh264_path`]).
//! * The decompressed bytes are SHA-256-verified against the pinned per-platform
//!   hash before anything is written to the final path, and the write is atomic
//!   (temp file + rename), so a failed or interrupted download leaves no partial
//!   or corrupt file behind.

/// Clears the GFX loader's cached OpenH264 probe so the next RDP connection
/// re-scans and finds a freshly downloaded blob — no app restart needed.
///
/// Call this right after [`download_openh264`] succeeds. It is a thin, always-
/// compiled front for `gfx_handler::invalidate_openh264_cache`: when the
/// `gfx-h264` feature is off there is no loader cache to clear, so it is a
/// no-op, which lets the GUI call one coherent module regardless of features.
pub fn invalidate_loader_cache() {
    #[cfg(feature = "gfx-h264")]
    super::gfx_handler::invalidate_openh264_cache();
}

/// Enable or disable use of the downloaded OpenH264 codec, an always-compiled
/// front for `gfx_handler::set_openh264_enabled`.
///
/// Called by the GUI from `ConnectionSettings::use_openh264` at startup and on
/// the Media Codecs switch. A no-op when the `gfx-h264` feature is off (there
/// is no loader to gate), so the GUI can call one coherent module either way.
pub fn set_openh264_enabled(enabled: bool) {
    #[cfg(feature = "gfx-h264")]
    super::gfx_handler::set_openh264_enabled(enabled);
    #[cfg(not(feature = "gfx-h264"))]
    let _ = enabled;
}

use std::path::{Path, PathBuf};

/// A platform's Cisco OpenH264 artifact: the CDN file name and the SHA-256 of
/// the **decompressed** `.so`/`.dylib` that the loader's allow-list expects.
///
/// The hash is of the raw library, not the `.bz2` — the CDN serves
/// `<file>.bz2`, we bunzip2 it, and the result must hash to `sha256`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenH264Artifact {
    /// Decompressed library file name, e.g. `libopenh264-2.6.0-linux64.8.so`.
    /// Also the CDN path once `.bz2` is appended.
    pub file: &'static str,
    /// SHA-256 (hex) of the decompressed library — must match Cisco's published
    /// value and the `openh264` crate's built-in allow-list for v2.6.0.
    pub sha256: &'static str,
}

/// OpenH264 version pinned by this build. Must match the `openh264-sys2`
/// allow-list version (0.9.8 → v2.6.0) and the artifacts in [`artifact`].
pub const OPENH264_VERSION: &str = "2.6.0";

/// Cisco CDN base URL. Each artifact is served bzip2-compressed as
/// `<base>/<file>.bz2`. HTTPS: the pinned SHA-256 already rejects tampered
/// bytes, but TLS keeps the request and redirect chain private and unmodified.
const CISCO_CDN_BASE: &str = "https://ciscobinary.openh264.org";

/// Hard ceiling on the compressed `.bz2` we will buffer from the CDN. Cisco's
/// blobs are ~700 KiB; 8 MiB is generous headroom and caps a hostile or broken
/// endpoint streaming an unbounded body into memory.
const MAX_COMPRESSED_BYTES: u64 = 8 * 1024 * 1024;

/// Hard ceiling on the decompressed library. The real libraries are ~2 MiB;
/// 16 MiB caps a bzip2 bomb before it exhausts memory.
const MAX_DECOMPRESSED_BYTES: u64 = 16 * 1024 * 1024;

/// Network timeout for the CDN fetch, matching `cli_download`'s own
/// `HTTP_DOWNLOAD_TIMEOUT` (15 s) — kept as a local constant because that one is
/// `pub(crate)` to the `cli_download` module.
const DOWNLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Returns the Cisco OpenH264 artifact for the platform this binary was built
/// for, or `None` on an unsupported `(os, arch)` combination.
///
/// Data-driven via `cfg!` so the table is a compile-time constant for the
/// current target and the "unsupported platform" branch is explicit rather than
/// a silent fallthrough.
#[must_use]
pub fn artifact() -> Option<OpenH264Artifact> {
    // x86_64 Linux
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    let artifact = Some(OpenH264Artifact {
        file: "libopenh264-2.6.0-linux64.8.so",
        sha256: "2f0cde7c6a6abcf5cae76942894ea42897fa677bce4ed6c91a24dd1b041d5f04",
    });
    // aarch64 Linux
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    let artifact = Some(OpenH264Artifact {
        file: "libopenh264-2.6.0-linux-arm64.8.so",
        sha256: "12e7b33623667cdab0e575170c147b1b36eadb77d0d2aa7ceb5afd3e58902140",
    });
    // armv7 Linux (hard-float). SHA matches the openh264-sys2 allow-list entry
    // for `libopenh264-2.6.0-linux-arm.8.so`.
    #[cfg(all(target_os = "linux", target_arch = "arm"))]
    let artifact = Some(OpenH264Artifact {
        file: "libopenh264-2.6.0-linux-arm.8.so",
        sha256: "df91866de0e93773019e30a8f2bdee8b15de4abe2bf89a228ae9f064ff1e85bb",
    });
    // aarch64 macOS
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    let artifact = Some(OpenH264Artifact {
        file: "libopenh264-2.6.0-mac-arm64.dylib",
        sha256: "052e98bfcf7a9167d22f3bbb3f5988ef79065591f36af8b52924b22b13624551",
    });
    // x86_64 macOS
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    let artifact = Some(OpenH264Artifact {
        file: "libopenh264-2.6.0-mac-x64.dylib",
        sha256: "e3dc8bc01fe69363f61fd3c02fd27798537a585eadd38cd808f303d1ee505a19",
    });
    // Any other platform: no Cisco binary we can pin.
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "arm"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
    )))]
    let artifact = None;

    artifact
}

/// Errors from the OpenH264 download path.
///
/// Deliberately a small, self-contained enum (not the `cli_download` error
/// type): this module is compiled into the minimal `rustconn-core` the CLI uses
/// with `default-features = false`, and must not drag the CLI-download surface
/// in with it.
#[derive(Debug, thiserror::Error)]
pub enum OpenH264DownloadError {
    /// Download attempted without the caller passing `consent == true`.
    #[error("refusing to download the Cisco OpenH264 binary without explicit consent")]
    ConsentRequired,

    /// No pinned Cisco artifact for this OS/arch.
    #[error("no Cisco OpenH264 binary is published for this platform (os/arch unsupported)")]
    UnsupportedPlatform,

    /// The OS gave us no cache directory to write into.
    #[error("could not determine a cache directory for the OpenH264 library")]
    NoCacheDir,

    /// Network / HTTP failure fetching the `.bz2`.
    #[error("download failed: {0}")]
    Download(String),

    /// bunzip2 of the downloaded stream failed.
    #[error("decompression failed: {0}")]
    Decompress(String),

    /// Decompressed bytes did not match the pinned SHA-256. The on-disk result
    /// would be rejected by the loader, so this is a hard failure.
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Pinned SHA-256 for this platform.
        expected: String,
        /// SHA-256 actually computed over the decompressed bytes.
        actual: String,
    },

    /// Filesystem error creating the cache dir or writing the file.
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
}

/// Directory the cached library lives in: `<data_dir>/rustconn/openh264`.
///
/// Uses `dirs::data_dir()` rather than `cache_dir()`: the blob is a
/// user-installed, licence-gated artifact that must survive a cache sweep —
/// a `~/.cache` cleaner wiping it would silently drop H.264 back to the
/// software path. Honours `$XDG_DATA_HOME` and the per-OS convention.
fn cache_subdir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("rustconn").join("openh264"))
}

/// Full install path for the current platform's artifact,
/// `<data_dir>/rustconn/openh264/<file>`.
#[must_use]
pub fn cache_path() -> Option<PathBuf> {
    Some(cache_subdir()?.join(artifact()?.file))
}

/// Computes the SHA-256 (hex) of `bytes`, reusing the crate's `ring` dependency
/// — the same primitive `cli_download::download::verify_checksum` uses.
fn sha256_hex(bytes: &[u8]) -> String {
    use ring::digest::{Context, SHA256};
    let mut ctx = Context::new(&SHA256);
    ctx.update(bytes);
    hex::encode(ctx.finish().as_ref())
}

/// Verifies `bytes` against the platform's pinned hash.
fn verify_against_platform(bytes: &[u8]) -> Result<(), OpenH264DownloadError> {
    let expected = artifact()
        .ok_or(OpenH264DownloadError::UnsupportedPlatform)?
        .sha256;
    let actual = sha256_hex(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(OpenH264DownloadError::ChecksumMismatch {
            expected: expected.to_string(),
            actual,
        })
    }
}

/// Returns the cached library path when it is present and valid.
///
/// Yields `Some` **only if** the file exists and its SHA-256 matches the pinned
/// platform hash; a corrupt, partial, or stale file returns `None` rather than a
/// path the loader would then reject.
///
/// This is the only function the loader probe calls, and it is pure read: it
/// never downloads, and never writes.
#[must_use]
pub fn cached_openh264_path() -> Option<PathBuf> {
    cached_openh264_path_at(&cache_path()?)
}

/// Validate a specific candidate path against the platform's pinned hash.
///
/// Factored out of [`cached_openh264_path`] so the acceptance behaviour (a
/// wrong-hash file is rejected) can be tested against a caller-supplied path
/// without mutating process-wide environment variables — the crate forbids
/// `unsafe`, and `std::env::set_var` is `unsafe`.
fn cached_openh264_path_at(path: &Path) -> Option<PathBuf> {
    let bytes = std::fs::read(path).ok()?;
    verify_against_platform(&bytes).ok()?;
    Some(path.to_path_buf())
}

/// bunzip2 a `.bz2` byte stream into the decompressed library bytes.
///
/// Uses `bzip2::read::BzDecoder` (bzip2 0.6, pure-Rust `libbz2-rs-sys` backend —
/// no C/system dependency). The crate is already in the tree transitively via
/// `zip`; this module pins it as a direct dependency.
fn bunzip2(compressed: &[u8]) -> Result<Vec<u8>, OpenH264DownloadError> {
    use std::io::Read;
    let decoder = bzip2::read::BzDecoder::new(compressed);
    // Cap the decompressed size: a crafted .bz2 could otherwise expand without
    // bound. One byte over the ceiling means the stream is longer than any real
    // library, so refuse it rather than truncate to a file the loader rejects.
    let mut limited = decoder.take(MAX_DECOMPRESSED_BYTES + 1);
    let mut out = Vec::new();
    limited
        .read_to_end(&mut out)
        .map_err(|e| OpenH264DownloadError::Decompress(e.to_string()))?;
    if out.len() as u64 > MAX_DECOMPRESSED_BYTES {
        return Err(OpenH264DownloadError::Decompress(format!(
            "decompressed stream exceeded the {MAX_DECOMPRESSED_BYTES}-byte ceiling"
        )));
    }
    Ok(out)
}

/// Atomically write `bytes` to `dest` via a sibling temp file + rename, so a
/// crash mid-write never leaves a half-written library at `dest`.
///
/// Mirrors the temp-then-rename pattern used elsewhere in the crate. The temp
/// file is removed on any failure before the rename, so no partial file leaks.
fn atomic_write(dest: &Path, bytes: &[u8]) -> Result<(), OpenH264DownloadError> {
    use std::io::Write;

    let parent = dest.parent().ok_or(OpenH264DownloadError::NoCacheDir)?;
    std::fs::create_dir_all(parent)?;

    // Unique-enough temp name in the same directory (same filesystem → rename is
    // atomic). PID + nanos avoids clobbering a concurrent download's temp file.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = parent.join(format!(
        ".{}.{}.{}.tmp",
        dest.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("openh264"),
        std::process::id(),
        nanos
    ));

    let write_result = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        Ok(())
    })();

    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(OpenH264DownloadError::Io(e));
    }

    if let Err(e) = std::fs::rename(&tmp, dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(OpenH264DownloadError::Io(e));
    }

    Ok(())
}

/// Download URL for the current platform's artifact, `<base>/<file>.bz2`.
#[must_use]
pub fn download_url() -> Option<String> {
    Some(format!("{CISCO_CDN_BASE}/{}.bz2", artifact()?.file))
}

/// Delete the installed OpenH264 blob, if present.
///
/// For the Media Codecs "Remove" action. Returns `Ok(true)` when a file was
/// removed, `Ok(false)` when there was nothing to remove. The loader's probe
/// cache is the caller's responsibility to invalidate (the GUI calls
/// [`invalidate_loader_cache`] after this). A missing file is not an error.
///
/// # Errors
///
/// Returns [`OpenH264DownloadError::Io`] if the file exists but cannot be
/// deleted (permissions, a filesystem error).
pub fn remove_openh264() -> Result<bool, OpenH264DownloadError> {
    let Some(path) = cache_path() else {
        return Ok(false);
    };
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(OpenH264DownloadError::Io(e)),
    }
}

/// Fetch the `.bz2` from the Cisco CDN, reusing the same `reqwest` client
/// configuration (`reqwest`, limited redirects) that
/// `cli_download::download::download_with_progress` uses.
async fn fetch_bz2(url: &str) -> Result<Vec<u8>, OpenH264DownloadError> {
    use futures::StreamExt;

    let client = reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| OpenH264DownloadError::Download(e.to_string()))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| OpenH264DownloadError::Download(e.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        return Err(OpenH264DownloadError::Download(format!(
            "HTTP {} - {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Unknown error")
        )));
    }

    // A declared Content-Length over the ceiling is rejected before any body is
    // read; the running total below still guards a chunked response that lies.
    if let Some(len) = response.content_length()
        && len > MAX_COMPRESSED_BYTES
    {
        return Err(OpenH264DownloadError::Download(format!(
            "refusing a {len}-byte download; the Cisco OpenH264 blob is well under {MAX_COMPRESSED_BYTES} bytes"
        )));
    }

    let mut stream = response.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| OpenH264DownloadError::Download(e.to_string()))?;
        if buf.len() as u64 + chunk.len() as u64 > MAX_COMPRESSED_BYTES {
            return Err(OpenH264DownloadError::Download(format!(
                "download exceeded the {MAX_COMPRESSED_BYTES}-byte ceiling"
            )));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Download, decompress, verify, and atomically install Cisco's OpenH264 binary
/// into the user cache, returning the path the loader can be pointed at.
///
/// # Consent
///
/// This is the user-initiated, legally-required path. It refuses with
/// [`OpenH264DownloadError::ConsentRequired`] unless `consent == true`; callers
/// MUST only pass `true` in direct response to an explicit user action (a GUI
/// confirm dialog, a CLI `--accept-cisco-license` flag). It must never be called
/// at startup, during the loader probe, or implicitly.
///
/// # Behaviour
///
/// 1. Refuse without consent.
/// 2. Resolve the platform artifact (error on unsupported platform).
/// 3. If a valid cached copy already exists, return it without re-downloading.
/// 4. GET `<file>.bz2` from the Cisco CDN.
/// 5. bunzip2 the stream.
/// 6. SHA-256-verify the decompressed bytes against the pinned platform hash —
///    a mismatch is a hard failure and nothing is written.
/// 7. Atomically write the verified bytes to the cache path.
///
/// # Errors
///
/// See [`OpenH264DownloadError`].
pub async fn download_openh264(consent: bool) -> Result<PathBuf, OpenH264DownloadError> {
    if !consent {
        return Err(OpenH264DownloadError::ConsentRequired);
    }

    // Resolve platform first so an unsupported target fails before any network.
    let _artifact = artifact().ok_or(OpenH264DownloadError::UnsupportedPlatform)?;
    let dest = cache_path().ok_or(OpenH264DownloadError::NoCacheDir)?;

    // Already installed and valid? Don't re-fetch.
    if let Some(existing) = cached_openh264_path() {
        return Ok(existing);
    }

    let url = download_url().ok_or(OpenH264DownloadError::UnsupportedPlatform)?;
    let compressed = fetch_bz2(&url).await?;
    let decompressed = bunzip2(&compressed)?;

    // Verify BEFORE writing — never persist bytes the loader would reject.
    verify_against_platform(&decompressed)?;

    atomic_write(&dest, &decompressed)?;

    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SHA-256 of the ASCII bytes "hello" — a known value independent of the
    /// platform table, used to exercise the verify helper directly.
    const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(sha256_hex(b"hello"), HELLO_SHA256);
    }

    #[test]
    fn artifact_is_resolvable_on_supported_platforms() {
        // The CI/dev matrix is exactly the four supported targets, so on any
        // platform this test runs on, the table must resolve AND the hash must
        // be a 64-char lowercase hex string.
        let a = artifact().expect("supported platform must have an artifact");
        assert_eq!(a.sha256.len(), 64, "SHA-256 hex must be 64 chars");
        assert!(
            a.sha256.chars().all(|c| c.is_ascii_hexdigit()),
            "SHA-256 must be hex"
        );
        assert!(a.file.contains(OPENH264_VERSION), "file names the version");
    }

    #[test]
    fn download_url_is_cdn_bz2() {
        let url = download_url().expect("supported platform");
        let a = artifact().expect("supported platform");
        assert!(url.starts_with("https://ciscobinary.openh264.org/"));
        // The CDN serves "<file>.bz2"; assert the exact composed suffix rather
        // than an extension comparison (which trips a pedantic lint).
        let expected_suffix = format!("{}.bz2", a.file);
        assert!(
            url.ends_with(&expected_suffix),
            "url must end with the artifact's .bz2 name"
        );
    }

    #[tokio::test]
    async fn download_refuses_without_consent() {
        // The consent gate must fire BEFORE any network/platform work, so this
        // holds even in a sandbox with no network.
        let err = download_openh264(false).await.unwrap_err();
        assert!(matches!(err, OpenH264DownloadError::ConsentRequired));
    }

    #[test]
    fn verify_rejects_wrong_bytes() {
        // Bytes that are not the pinned library must fail verification — this is
        // the SHA-gate the on-demand download relies on. "hello" is not any
        // OpenH264 build.
        let err = verify_against_platform(b"hello").unwrap_err();
        assert!(matches!(
            err,
            OpenH264DownloadError::ChecksumMismatch { .. }
        ));
    }

    #[test]
    fn cached_path_rejects_wrong_hash_file() {
        // A file whose contents are not the pinned library must be rejected
        // (returns None), because its SHA does not match the platform hash.
        // Uses the path-taking helper so no process env is mutated (the crate
        // forbids `unsafe`, which `std::env::set_var` requires) and the real
        // user cache is never touched.
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("libopenh264-bogus.so");
        std::fs::write(&path, b"not a real openh264 library").unwrap();
        assert!(
            cached_openh264_path_at(&path).is_none(),
            "a wrong-hash file must be rejected"
        );
    }

    #[test]
    fn cached_path_rejects_missing_file() {
        // A non-existent path is simply absent, not an error.
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("does-not-exist.so");
        assert!(cached_openh264_path_at(&path).is_none());
    }

    #[test]
    fn cache_path_lives_under_rustconn_openh264() {
        // The cache path, when resolvable, must sit under <cache>/rustconn/openh264
        // and end in the platform artifact's file name — the layout the loader
        // wiring and the download writer both assume.
        if let (Some(path), Some(a)) = (cache_path(), artifact()) {
            assert!(path.ends_with(a.file));
            let parent = path.parent().unwrap();
            assert_eq!(parent.file_name().unwrap(), "openh264");
            assert_eq!(parent.parent().unwrap().file_name().unwrap(), "rustconn");
        }
    }

    #[test]
    fn atomic_write_leaves_no_tmp_on_success() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dest = tmp.path().join("lib.so");
        atomic_write(&dest, b"content").expect("write");
        assert_eq!(std::fs::read(&dest).unwrap(), b"content");
        // No leftover .tmp sibling.
        let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file must not survive");
    }

    #[test]
    fn bunzip2_roundtrips() {
        use std::io::Write;
        // Compress a known payload, then assert bunzip2 restores it — proves the
        // decoder path and the pure-Rust backend are wired correctly.
        let payload = b"openh264 bunzip2 roundtrip test payload";
        let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
        encoder.write_all(payload).unwrap();
        let compressed = encoder.finish().unwrap();
        assert_eq!(bunzip2(&compressed).unwrap(), payload);
    }
}
