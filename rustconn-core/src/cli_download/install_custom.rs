use std::path::{Path, PathBuf};

use super::download::{download_with_progress, verify_checksum, verify_published_sha256};
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
        "pass-cli" => {
            install_proton_pass(component, cli_dir, progress_callback, cancel_token).await
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

    verify_published_sha256(&format!("{download_url}.sha256"), "kubectl", &bytes).await?;

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

    verify_published_sha256(
        &format!("{download_url}.sha256"),
        &format!("teleport-v{version}-linux-{arch}-bin.tar.gz"),
        &bytes,
    )
    .await?;

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

    verify_published_sha256(
        &format!("{download_url}.sha256"),
        &format!("tailscale_{version}_{arch_suffix}.tgz"),
        &bytes,
    )
    .await?;

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

    verify_published_sha256(
        &format!("https://releases.hashicorp.com/boundary/{version}/boundary_{version}_SHA256SUMS"),
        &format!("boundary_{version}_linux_{arch}.zip"),
        &bytes,
    )
    .await?;

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

    verify_published_sha256(
        &format!("https://releases.hoop.dev/release/{version}/checksums.txt"),
        &format!("hoop_{version}_Linux_{arch}.tar.gz"),
        &bytes,
    )
    .await?;

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

/// Proton's release manifest for `pass-cli`, the one its own `install.sh` reads.
pub(super) const PROTON_PASS_MANIFEST_URL: &str =
    "https://proton.me/download/pass-cli/versions.json";

/// One Linux build of `pass-cli`, as listed in [`PROTON_PASS_MANIFEST_URL`].
#[derive(Debug, PartialEq, Eq)]
struct PassCliRelease {
    version: String,
    url: String,
    sha256: String,
}

/// Reads the `arch` (`x86_64` or `aarch64`) Linux build from Proton's manifest.
///
/// Refuses everything `install.sh` refuses — a `formatVersion` other than 1, a
/// missing version, URL or hash — and also a URL outside `https://proton.me/`,
/// a hash that is not 64 hex digits, and a version that is not a plain version
/// string: the manifest is the only thing vouching for the binary.
fn parse_pass_cli_manifest(text: &str, arch: &str) -> CliDownloadResult<PassCliRelease> {
    let manifest: serde_json::Value = serde_json::from_str(text).map_err(|e| {
        CliDownloadError::NotAvailable(format!("Proton Pass manifest is not valid JSON: {e}"))
    })?;
    if manifest
        .get("formatVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        return Err(CliDownloadError::NotAvailable(
            "Unsupported Proton Pass manifest format version".to_string(),
        ));
    }

    let versions = &manifest["passCliVersions"];
    let build = &versions["urls"]["linux"][arch];
    let field =
        |value: &serde_json::Value| value.as_str().filter(|s| !s.is_empty()).map(str::to_string);
    let (Some(version), Some(url), Some(hash)) = (
        field(&versions["version"]),
        field(&build["url"]),
        field(&build["hash"]),
    ) else {
        return Err(CliDownloadError::NotAvailable(format!(
            "Proton Pass manifest lists no Linux {arch} build"
        )));
    };

    if !version
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
    {
        return Err(CliDownloadError::NotAvailable(
            "Proton Pass manifest has a malformed version".to_string(),
        ));
    }
    if !url.starts_with("https://proton.me/") {
        return Err(CliDownloadError::NotAvailable(
            "Proton Pass manifest points outside proton.me".to_string(),
        ));
    }
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(CliDownloadError::NoChecksum);
    }

    Ok(PassCliRelease {
        version,
        url,
        sha256: hash.to_ascii_lowercase(),
    })
}

/// Installs the Proton Pass CLI from the build Proton's manifest names, refusing
/// it unless it matches the manifest's SHA-256.
pub(super) async fn install_proton_pass(
    component: &DownloadableComponent,
    cli_dir: &Path,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: "Detecting latest Proton Pass CLI version...".to_string(),
        });
    }

    let client = reqwest::Client::builder()
        .timeout(super::HTTP_DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    let manifest = client
        .get(PROTON_PASS_MANIFEST_URL)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?
        .text()
        .await
        .map_err(|e| CliDownloadError::DownloadFailed(e.to_string()))?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };
    let release = parse_pass_cli_manifest(&manifest, arch)?;
    let version = release.version;

    tracing::info!(%version, download_url = %release.url, "Proton Pass CLI: detected latest version");

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: format!("Downloading Proton Pass CLI {version}..."),
        });
    }

    let bytes = download_with_progress(&release.url, &progress_callback, &cancel_token).await?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    verify_checksum(&bytes, &release.sha256)?;

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
            status: format!("Proton Pass CLI {version} installed successfully"),
        });
    }

    Ok(binary_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "3e9520fe79ab4698e03905c8bf0fff4889b3028975e832a5f801a9c82008b612";

    fn manifest(format: &str, url: &str, hash: &str) -> String {
        format!(
            r#"{{"formatVersion": {format}, "passCliVersions": {{"version": "2.7.1", "urls": {{
                "linux": {{"x86_64": {{"url": "{url}", "hash": "{hash}"}}}},
                "macos": {{"aarch64": {{"url": "https://proton.me/m", "hash": "{HASH}"}}}}
            }}}}}}"#
        )
    }

    #[test]
    fn pass_cli_manifest_yields_the_linux_build() {
        let url = "https://proton.me/download/pass-cli/2.7.1/pass-cli-linux-x86_64";
        let release = parse_pass_cli_manifest(&manifest("1", url, &HASH.to_uppercase()), "x86_64")
            .expect("valid manifest");
        assert_eq!(
            release,
            PassCliRelease {
                version: "2.7.1".to_string(),
                url: url.to_string(),
                sha256: HASH.to_string(),
            }
        );
    }

    #[test]
    fn pass_cli_manifest_refuses_what_it_cannot_vouch_for() {
        let url = "https://proton.me/download/pass-cli/2.7.1/pass-cli-linux-x86_64";
        // Unknown format, as install.sh refuses.
        assert!(parse_pass_cli_manifest(&manifest("2", url, HASH), "x86_64").is_err());
        // No build for this architecture (only macOS has aarch64 here).
        assert!(parse_pass_cli_manifest(&manifest("1", url, HASH), "aarch64").is_err());
        // A download from anywhere but proton.me.
        let evil = "https://example.com/pass-cli";
        assert!(parse_pass_cli_manifest(&manifest("1", evil, HASH), "x86_64").is_err());
        let lookalike = "https://proton.me.example.com/pass-cli";
        assert!(parse_pass_cli_manifest(&manifest("1", lookalike, HASH), "x86_64").is_err());
        // A hash that is not a SHA-256.
        assert!(matches!(
            parse_pass_cli_manifest(&manifest("1", url, &HASH[..63]), "x86_64"),
            Err(CliDownloadError::NoChecksum)
        ));
        assert!(matches!(
            parse_pass_cli_manifest(&manifest("1", url, ""), "x86_64"),
            Err(CliDownloadError::NotAvailable(_))
        ));
        assert!(parse_pass_cli_manifest("<html>404</html>", "x86_64").is_err());
        let bad_version = manifest("1", url, HASH).replace("2.7.1", "../x");
        assert!(parse_pass_cli_manifest(&bad_version, "x86_64").is_err());
    }
}
