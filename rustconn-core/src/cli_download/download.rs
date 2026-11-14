use super::{
    ChecksumPolicy, CliDownloadError, CliDownloadResult, DownloadCancellation, DownloadProgress,
    ProgressCallback,
};

/// Download file with progress reporting
pub(super) async fn download_with_progress(
    url: &str,
    progress_callback: &ProgressCallback,
    cancel_token: &DownloadCancellation,
) -> CliDownloadResult<Vec<u8>> {
    use futures::StreamExt;

    // HTTPS only, redirects included: a redirect to http:// would let a network
    // attacker swap a download whose checksum is not published (the
    // `SkipLatest` components). The read timeout is per chunk, so a large
    // archive on a slow link still completes while a stalled one fails.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .https_only(true)
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    // Check for HTTP errors
    let status = response.status();
    if !status.is_success() {
        return Err(CliDownloadError::DownloadFailed(format!(
            "HTTP {} - {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Unknown error")
        )));
    }

    let total_size = response.content_length();
    let mut downloaded: u64 = 0;
    let mut data = Vec::with_capacity(total_size.unwrap_or(1_000_000) as usize);

    let mut stream = response.bytes_stream();

    while let Some(chunk_result) = stream.next().await {
        // Check for cancellation
        if cancel_token.is_cancelled() {
            return Err(CliDownloadError::Cancelled);
        }

        let chunk = chunk_result.map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;
        downloaded += chunk.len() as u64;
        data.extend_from_slice(&chunk);

        if let Some(cb) = progress_callback {
            cb(DownloadProgress {
                downloaded,
                total: total_size,
                status: format!("Downloading... {:.1} MB", downloaded as f64 / 1_000_000.0),
            });
        }
    }

    Ok(data)
}

/// Verify SHA256 checksum of downloaded data
pub(super) fn verify_checksum(data: &[u8], expected: &str) -> CliDownloadResult<()> {
    use ring::digest::{Context, SHA256};

    let mut context = Context::new(&SHA256);
    context.update(data);
    let digest = context.finish();
    let actual = hex::encode(digest.as_ref());

    if actual != expected {
        return Err(CliDownloadError::ChecksumMismatch {
            expected: expected.to_string(),
            actual,
        });
    }

    Ok(())
}

/// Enforce a component's [`ChecksumPolicy`] against freshly downloaded bytes.
///
/// This is the single decision point that `install.rs`, `install_cloud.rs` and
/// the custom (`install_custom.rs`) installers all funnel through, so every
/// download path treats checksums identically:
/// - `Static` → verified against the pinned SHA256 (hard failure on mismatch),
/// - `SkipLatest` → allowed, but logged with a `warn!` so the skip is visible
///   in the logs rather than silent (these are "latest" URLs with no stable
///   published hash to pin against),
/// - `None` → refused ([`CliDownloadError::NoChecksum`]).
///
/// The custom installers previously never consulted the policy at all, so the
/// module's "all downloads are verified" claim was false for kubectl, tsh,
/// tailscale, boundary and hoop; routing them here fixes that.
pub(super) fn enforce_checksum_policy(
    policy: ChecksumPolicy,
    component_name: &str,
    bytes: &[u8],
) -> CliDownloadResult<()> {
    match policy {
        ChecksumPolicy::Static(expected) => verify_checksum(bytes, expected),
        ChecksumPolicy::SkipLatest => {
            tracing::warn!(
                "Skipping checksum for {component_name} (latest URL, no stable hash) — \
                 the download is not integrity-verified"
            );
            Ok(())
        }
        // The installer must verify against the published hash itself (see
        // `verify_published_sha256`); reaching here means it did not, so fail
        // closed rather than install unverified bytes.
        ChecksumPolicy::Published | ChecksumPolicy::None => Err(CliDownloadError::NoChecksum),
    }
}

/// Finds the SHA256 for `file_name` in a published checksum file.
///
/// Accepts a single-artifact `.sha256` file (a bare hash, optionally followed
/// by the name) and a multi-line `SHA256SUMS` / `checksums.txt` (`<hash>
/// [*]<path>` per line, matched on the last path component).
pub(super) fn find_published_sha256(text: &str, file_name: &str) -> Option<String> {
    let mut bare = None;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(hash) = parts.next() else { continue };
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let hash = hash.to_ascii_lowercase();
        match parts.next() {
            Some(name) => {
                let name = name.trim_start_matches('*');
                if name.rsplit('/').next() == Some(file_name) {
                    return Some(hash);
                }
            }
            None => bare = Some(hash),
        }
    }
    bare
}

/// Verifies `bytes` against the SHA256 upstream publishes at `sums_url` for
/// `file_name` ([`ChecksumPolicy::Published`]).
///
/// The hash comes from the same vendor as the artifact, so this catches a
/// corrupted, truncated or swapped download and a CDN serving the wrong
/// release, not a compromised vendor.
///
/// # Errors
///
/// [`CliDownloadError::DownloadFailed`] if the checksum file cannot be fetched,
/// [`CliDownloadError::NoChecksum`] if it has no entry for `file_name`, and
/// [`CliDownloadError::ChecksumMismatch`] if the hash differs.
pub(super) async fn verify_published_sha256(
    sums_url: &str,
    file_name: &str,
    bytes: &[u8],
) -> CliDownloadResult<()> {
    let client = reqwest::Client::builder()
        .timeout(super::HTTP_DOWNLOAD_TIMEOUT)
        .user_agent("RustConn")
        .build()
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;
    let response = client
        .get(sums_url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;
    let text = response
        .text()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;
    let expected = find_published_sha256(&text, file_name).ok_or(CliDownloadError::NoChecksum)?;
    verify_checksum(bytes, &expected)?;
    tracing::info!(%file_name, "Download verified against the published SHA256");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // SHA256 of the ASCII bytes "hello" — precomputed, so a Static policy over
    // exactly this input must verify and any other hash must be a mismatch.
    const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    #[test]
    fn verify_checksum_accepts_matching_hash() {
        assert!(verify_checksum(b"hello", HELLO_SHA256).is_ok());
    }

    #[test]
    fn verify_checksum_rejects_wrong_hash() {
        // A mismatch (different bytes, same expected hash) must fail.
        assert!(matches!(
            verify_checksum(b"goodbye", HELLO_SHA256),
            Err(CliDownloadError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn enforce_static_policy_verifies() {
        assert!(
            enforce_checksum_policy(ChecksumPolicy::Static(HELLO_SHA256), "test", b"hello").is_ok()
        );
        assert!(matches!(
            enforce_checksum_policy(ChecksumPolicy::Static(HELLO_SHA256), "test", b"tampered"),
            Err(CliDownloadError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn enforce_skip_latest_allows_any_bytes() {
        // SkipLatest is not integrity-verified: it must succeed regardless of
        // content (the warning is a side effect we don't assert here).
        assert!(enforce_checksum_policy(ChecksumPolicy::SkipLatest, "test", b"anything").is_ok());
    }

    #[test]
    fn enforce_published_policy_fails_closed() {
        assert!(matches!(
            enforce_checksum_policy(ChecksumPolicy::Published, "test", b"anything"),
            Err(CliDownloadError::NoChecksum)
        ));
    }

    #[test]
    fn find_published_sha256_handles_every_upstream_format() {
        let h = HELLO_SHA256;
        // kubectl / tailscale: bare hash.
        assert_eq!(
            find_published_sha256(&format!("{h}\n"), "kubectl"),
            Some(h.into())
        );
        // teleport: hash + name.
        let tsh = format!("{h}  teleport-v18-linux-amd64-bin.tar.gz\n");
        assert_eq!(
            find_published_sha256(&tsh, "teleport-v18-linux-amd64-bin.tar.gz"),
            Some(h.into())
        );
        // hashicorp SHA256SUMS / goreleaser checksums.txt with paths and `*`.
        let other = "0".repeat(64);
        let sums = format!(
            "{other}  boundary_1_darwin_amd64.zip\n{h} *./dist/binaries/boundary_1_linux_amd64.zip\n"
        );
        assert_eq!(
            find_published_sha256(&sums, "boundary_1_linux_amd64.zip"),
            Some(h.into())
        );
        // Nearby inputs that must not match.
        assert_eq!(
            find_published_sha256(&sums, "boundary_1_linux_arm64.zip"),
            None
        );
        assert_eq!(find_published_sha256(&sums, "linux_amd64.zip"), None);
        assert_eq!(find_published_sha256("", "kubectl"), None);
        assert_eq!(find_published_sha256("<html>404</html>", "kubectl"), None);
        assert_eq!(find_published_sha256(&h[..63], "kubectl"), None);
        assert_eq!(
            find_published_sha256(&h.to_uppercase(), "kubectl"),
            Some(h.into()),
            "uppercase hex is normalised"
        );
    }

    #[test]
    fn enforce_none_policy_refuses() {
        assert!(matches!(
            enforce_checksum_policy(ChecksumPolicy::None, "test", b"anything"),
            Err(CliDownloadError::NoChecksum)
        ));
    }
}
