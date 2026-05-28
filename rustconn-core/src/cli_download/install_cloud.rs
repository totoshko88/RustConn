use std::path::{Path, PathBuf};

use super::{
    ChecksumPolicy, CliDownloadError, CliDownloadResult, DownloadCancellation, DownloadProgress,
    DownloadableComponent, ProgressCallback,
    download::{download_with_progress, verify_checksum},
    extract::{extract_tar_gz_preserve, extract_zip, find_binary_recursive},
};

#[expect(
    clippy::too_many_lines,
    reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
)]
pub(super) async fn install_gcloud(
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
            status: "Downloading Google Cloud CLI...".to_string(),
        });
    }

    let bytes = download_with_progress(url, &progress_callback, &cancel_token).await?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

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

    // Extract to cli_dir - the archive contains google-cloud-sdk/ directory
    extract_tar_gz_preserve(&bytes, cli_dir)?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Running install script...".to_string(),
        });
    }

    // Run install.sh
    let install_script = cli_dir.join("google-cloud-sdk/install.sh");
    if install_script.exists() {
        let mut cmd = tokio::process::Command::new("bash");
        cmd.args([
            install_script.to_str().unwrap_or("install.sh"),
            "--quiet",
            "--path-update=false",
            "--command-completion=false",
            "--usage-reporting=false",
        ]);

        if crate::flatpak::is_flatpak()
            && let Some(config_dir) = crate::flatpak::get_flatpak_gcloud_config_dir()
        {
            cmd.env("CLOUDSDK_CONFIG", &config_dir);
        }

        let output = cmd.output().await?;

        if !output.status.success() {
            tracing::warn!(
                "gcloud install.sh returned non-zero: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    } else {
        tracing::warn!("gcloud install.sh not found at {:?}", install_script);
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Done".to_string(),
        });
    }

    let binary_path = cli_dir.join("google-cloud-sdk/bin/gcloud");
    if binary_path.exists() {
        Ok(binary_path)
    } else {
        tracing::error!("gcloud binary not found at {:?}", binary_path);
        if let Ok(entries) = std::fs::read_dir(cli_dir) {
            for entry in entries.flatten() {
                tracing::debug!("  cli_dir contains: {:?}", entry.path());
            }
        }
        let sdk_dir = cli_dir.join("google-cloud-sdk");
        if sdk_dir.exists()
            && let Ok(entries) = std::fs::read_dir(&sdk_dir)
        {
            for entry in entries.flatten() {
                tracing::debug!("  google-cloud-sdk contains: {:?}", entry.path());
            }
        }
        Err(CliDownloadError::ExtractionFailed(
            "gcloud binary not found. Check logs for details.".to_string(),
        ))
    }
}

/// Install AWS CLI v2
#[expect(
    clippy::too_many_lines,
    reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
)]
pub(super) async fn install_aws_cli(
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
            status: "Downloading AWS CLI...".to_string(),
        });
    }

    let bytes = download_with_progress(url, &progress_callback, &cancel_token).await?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

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

    let temp_dir = cli_dir.join("aws-cli-temp");
    tokio::fs::create_dir_all(&temp_dir).await?;

    extract_zip(&bytes, &temp_dir)?;

    if cancel_token.is_cancelled() {
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        return Err(CliDownloadError::Cancelled);
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Running installer...".to_string(),
        });
    }

    let install_script = temp_dir.join("aws/install");
    let install_dir = cli_dir.join("aws-cli");

    if install_script.exists() {
        let output = tokio::process::Command::new(&install_script)
            .args([
                "--install-dir",
                install_dir.to_str().unwrap_or("aws-cli"),
                "--bin-dir",
                install_dir.join("bin").to_str().unwrap_or("bin"),
                "--update",
            ])
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("AWS CLI installer returned non-zero: {}", stderr);
        }
    } else {
        tracing::warn!("AWS CLI installer not found at {:?}", install_script);
        if let Some(found) = find_binary_recursive(&temp_dir, "install", 3) {
            tracing::info!("Found installer at {:?}", found);
            let output = tokio::process::Command::new(&found)
                .args([
                    "--install-dir",
                    install_dir.to_str().unwrap_or("aws-cli"),
                    "--bin-dir",
                    install_dir.join("bin").to_str().unwrap_or("bin"),
                    "--update",
                ])
                .output()
                .await?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!("AWS CLI installer returned non-zero: {}", stderr);
            }
        }
    }

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Done".to_string(),
        });
    }

    let binary_path = install_dir.join("bin/aws");
    if binary_path.exists() {
        return Ok(binary_path);
    }

    let v2_binary = install_dir.join("v2/current/bin/aws");
    if v2_binary.exists() {
        return Ok(v2_binary);
    }

    if let Some(found) = find_binary_recursive(&install_dir, "aws", 5) {
        return Ok(found);
    }

    tracing::error!("AWS CLI binary not found after installation");
    Err(CliDownloadError::ExtractionFailed(
        "AWS CLI binary not found. Check logs for details.".to_string(),
    ))
}

/// Installs Bitwarden CLI by auto-detecting the latest version from GitHub.
pub(super) async fn install_bitwarden(
    component: &DownloadableComponent,
    cli_dir: &Path,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: "Detecting latest Bitwarden CLI version...".to_string(),
        });
    }

    let client = reqwest::Client::builder()
        .timeout(super::HTTP_DOWNLOAD_TIMEOUT)
        .user_agent("RustConn")
        .build()
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    let response = client
        .get("https://api.github.com/repos/bitwarden/clients/releases?per_page=30")
        .send()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?
        .text()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    let tag_re = regex::Regex::new(r#""tag_name"\s*:\s*"cli-v([^"]+)""#)
        .map_err(|e| CliDownloadError::NotAvailable(format!("Regex error: {e}")))?;

    let version = tag_re
        .captures(&response)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| {
            CliDownloadError::NotAvailable(
                "Could not detect latest Bitwarden CLI version from GitHub API".to_string(),
            )
        })?;

    let download_url = format!(
        "https://github.com/bitwarden/clients/releases/download/cli-v{version}/bw-linux-{version}.zip"
    );

    tracing::info!(%version, %download_url, "Bitwarden CLI: detected latest version");

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: format!("Downloading Bitwarden CLI {version}..."),
        });
    }

    let bytes = download_with_progress(&download_url, &progress_callback, &cancel_token).await?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Extracting Bitwarden CLI...".to_string(),
        });
    }

    let install_dir = cli_dir.join(component.install_subdir);
    tokio::fs::create_dir_all(&install_dir).await?;

    extract_zip(&bytes, &install_dir)?;

    let binary_path = install_dir.join(component.binary_name);
    if !binary_path.exists() {
        return Err(CliDownloadError::NotAvailable(
            "bw binary not found after extraction".to_string(),
        ));
    }

    let version_file = install_dir.join(".version");
    tokio::fs::write(&version_file, &version).await?;

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: format!("Bitwarden CLI {version} installed successfully"),
        });
    }

    Ok(binary_path)
}

/// Installs 1Password CLI by auto-detecting the latest version.
pub(super) async fn install_1password(
    component: &DownloadableComponent,
    cli_dir: &Path,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: "Detecting latest 1Password CLI version...".to_string(),
        });
    }

    let client = reqwest::Client::builder()
        .timeout(super::HTTP_DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    let response = client
        .get("https://app-updates.agilebits.com/check/1/0/CLI2/en/2.0.0/N")
        .send()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?
        .text()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    let version_re = regex::Regex::new(r#""version"\s*:\s*"([^"]+)""#)
        .map_err(|e| CliDownloadError::NotAvailable(format!("Regex error: {e}")))?;

    let version = version_re
        .captures(&response)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| {
            CliDownloadError::NotAvailable(
                "Could not detect latest 1Password CLI version from update API".to_string(),
            )
        })?;

    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };

    let download_url = format!(
        "https://cache.agilebits.com/dist/1P/op2/pkg/v{version}/op_linux_{arch}_v{version}.zip"
    );

    tracing::info!(%version, %download_url, "1Password CLI: detected latest version");

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: format!("Downloading 1Password CLI {version}..."),
        });
    }

    let bytes = download_with_progress(&download_url, &progress_callback, &cancel_token).await?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: "Extracting 1Password CLI...".to_string(),
        });
    }

    let install_dir = cli_dir.join(component.install_subdir);
    tokio::fs::create_dir_all(&install_dir).await?;

    extract_zip(&bytes, &install_dir)?;

    let binary_path = install_dir.join(component.binary_name);
    if !binary_path.exists() {
        return Err(CliDownloadError::NotAvailable(
            "op binary not found after extraction".to_string(),
        ));
    }

    let version_file = install_dir.join(".version");
    tokio::fs::write(&version_file, &version).await?;

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: format!("1Password CLI {version} installed successfully"),
        });
    }

    Ok(binary_path)
}
