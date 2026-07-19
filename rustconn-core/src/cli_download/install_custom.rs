use std::path::{Path, PathBuf};

use super::download::download_with_progress;
use super::extract::{extract_tar_gz, extract_zip};
use super::{
    CliDownloadError, CliDownloadResult, DownloadCancellation, DownloadProgress,
    DownloadableComponent, ProgressCallback,
};

pub(super) async fn install_custom_component(
    component: &DownloadableComponent,
    cli_dir: &Path,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    match component.id {
        "gcloud" => {
            super::install_cloud::install_gcloud(
                component,
                cli_dir,
                progress_callback,
                cancel_token,
            )
            .await
        }
        "aws" => {
            super::install_cloud::install_aws_cli(
                component,
                cli_dir,
                progress_callback,
                cancel_token,
            )
            .await
        }
        "tailscale" => install_tailscale(component, cli_dir, progress_callback, cancel_token).await,
        "kubectl" => install_kubectl(component, cli_dir, progress_callback, cancel_token).await,
        "tsh" => install_teleport(component, cli_dir, progress_callback, cancel_token).await,
        "boundary" => install_boundary(component, cli_dir, progress_callback, cancel_token).await,
        "hoop" => install_hoop(component, cli_dir, progress_callback, cancel_token).await,
        "bw" => {
            super::install_cloud::install_bitwarden(
                component,
                cli_dir,
                progress_callback,
                cancel_token,
            )
            .await
        }
        "op" => {
            super::install_cloud::install_1password(
                component,
                cli_dir,
                progress_callback,
                cancel_token,
            )
            .await
        }
        _ => Err(CliDownloadError::NotAvailable(format!(
            "Custom installation not implemented for {}",
            component.id
        ))),
    }
}

/// Installs kubectl by auto-detecting the latest stable version from
/// `https://dl.k8s.io/release/stable.txt` and downloading the binary.
pub(super) async fn install_kubectl(
    component: &DownloadableComponent,
    cli_dir: &Path,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: "Detecting latest kubectl version...".to_string(),
        });
    }

    let client = reqwest::Client::builder()
        .timeout(super::HTTP_DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    let version = client
        .get("https://dl.k8s.io/release/stable.txt")
        .send()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?
        .text()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?
        .trim()
        .to_string();

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };

    let download_url = format!("https://dl.k8s.io/release/{version}/bin/linux/{arch}/kubectl");

    tracing::info!(%version, %download_url, "kubectl: detected latest stable version");

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: format!("Downloading kubectl {version}..."),
        });
    }

    let bytes = download_with_progress(&download_url, &progress_callback, &cancel_token).await?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    let install_dir = cli_dir.join(component.install_subdir);
    tokio::fs::create_dir_all(&install_dir).await?;

    let binary_path = install_dir.join(component.binary_name);
    tokio::fs::write(&binary_path, &bytes).await?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755))
            .map_err(CliDownloadError::IoError)?;
    }

    let version_file = install_dir.join(".version");
    tokio::fs::write(&version_file, &version).await?;

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: format!("kubectl {version} installed successfully"),
        });
    }

    Ok(binary_path)
}

/// Installs Teleport tsh by auto-detecting the latest version from GitHub
/// releases API and downloading the tar.gz from CDN.
pub(super) async fn install_teleport(
    component: &DownloadableComponent,
    cli_dir: &Path,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: "Detecting latest Teleport version...".to_string(),
        });
    }

    let client = reqwest::Client::builder()
        .timeout(super::HTTP_DOWNLOAD_TIMEOUT)
        .user_agent("RustConn")
        .build()
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    let response = client
        .get("https://api.github.com/repos/gravitational/teleport/releases/latest")
        .send()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?
        .text()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    let tag_re = regex::Regex::new(r#""tag_name"\s*:\s*"v([^"]+)""#)
        .map_err(|e| CliDownloadError::NotAvailable(format!("Regex error: {e}")))?;

    let version = tag_re
        .captures(&response)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| {
            CliDownloadError::NotAvailable(
                "Could not detect latest Teleport version from GitHub API".to_string(),
            )
        })?;

    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };

    let download_url =
        format!("https://cdn.teleport.dev/teleport-v{version}-linux-{arch}-bin.tar.gz");

    tracing::info!(%version, %download_url, "Teleport: detected latest version");

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: format!("Downloading Teleport {version}..."),
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
            status: "Extracting Teleport...".to_string(),
        });
    }

    let install_dir = cli_dir.join(component.install_subdir);
    tokio::fs::create_dir_all(&install_dir).await?;

    extract_tar_gz(&bytes, &install_dir)?;

    let binary_path = install_dir.join(component.binary_name);
    if !binary_path.exists() {
        return Err(CliDownloadError::NotAvailable(
            "tsh binary not found after extraction".to_string(),
        ));
    }

    let version_file = install_dir.join(".version");
    tokio::fs::write(&version_file, &version).await?;

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: format!("Teleport {version} installed successfully"),
        });
    }

    Ok(binary_path)
}

/// Installs Tailscale by auto-detecting the latest stable version.
pub(super) async fn install_tailscale(
    component: &DownloadableComponent,
    cli_dir: &Path,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: "Detecting latest Tailscale version...".to_string(),
        });
    }

    let index_url = "https://pkgs.tailscale.com/stable/";
    let client = reqwest::Client::builder()
        .timeout(super::HTTP_DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    let index_body = client
        .get(index_url)
        .send()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?
        .text()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    let arch_suffix = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };

    let pattern = format!("tailscale_([0-9]+\\.[0-9]+\\.[0-9]+)_{arch_suffix}\\.tgz");
    let re = regex::Regex::new(&pattern)
        .map_err(|e| CliDownloadError::NotAvailable(format!("Regex error: {e}")))?;

    let version = re
        .captures(&index_body)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| {
            CliDownloadError::NotAvailable(
                "Could not detect latest Tailscale version from stable page".to_string(),
            )
        })?;

    let download_url =
        format!("https://pkgs.tailscale.com/stable/tailscale_{version}_{arch_suffix}.tgz");

    tracing::info!(%version, %download_url, "Tailscale: detected latest version");

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: format!("Downloading Tailscale {version}..."),
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
            status: "Extracting Tailscale...".to_string(),
        });
    }

    let install_dir = cli_dir.join(component.install_subdir);
    tokio::fs::create_dir_all(&install_dir).await?;

    extract_tar_gz(&bytes, &install_dir)?;

    let binary_path = install_dir.join(component.binary_name);
    if !binary_path.exists() {
        return Err(CliDownloadError::NotAvailable(
            "Tailscale binary not found after extraction".to_string(),
        ));
    }

    let version_file = install_dir.join(".version");
    tokio::fs::write(&version_file, &version).await?;

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: format!("Tailscale {version} installed successfully"),
        });
    }

    Ok(binary_path)
}

/// Installs HashiCorp Boundary by auto-detecting the latest version.
pub(super) async fn install_boundary(
    component: &DownloadableComponent,
    cli_dir: &Path,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: "Detecting latest Boundary version...".to_string(),
        });
    }

    let client = reqwest::Client::builder()
        .timeout(super::HTTP_DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    let response = client
        .get("https://checkpoint-api.hashicorp.com/v1/check/boundary")
        .send()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?
        .text()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    let version_re = regex::Regex::new(r#""current_version"\s*:\s*"([^"]+)""#)
        .map_err(|e| CliDownloadError::NotAvailable(format!("Regex error: {e}")))?;

    let version = version_re
        .captures(&response)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| {
            CliDownloadError::NotAvailable(
                "Could not detect latest Boundary version from checkpoint API".to_string(),
            )
        })?;

    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };

    let download_url = format!(
        "https://releases.hashicorp.com/boundary/{version}/boundary_{version}_linux_{arch}.zip"
    );

    tracing::info!(%version, %download_url, "Boundary: detected latest version");

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: format!("Downloading Boundary {version}..."),
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
            status: "Extracting Boundary...".to_string(),
        });
    }

    let install_dir = cli_dir.join(component.install_subdir);
    tokio::fs::create_dir_all(&install_dir).await?;

    extract_zip(&bytes, &install_dir)?;

    let binary_path = install_dir.join(component.binary_name);
    if !binary_path.exists() {
        return Err(CliDownloadError::NotAvailable(
            "boundary binary not found after extraction".to_string(),
        ));
    }

    let version_file = install_dir.join(".version");
    tokio::fs::write(&version_file, &version).await?;

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: format!("Boundary {version} installed successfully"),
        });
    }

    Ok(binary_path)
}

/// Installs Hoop by auto-detecting the latest version.
pub(super) async fn install_hoop(
    component: &DownloadableComponent,
    cli_dir: &Path,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: "Detecting latest Hoop version...".to_string(),
        });
    }

    let client = reqwest::Client::builder()
        .timeout(super::HTTP_DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    let version = client
        .get("https://releases.hoop.dev/release/latest.txt")
        .send()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?
        .text()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?
        .trim()
        .to_string();

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    if version.is_empty() {
        return Err(CliDownloadError::NotAvailable(
            "Could not detect latest Hoop version from latest.txt".to_string(),
        ));
    }

    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    };

    let download_url =
        format!("https://releases.hoop.dev/release/{version}/hoop_{version}_Linux_{arch}.tar.gz");

    tracing::info!(%version, %download_url, "Hoop: detected latest version");

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: format!("Downloading Hoop {version}..."),
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
            status: "Extracting Hoop...".to_string(),
        });
    }

    let install_dir = cli_dir.join(component.install_subdir);
    tokio::fs::create_dir_all(&install_dir).await?;

    extract_tar_gz(&bytes, &install_dir)?;

    let binary_path = install_dir.join(component.binary_name);
    if !binary_path.exists() {
        return Err(CliDownloadError::NotAvailable(
            "hoop binary not found after extraction".to_string(),
        ));
    }

    let version_file = install_dir.join(".version");
    tokio::fs::write(&version_file, &version).await?;

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
            status: format!("Hoop {version} installed successfully"),
        });
    }

    Ok(binary_path)
}
