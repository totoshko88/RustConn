use std::path::{Path, PathBuf};

use super::{CliDownloadError, CliDownloadResult};

/// Find a binary in a directory, searching recursively if needed
pub(super) fn find_binary_in_dir(dir: &Path, binary_name: &str) -> CliDownloadResult<PathBuf> {
    // First check direct path
    let direct = dir.join(binary_name);
    if direct.exists() {
        return Ok(direct);
    }

    // Check common subdirectories (including SSM plugin path)
    let common_subdirs = [
        "bin",
        "usr/bin",
        "usr/local/bin",
        "usr/local/sessionmanagerplugin/bin", // AWS SSM Plugin
    ];
    for subdir in common_subdirs {
        let path = dir.join(subdir).join(binary_name);
        if path.exists() {
            return Ok(path);
        }
    }

    // Search recursively
    if let Some(found) = find_binary_recursive(dir, binary_name, 5) {
        return Ok(found);
    }

    Err(CliDownloadError::ExtractionFailed(format!(
        "Binary '{}' not found in extracted files",
        binary_name
    )))
}

/// Recursively search for a binary by name in a directory tree.
///
/// Returns the first matching file path, or `None` if not found.
/// `max_depth` limits recursion to avoid traversing very deep trees.
pub fn find_binary_recursive(dir: &Path, binary_name: &str, max_depth: u32) -> Option<PathBuf> {
    if max_depth == 0 || !dir.exists() {
        return None;
    }

    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(name) = path.file_name()
                && name == binary_name
            {
                return Some(path);
            }
        } else if path.is_dir()
            && let Some(found) = find_binary_recursive(&path, binary_name, max_depth - 1)
        {
            return Some(found);
        }
    }
    None
}

pub(super) fn extract_zip(data: &[u8], dest: &Path) -> CliDownloadResult<()> {
    use std::io::Cursor;

    let cursor = Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| CliDownloadError::ExtractionFailed(e.to_string()))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| CliDownloadError::ExtractionFailed(e.to_string()))?;

        // enclosed_name() validates against path traversal (e.g. "../../../etc/passwd")
        let relative = file.enclosed_name().ok_or_else(|| {
            CliDownloadError::ExtractionFailed(format!(
                "zip entry has unsafe path: {:?}",
                file.name()
            ))
        })?;
        let outpath = dest.join(relative);

        if file.is_dir() {
            std::fs::create_dir_all(&outpath)?;
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut outfile = std::fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;

            // Set executable permission on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = file.unix_mode() {
                    std::fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode))?;
                }
            }
        }
    }

    Ok(())
}

/// Extract .deb package (ar archive containing data.tar.gz or data.tar.xz)
pub(super) fn extract_deb(data: &[u8], dest: &Path) -> CliDownloadResult<()> {
    use std::io::{Cursor, Read};

    let cursor = Cursor::new(data);
    let mut archive = ar::Archive::new(cursor);

    // Find and extract data.tar.* from the .deb
    while let Some(entry_result) = archive.next_entry() {
        let mut entry = entry_result
            .map_err(|e| CliDownloadError::ExtractionFailed(format!("ar read error: {e}")))?;

        let name = String::from_utf8_lossy(entry.header().identifier()).to_string();

        if name.starts_with("data.tar") {
            // Read the data archive
            let mut data_archive = Vec::new();
            entry
                .read_to_end(&mut data_archive)
                .map_err(|e| CliDownloadError::ExtractionFailed(format!("read data.tar: {e}")))?;

            // Extract based on compression type
            #[expect(
                clippy::case_sensitive_file_extension_comparisons,
                reason = "extension comes from a controlled allow-list, not user input - case is already normalised upstream"
            )]
            if name.ends_with(".gz") {
                extract_tar_gz(&data_archive, dest)?;
            } else if name.ends_with(".xz") {
                extract_tar_xz(&data_archive, dest)?;
            } else if name.ends_with(".zst") {
                extract_tar_zst(&data_archive, dest)?;
            } else {
                // Uncompressed tar
                extract_tar(&data_archive, dest)?;
            }

            return Ok(());
        }
    }

    Err(CliDownloadError::ExtractionFailed(
        "data.tar not found in .deb package".to_string(),
    ))
}

/// Extract uncompressed tar archive
pub(super) fn extract_tar(data: &[u8], dest: &Path) -> CliDownloadResult<()> {
    use std::io::Cursor;
    use tar::Archive;

    let cursor = Cursor::new(data);
    let mut archive = Archive::new(cursor);

    safe_unpack_tar(&mut archive, dest)
}

/// Safely unpacks a tar archive with manual path traversal validation.
fn safe_unpack_tar<R: std::io::Read>(
    archive: &mut tar::Archive<R>,
    dest: &Path,
) -> CliDownloadResult<()> {
    let entries = archive
        .entries()
        .map_err(|e| CliDownloadError::ExtractionFailed(format!("failed to read tar: {e}")))?;

    for entry in entries {
        let mut entry =
            entry.map_err(|e| CliDownloadError::ExtractionFailed(format!("bad entry: {e}")))?;

        let path = entry
            .path()
            .map_err(|e| CliDownloadError::ExtractionFailed(format!("bad path: {e}")))?;

        // Reject entries with ".." components or absolute paths
        for component in path.components() {
            match component {
                std::path::Component::ParentDir => {
                    return Err(CliDownloadError::ExtractionFailed(format!(
                        "tar entry has unsafe path (..): {}",
                        path.display()
                    )));
                }
                std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                    return Err(CliDownloadError::ExtractionFailed(format!(
                        "tar entry has absolute path: {}",
                        path.display()
                    )));
                }
                _ => {}
            }
        }

        entry
            .unpack_in(dest)
            .map_err(|e| CliDownloadError::ExtractionFailed(format!("unpack failed: {e}")))?;
    }

    Ok(())
}

/// Extract tar.xz archive
pub(super) fn extract_tar_xz(data: &[u8], dest: &Path) -> CliDownloadResult<()> {
    use std::io::Read;

    let temp_file = dest.join("_temp_data.tar.xz");
    std::fs::write(&temp_file, data)?;

    let output = std::process::Command::new("xz")
        .args(["-d", "-k", "-f"])
        .arg(&temp_file)
        .output();

    match output {
        Ok(result) if result.status.success() => {
            let tar_file = dest.join("_temp_data.tar");
            if tar_file.exists() {
                let tar_data = std::fs::read(&tar_file)?;
                let _ = std::fs::remove_file(&tar_file);
                let _ = std::fs::remove_file(&temp_file);
                return extract_tar(&tar_data, dest);
            }
        }
        _ => {}
    }

    let _ = std::fs::remove_file(&temp_file);

    // Fallback: try reading as gzip (some .xz files are actually gzip)
    let cursor = std::io::Cursor::new(data);
    let mut decoder = flate2::read::GzDecoder::new(cursor);
    let mut decompressed = Vec::new();
    if decoder.read_to_end(&mut decompressed).is_ok() {
        return extract_tar(&decompressed, dest);
    }

    Err(CliDownloadError::ExtractionFailed(
        "xz decompression failed - xz command not available".to_string(),
    ))
}

/// Extract tar.zst archive
pub(super) fn extract_tar_zst(data: &[u8], dest: &Path) -> CliDownloadResult<()> {
    let temp_file = dest.join("_temp_data.tar.zst");
    std::fs::write(&temp_file, data)?;

    let output = std::process::Command::new("zstd")
        .args(["-d", "-f"])
        .arg(&temp_file)
        .output();

    match output {
        Ok(result) if result.status.success() => {
            let tar_file = dest.join("_temp_data.tar");
            if tar_file.exists() {
                let tar_data = std::fs::read(&tar_file)?;
                let _ = std::fs::remove_file(&tar_file);
                let _ = std::fs::remove_file(&temp_file);
                return extract_tar(&tar_data, dest);
            }
        }
        _ => {}
    }

    let _ = std::fs::remove_file(&temp_file);

    Err(CliDownloadError::ExtractionFailed(
        "zstd decompression failed - zstd command not available".to_string(),
    ))
}

pub(super) fn extract_tar_gz(data: &[u8], dest: &Path) -> CliDownloadResult<()> {
    use flate2::read::GzDecoder;
    use std::io::Cursor;
    use tar::Archive;

    let cursor = Cursor::new(data);
    let decoder = GzDecoder::new(cursor);
    let mut archive = Archive::new(decoder);

    // First, try to get entries to check the archive structure
    let cursor2 = Cursor::new(data);
    let decoder2 = GzDecoder::new(cursor2);
    let mut archive2 = Archive::new(decoder2);

    // Check if archive has a single top-level directory
    let entries = archive2
        .entries()
        .map_err(|e| CliDownloadError::ExtractionFailed(format!("failed to read archive: {e}")))?;

    let mut top_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();
    for entry in entries {
        let entry =
            entry.map_err(|e| CliDownloadError::ExtractionFailed(format!("bad entry: {e}")))?;
        if let Ok(path) = entry.path()
            && let Some(std::path::Component::Normal(name)) = path.components().next()
        {
            top_dirs.insert(name.to_string_lossy().to_string());
        }
    }

    // Extract to destination with path traversal validation
    safe_unpack_tar(&mut archive, dest)?;

    // If there's exactly one top-level directory, move its contents up
    if top_dirs.len() == 1 {
        let top_dir_name = top_dirs.into_iter().next().unwrap_or_default();
        let top_dir = dest.join(&top_dir_name);
        if top_dir.is_dir() {
            if let Ok(dir_entries) = std::fs::read_dir(&top_dir) {
                for entry in dir_entries.flatten() {
                    let src = entry.path();
                    let file_name = entry.file_name();
                    let target = dest.join(&file_name);
                    if !target.exists()
                        && let Err(e) = std::fs::rename(&src, &target)
                    {
                        tracing::debug!(
                            "Could not move {:?} to {:?}: {}, trying copy",
                            src,
                            target,
                            e
                        );
                        if src.is_dir() {
                            copy_dir_recursive(&src, &target)?;
                        } else {
                            std::fs::copy(&src, &target)?;
                        }
                    }
                }
            }
            let _ = std::fs::remove_dir_all(&top_dir);
        }
    }

    Ok(())
}

/// Extract tar.gz archive preserving directory structure (no flattening)
pub(super) fn extract_tar_gz_preserve(data: &[u8], dest: &Path) -> CliDownloadResult<()> {
    use flate2::read::GzDecoder;
    use std::io::Cursor;
    use tar::Archive;

    let cursor = Cursor::new(data);
    let decoder = GzDecoder::new(cursor);
    let mut archive = Archive::new(decoder);

    safe_unpack_tar(&mut archive, dest)?;

    Ok(())
}

pub(super) fn copy_dir_recursive(src: &Path, dst: &Path) -> CliDownloadResult<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
