use std::path::{Path, PathBuf};

use super::download::{download_with_progress, verify_checksum};
use super::extract::{extract_deb, extract_tar_gz, extract_zip, find_binary_in_dir};
use super::{
    ChecksumPolicy, CliDownloadError, CliDownloadResult, DownloadCancellation, DownloadProgress,
    DownloadableComponent, ProgressCallback,
};

pub(super) async fn install_download_component(
    component: &DownloadableComponent,
    cli_dir: &Path,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    let url = component
        .download_url_for_arch()
        .ok_or_else(|| CliDownloadError::NotAvailable("No download URL".to_string()))?;

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: format!("Downloading {}...", component.name),
        });
    }

    // Download with progress
    let bytes = download_with_progress(url, &progress_callback, &cancel_token).await?;

    // Check cancellation before verification
    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Verifying checksum...".to_string(),
        });
    }

    // Verify checksum based on policy
    match component.checksum {
        ChecksumPolicy::Static(expected) => {
            verify_checksum(&bytes, expected)?;
        }
        ChecksumPolicy::SkipLatest => {
            tracing::warn!(
                "Skipping checksum for {} (latest URL, no stable hash)",
                component.name
            );
        }
        ChecksumPolicy::None => {
            return Err(CliDownloadError::NoChecksum);
        }
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Extracting...".to_string(),
        });
    }

    let install_dir = cli_dir.join(component.install_subdir);
    tokio::fs::create_dir_all(&install_dir).await?;

    // Determine file type and extract
    let url_lower = url.to_lowercase();
    #[expect(
        clippy::case_sensitive_file_extension_comparisons,
        reason = "extension comes from a controlled allow-list, not user input - case is already normalised upstream"
    )]
    if url_lower.ends_with(".zip") {
        extract_zip(&bytes, &install_dir)?;
    } else if url_lower.ends_with(".tar.gz") || url_lower.ends_with(".tgz") {
        extract_tar_gz(&bytes, &install_dir)?;
    } else if url_lower.ends_with(".deb") {
        extract_deb(&bytes, &install_dir)?;
    } else {
        // Single binary file
        let binary_path = install_dir.join(component.binary_name);
        tokio::fs::write(&binary_path, &bytes).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = tokio::fs::metadata(&binary_path).await?.permissions();
            perms.set_mode(0o755);
            tokio::fs::set_permissions(&binary_path, perms).await?;
        }
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Done".to_string(),
        });
    }

    // Try to find the binary - it might be in a subdirectory or have a different name
    let binary_path = find_binary_in_dir(&install_dir, component.binary_name)?;
    Ok(binary_path)
}
