//! `codec` command handler — on-demand media codec management.
//!
//! The only operation today is downloading Cisco's official OpenH264 binary,
//! which is the sole library the RDP GFX H.264 loader accepts on a packaged
//! install (distribution builds are refused by the loader's SHA-256 allow-list).
//! Cisco holds the MPEG-LA patent licence for the binary it distributes, so the
//! download is strictly user-initiated and gated behind `--accept-cisco-license`.

use rustconn_core::rdp_client::openh264_download::{self, OPENH264_VERSION, OpenH264DownloadError};

use crate::cli::CodecCommands;
use crate::error::CliError;

/// Cisco OpenH264 binary licence URL, surfaced so the user can read it before
/// accepting.
const CISCO_LICENSE_URL: &str = "https://www.openh264.org/BINARY_LICENSE.txt";

/// Dispatch a `codec` subcommand.
///
/// # Errors
///
/// Returns [`CliError::Protocol`] when the platform is unsupported, the download
/// fails, decompression fails, or the downloaded bytes fail SHA-256
/// verification against Cisco's published hash.
#[expect(
    clippy::needless_pass_by_value,
    reason = "owned subcommand matches the other cmd_* dispatch handlers; the \
              enum is consumed here as the single owner of the parsed args"
)]
pub(super) fn cmd_codec(subcmd: CodecCommands) -> Result<(), CliError> {
    match subcmd {
        CodecCommands::DownloadH264 {
            accept_cisco_license,
        } => cmd_download_h264(accept_cisco_license),
    }
}

/// Download Cisco's OpenH264 binary (requires `--accept-cisco-license`).
fn cmd_download_h264(accept: bool) -> Result<(), CliError> {
    // No pinned Cisco binary for this platform → bail early with a clear message
    // rather than attempting a download that cannot succeed.
    let Some(artifact) = openh264_download::artifact() else {
        return Err(CliError::Protocol(
            "no Cisco OpenH264 binary is published for this platform (os/arch unsupported)"
                .to_string(),
        ));
    };

    let url = openh264_download::download_url().unwrap_or_default();

    if !accept {
        // Dry explanation: say exactly what accepting would do, and show the
        // licence, so consent is informed. Download nothing.
        println!("H.264 codec download for RDP GFX (OpenH264 {OPENH264_VERSION})");
        println!();
        println!("This downloads Cisco's official OpenH264 binary from Cisco's CDN:");
        println!("    {url}");
        if let Some(dest) = openh264_download::cache_path() {
            println!("and installs it to:");
            println!("    {}", dest.display());
        }
        println!();
        println!(
            "Cisco — not RustConn — provides this binary and holds the MPEG-LA\n\
             patent licence for it. By downloading you accept Cisco's licence:"
        );
        println!("    {CISCO_LICENSE_URL}");
        println!();
        println!("Re-run with --accept-cisco-license to download.");
        return Ok(());
    }

    // Already installed and valid? Report and stop — no re-download.
    if let Some(existing) = openh264_download::cached_openh264_path() {
        println!(
            "OpenH264 {OPENH264_VERSION} already installed and verified at:\n    {}",
            existing.display()
        );
        println!("Restart RustConn to use H.264 in RDP GFX sessions.");
        return Ok(());
    }

    println!(
        "Downloading Cisco OpenH264 {OPENH264_VERSION} ({})…",
        artifact.file
    );

    // Bridge the async downloader onto a one-shot runtime — the same pattern the
    // other async CLI handlers (secret, test, dynamic_folder) use.
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| CliError::Protocol(format!("failed to create async runtime: {e}")))?;

    let path = runtime
        .block_on(openh264_download::download_openh264(true))
        .map_err(map_download_error)?;

    println!("Installed and SHA-256-verified at:\n    {}", path.display());
    println!("Restart RustConn to use H.264 in RDP GFX sessions.");
    Ok(())
}

/// Map the module's error enum onto a CLI error with an actionable message.
fn map_download_error(err: OpenH264DownloadError) -> CliError {
    match err {
        OpenH264DownloadError::ConsentRequired => {
            // Should not reach here (we pass consent==true), but map it honestly.
            CliError::Protocol("consent required — pass --accept-cisco-license".to_string())
        }
        OpenH264DownloadError::ChecksumMismatch { expected, actual } => {
            CliError::Protocol(format!(
                "downloaded OpenH264 failed SHA-256 verification \
             (expected {expected}, got {actual}); nothing was installed"
            ))
        }
        other => CliError::Protocol(other.to_string()),
    }
}
