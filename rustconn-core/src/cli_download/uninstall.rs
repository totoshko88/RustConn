use super::{
    CliDownloadError, CliDownloadResult, DownloadableComponent, InstallMethod, get_cli_install_dir,
};

/// Uninstall a component
pub(super) async fn uninstall_component_impl(
    component: &DownloadableComponent,
) -> CliDownloadResult<()> {
    if !crate::is_sandboxed() {
        return Err(CliDownloadError::NotSandboxed);
    }

    let cli_dir = get_cli_install_dir().ok_or(CliDownloadError::NotSandboxed)?;
    let install_dir = cli_dir.join(component.install_subdir);

    if install_dir.exists() {
        tokio::fs::remove_dir_all(&install_dir).await?;
    }

    // Custom components may install to additional directories
    match component.id {
        "aws" => {
            let aws_cli_dir = cli_dir.join("aws-cli");
            if aws_cli_dir.exists() {
                tokio::fs::remove_dir_all(&aws_cli_dir).await?;
            }
            let aws_temp_dir = cli_dir.join("aws-cli-temp");
            if aws_temp_dir.exists() {
                tokio::fs::remove_dir_all(&aws_temp_dir).await?;
            }
        }
        "gcloud" => {
            let gcloud_dir = cli_dir.join("google-cloud-sdk");
            if gcloud_dir.exists() {
                tokio::fs::remove_dir_all(&gcloud_dir).await?;
            }
        }
        _ => {}
    }

    // Also clean up pip/python directory for pip-based components
    if component.install_method == InstallMethod::Pip {
        let python_bin = cli_dir
            .join("python")
            .join("bin")
            .join(component.binary_name);
        if python_bin.exists() {
            let _ = tokio::fs::remove_file(&python_bin).await;
        }
    }

    Ok(())
}
