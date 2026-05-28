use std::path::{Path, PathBuf};

use super::{
    CliDownloadError, CliDownloadResult, DownloadCancellation, DownloadProgress,
    DownloadableComponent, ProgressCallback,
};

/// Check if pip is available (either system pip or our installed pip)
pub(super) async fn ensure_pip_available(python_dir: &Path) -> CliDownloadResult<PathBuf> {
    // First check if pip is already available
    let pip_check = tokio::process::Command::new("python3")
        .args(["-m", "pip", "--version"])
        .output()
        .await;

    if let Ok(output) = pip_check
        && output.status.success()
    {
        tracing::debug!("System pip is available");
        return Ok(PathBuf::from("pip")); // Use system pip
    }

    // Check if we have pip installed in our python directory
    let local_pip = python_dir.join("bin/pip3");
    if local_pip.exists() {
        tracing::debug!("Local pip found at {:?}", local_pip);
        return Ok(local_pip);
    }

    // Install pip using ensurepip
    tracing::info!("Installing pip via ensurepip...");

    tokio::fs::create_dir_all(python_dir).await?;

    let output = tokio::process::Command::new("python3")
        .args(["-m", "ensurepip", "--user", "--upgrade"])
        .env("PYTHONUSERBASE", python_dir)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("ensurepip failed: {}", stderr);
        return Err(CliDownloadError::PipInstallFailed(format!(
            "Failed to install pip via ensurepip: {}",
            stderr
        )));
    }

    tracing::info!("pip installed successfully via ensurepip");

    // Return path to the installed pip
    let pip_path = python_dir.join("bin/pip3");
    if pip_path.exists() {
        Ok(pip_path)
    } else {
        // Try pip instead of pip3
        let pip_path = python_dir.join("bin/pip");
        if pip_path.exists() {
            Ok(pip_path)
        } else {
            // Fall back to using python -m pip
            Ok(PathBuf::from("python3"))
        }
    }
}

/// Install a pip-based component
pub(super) async fn install_pip_component(
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

    // Ensure pip is available (we call this to install pip if needed, but use python -m pip)
    ensure_pip_available(&python_dir).await?;

    if cancel_token.is_cancelled() {
        return Err(CliDownloadError::Cancelled);
    }

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 0,
            total: None,
            status: format!("Installing {}...", component.name),
        });
    }

    // Install the package using pip with --target to control installation location
    let output = tokio::process::Command::new("python3")
        .args([
            "-m",
            "pip",
            "install",
            "--user",
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
        tracing::error!("pip install failed for {}: {}", pip_package, stderr);
        return Err(CliDownloadError::PipInstallFailed(stderr.to_string()));
    }

    // Log pip output for debugging
    let stdout = String::from_utf8_lossy(&output.stdout);
    tracing::debug!("pip install output: {}", stdout);

    if let Some(ref cb) = progress_callback {
        cb(DownloadProgress {
            downloaded: 100,
            total: Some(100),
            status: "Creating wrapper script...".to_string(),
        });
    }

    // pip with PYTHONUSERBASE doesn't create console scripts in bin/
    // We need to create wrapper scripts manually
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

/// Create a wrapper script for pip-installed CLI tools
///
/// pip with `--user` and `PYTHONUSERBASE` installs packages to site-packages
/// but doesn't create console scripts in bin/. We create wrapper scripts
/// that invoke the Python module directly.
#[expect(
    clippy::too_many_lines,
    reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
)]
pub(super) async fn create_pip_wrapper_script(
    python_dir: &Path,
    component: &DownloadableComponent,
) -> CliDownloadResult<PathBuf> {
    let bin_dir = python_dir.join("bin");
    tokio::fs::create_dir_all(&bin_dir).await?;

    let binary_path = bin_dir.join(component.binary_name);

    // Determine the Python module/entry point based on the component
    let script_content = match component.id {
        "az" => {
            format!(
                r#"#!/bin/bash
# Wrapper script for {name} CLI
# Auto-generated by RustConn

export PYTHONUSERBASE="{python_dir}"
PYVER=$(python3 -c "import sys; print(f'python{{sys.version_info.major}}.{{sys.version_info.minor}}')" 2>/dev/null || echo "python3")
export PYTHONPATH="{python_dir}/lib/$PYVER/site-packages:$PYTHONPATH"
exec python3 -m azure.cli "$@"
"#,
                name = component.name,
                python_dir = python_dir.display(),
            )
        }
        "oci" => {
            format!(
                r#"#!/bin/bash
# Wrapper script for {name} CLI
# Auto-generated by RustConn

export PYTHONUSERBASE="{python_dir}"
PYVER=$(python3 -c "import sys; print(f'python{{sys.version_info.major}}.{{sys.version_info.minor}}')" 2>/dev/null || echo "python3")
export PYTHONPATH="{python_dir}/lib/$PYVER/site-packages:$PYTHONPATH"
exec python3 -c "from oci_cli.cli import cli; cli()" "$@"
"#,
                name = component.name,
                python_dir = python_dir.display(),
            )
        }
        "session-manager-plugin" => {
            format!(
                r#"#!/bin/bash
# Wrapper script for {name}
# Auto-generated by RustConn

export PYTHONUSERBASE="{python_dir}"
PYVER=$(python3 -c "import sys; print(f'python{{sys.version_info.major}}.{{sys.version_info.minor}}')" 2>/dev/null || echo "python3")
export PYTHONPATH="{python_dir}/lib/$PYVER/site-packages:$PYTHONPATH"
exec python3 -c "from ssm_session_client.main import main; main()" "$@"
"#,
                name = component.name,
                python_dir = python_dir.display(),
            )
        }
        _ => {
            return Err(CliDownloadError::ExtractionFailed(format!(
                "Unknown pip component: {}",
                component.id
            )));
        }
    };

    tokio::fs::write(&binary_path, script_content).await?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(&binary_path).await?.permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(&binary_path, perms).await?;
    }

    tracing::info!("Created wrapper script at {:?}", binary_path);

    // Verify the script works by running --version
    let mut test_cmd = tokio::process::Command::new(&binary_path);
    test_cmd.arg("--version");

    if crate::flatpak::is_flatpak() {
        if component.id == "az" {
            if let Some(dir) = crate::flatpak::get_flatpak_azure_config_dir() {
                test_cmd.env("AZURE_CONFIG_DIR", &dir);
            }
        } else if component.id == "gcloud" {
            if let Some(dir) = crate::flatpak::get_flatpak_gcloud_config_dir() {
                test_cmd.env("CLOUDSDK_CONFIG", &dir);
            }
        } else if component.id == "oci"
            && let Some(dir) = crate::flatpak::get_flatpak_oci_config_dir()
        {
            test_cmd.env("OCI_CLI_CONFIG_FILE", dir.join("config"));
        }
    }

    let test_output = test_cmd.output().await;

    match test_output {
        Ok(output) if output.status.success() => {
            tracing::info!(
                "{} wrapper script verified successfully",
                component.binary_name
            );
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(
                "{} wrapper script test returned non-zero: {}",
                component.binary_name,
                stderr
            );
        }
        Err(e) => {
            tracing::warn!(
                "{} wrapper script test failed: {}",
                component.binary_name,
                e
            );
        }
    }

    Ok(binary_path)
}
