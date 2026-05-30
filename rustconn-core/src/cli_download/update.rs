use std::path::{Path, PathBuf};

use super::{
    CliDownloadError, CliDownloadResult, DownloadCancellation, DownloadProgress,
    DownloadableComponent, InstallMethod, ProgressCallback, get_cli_install_dir,
    install::install_download_component,
    install_custom,
    install_pip::{create_pip_wrapper_script, ensure_pip_available},
};

/// Update a component (uninstall and reinstall)
pub(super) async fn update_component_impl(
    component: &DownloadableComponent,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    if !crate::is_sandboxed() {
        return Err(CliDownloadError::NotSandboxed);
    }

    if !component.is_downloadable() {
        return Err(CliDownloadError::NotAvailable(component.name.to_string()));
    }

    // Report progress
    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: format!("Removing old version of {}...", component.name),
        });
    }

    // Remove existing installation
    let cli_dir = get_cli_install_dir().ok_or(CliDownloadError::NotSandboxed)?;
    let install_dir = cli_dir.join(component.install_subdir);

    if install_dir.exists() {
        tokio::fs::remove_dir_all(&install_dir).await?;
    }

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    // Reinstall
    match component.install_method {
        InstallMethod::Download => {
            install_download_component(component, &cli_dir, progress_callback, cancel_token).await
        }
        InstallMethod::Pip => {
            update_pip_component(component, &cli_dir, progress_callback, cancel_token).await
        }
        InstallMethod::CustomScript => {
            install_custom::install_custom_component(
                component,
                &cli_dir,
                progress_callback,
                cancel_token,
            )
            .await
        }
        InstallMethod::SystemPackage { .. } => Err(CliDownloadError::NotAvailable(
            "System packages cannot be updated through RustConn".to_string(),
        )),
    }
}

/// Update a pip-based component using pip install --upgrade
async fn update_pip_component(
    component: &DownloadableComponent,
    cli_dir: &Path,
    progress_callback: ProgressCallback,
    cancel_token: DownloadCancellation,
) -> CliDownloadResult<PathBuf> {
    let pip_package = component
        .pip_package
        .ok_or_else(|| CliDownloadError::NotAvailable("No pip package specified".to_string()))?;

    let python_dir = cli_dir.join("python");

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: "Checking pip availability...".to_string(),
        });
    }

    // Ensure pip is available
    let _pip_path = ensure_pip_available(&python_dir).await?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: format!("Updating {}...", component.name),
        });
    }

    // Update the package using python -m pip with --upgrade flag
    let output = tokio::process::Command::new("python3")
        .args([
            "-m",
            "pip",
            "install",
            "--user",
            "--upgrade",
            "--no-warn-script-location",
            pip_package,
        ])
        .env("PYTHONUSERBASE", &python_dir)
        .output()
        .await?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("pip upgrade failed for {}: {}", pip_package, stderr);
        return Err(CliDownloadError::PipInstallFailed(stderr.to_string()));
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 100,
            total: Some(100),
            status: "Updating wrapper script...".to_string(),
        });
    }

    // Recreate wrapper script (in case Python version changed)
    let binary_path = create_pip_wrapper_script(&python_dir, component).await?;

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 100,
            total: Some(100),
            status: "Done".to_string(),
        });
    }

    Ok(binary_path)
}
