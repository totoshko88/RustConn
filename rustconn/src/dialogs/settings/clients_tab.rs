//! Clients detection tab using libadwaita components
//!
//! Client detection is performed asynchronously to avoid blocking the UI thread.

use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Label, Spinner};
use libadwaita as adw;
use rustconn_core::protocol::ClientDetectionResult;
use std::path::PathBuf;
use std::rc::Rc;

use crate::i18n::i18n;

/// Client detection info for async loading
#[derive(Clone)]
struct ClientInfo {
    title: String,
    name: String,
    installed: bool,
    version: Option<String>,
    path: Option<String>,
    install_hint: String,
    /// For protocols with embedded support, show as available even without external client
    has_embedded: bool,
    /// Name of the embedded implementation
    embedded_name: Option<String>,
}

/// Creates the clients detection page using AdwPreferencesPage
/// Client detection is performed asynchronously after the page is shown.
pub fn create_clients_page() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title(i18n("Clients"))
        .icon_name("preferences-system-symbolic")
        .build();

    // === Core Clients Group ===
    let core_group = adw::PreferencesGroup::builder()
        .title(i18n("Core Clients"))
        .description(i18n("Essential connection clients"))
        .build();

    // Add placeholder rows with spinners
    let ssh_row = create_loading_row("SSH Client");
    let rdp_row = create_loading_row("RDP Client");
    let vnc_row = create_loading_row("VNC Client");
    let spice_row = create_loading_row("SPICE Client");

    core_group.add(&ssh_row);
    core_group.add(&rdp_row);
    core_group.add(&vnc_row);
    core_group.add(&spice_row);

    page.add(&core_group);

    // === Zero Trust Clients Group ===
    let zerotrust_group = adw::PreferencesGroup::builder()
        .title(i18n("Zero Trust Clients"))
        .description(i18n("Cloud provider CLI tools"))
        .build();

    let zerotrust_names = [
        "AWS CLI",
        "AWS SSM Plugin",
        "Google Cloud CLI",
        "Azure CLI",
        "OCI CLI",
        "Cloudflare CLI",
        "Teleport CLI",
        "Tailscale CLI",
        "Boundary CLI",
    ];

    let mut zerotrust_rows = Vec::new();
    for name in &zerotrust_names {
        let row = create_loading_row(name);
        zerotrust_group.add(&row);
        zerotrust_rows.push(row);
    }

    page.add(&zerotrust_group);

    // Schedule async detection
    let core_group_clone = core_group.clone();
    let zerotrust_group_clone = zerotrust_group.clone();
    let ssh_row_clone = ssh_row.clone();
    let rdp_row_clone = rdp_row.clone();
    let vnc_row_clone = vnc_row.clone();
    let spice_row_clone = spice_row.clone();
    let zerotrust_rows = Rc::new(zerotrust_rows);
    let zerotrust_rows_clone = zerotrust_rows.clone();

    // Run detection on a real OS thread so the GTK main loop stays idle
    // and can render frames while detection runs in the background.
    // GTK widgets are not Send, so we use a channel to pass results back.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = detect_all_clients();
        let _ = tx.send(result);
    });

    // Poll the channel from the main thread; GTK widgets stay here.
    glib::idle_add_local(move || match rx.try_recv() {
        Ok((core_clients, zerotrust_clients)) => {
            if core_clients.len() >= 4 {
                update_client_row(&core_group_clone, &ssh_row_clone, &core_clients[0]);
                update_client_row(&core_group_clone, &rdp_row_clone, &core_clients[1]);
                update_client_row(&core_group_clone, &vnc_row_clone, &core_clients[2]);
                update_client_row(&core_group_clone, &spice_row_clone, &core_clients[3]);
            }

            for (i, client) in zerotrust_clients.iter().enumerate() {
                if i < zerotrust_rows_clone.len() {
                    update_client_row(&zerotrust_group_clone, &zerotrust_rows_clone[i], client);
                }
            }
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
    });

    page
}

/// Creates a loading placeholder row with spinner
fn create_loading_row(title: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle("Checking...")
        .build();

    let spinner = Spinner::builder()
        .spinning(true)
        .valign(gtk4::Align::Center)
        .build();
    row.add_prefix(&spinner);

    row
}

/// Updates a row with detected client info
fn update_client_row(group: &adw::PreferencesGroup, row: &adw::ActionRow, client: &ClientInfo) {
    // Remove spinner prefix
    if let Some(prefix) = row.first_child() {
        if let Some(box_widget) = prefix.downcast_ref::<gtk4::Box>() {
            if let Some(first) = box_widget.first_child() {
                if first.downcast_ref::<Spinner>().is_some() {
                    box_widget.remove(&first);
                }
            }
        }
    }

    // Determine subtitle based on embedded support and external client availability
    let subtitle = if client.has_embedded {
        if client.installed {
            // External client found
            client.path.clone().unwrap_or_else(|| client.name.clone())
        } else {
            // No external client, but embedded is available
            format!(
                "Using embedded {} (external not found)",
                client.embedded_name.as_deref().unwrap_or("client")
            )
        }
    } else if client.installed {
        client.path.clone().unwrap_or_else(|| client.name.clone())
    } else {
        client.install_hint.clone()
    };
    row.set_subtitle(&subtitle);

    // Create new row with proper styling (easier than modifying existing)
    let new_row = adw::ActionRow::builder()
        .title(&client.title)
        .subtitle(&subtitle)
        .build();

    // Status icon - for embedded protocols, always show success
    let (icon, css_class) = if client.has_embedded {
        // Embedded support available - always show as available
        if client.installed {
            ("✓", "success") // External client found
        } else {
            ("●", "accent") // Using embedded (neutral/info indicator)
        }
    } else if client.installed {
        ("✓", "success")
    } else {
        ("✗", "error")
    };

    let status_label = Label::builder()
        .label(icon)
        .valign(gtk4::Align::Center)
        .css_classes([css_class])
        .build();
    new_row.add_prefix(&status_label);

    // Version label
    if client.installed {
        if let Some(ref v) = client.version {
            let version_label = Label::builder()
                .label(v)
                .valign(gtk4::Align::Center)
                .css_classes(["dim-label"])
                .build();
            new_row.add_suffix(&version_label);
        }
    } else if client.has_embedded {
        // Show embedded indicator
        let embedded_label = Label::builder()
            .label(i18n("Embedded"))
            .valign(gtk4::Align::Center)
            .css_classes(["dim-label"])
            .build();
        new_row.add_suffix(&embedded_label);
    }

    // Replace old row with new one
    let position = get_row_position(group, row);
    group.remove(row);

    // Insert at correct position
    if let Some(pos) = position {
        insert_row_at_position(group, &new_row, pos);
    } else {
        group.add(&new_row);
    }
}

/// Gets the position of a row in a group
fn get_row_position(group: &adw::PreferencesGroup, target_row: &adw::ActionRow) -> Option<usize> {
    let mut position = 0;
    let mut child = group.first_child();

    while let Some(widget) = child {
        // Skip the group header/title widgets
        if let Some(listbox) = widget.downcast_ref::<gtk4::ListBox>() {
            let mut row_child = listbox.first_child();
            while let Some(row_widget) = row_child {
                if let Some(row) = row_widget.downcast_ref::<adw::ActionRow>() {
                    if row == target_row {
                        return Some(position);
                    }
                    position += 1;
                }
                row_child = row_widget.next_sibling();
            }
        }
        child = widget.next_sibling();
    }
    None
}

/// Inserts a row at a specific position in a group
fn insert_row_at_position(group: &adw::PreferencesGroup, row: &adw::ActionRow, _position: usize) {
    // PreferencesGroup doesn't support insert_at, so we just add
    // The order is maintained by replacing rows in sequence
    group.add(row);
}

/// Detects all clients in a background thread with parallelized CLI checks
fn detect_all_clients() -> (Vec<ClientInfo>, Vec<ClientInfo>) {
    // Run core and zero trust detection in parallel
    std::thread::scope(|s| {
        let core_handle = s.spawn(|| detect_core_clients());
        let zt_handle = s.spawn(|| detect_zerotrust_clients());

        let core_clients = core_handle.join().unwrap_or_default();
        let zerotrust_clients = zt_handle.join().unwrap_or_default();
        (core_clients, zerotrust_clients)
    })
}

/// Detects core protocol clients (SSH, RDP, VNC, SPICE)
fn detect_core_clients() -> Vec<ClientInfo> {
    let detection_result = ClientDetectionResult::detect_all();

    let mut core_clients = Vec::with_capacity(4);

    // SSH - always embedded via VTE terminal
    core_clients.push(ClientInfo {
        title: "SSH Client".to_string(),
        name: detection_result.ssh.name.clone(),
        installed: detection_result.ssh.installed,
        version: detection_result.ssh.version.clone(),
        path: detection_result
            .ssh
            .path
            .as_ref()
            .map(|p| p.display().to_string()),
        install_hint: detection_result
            .ssh
            .install_hint
            .clone()
            .unwrap_or_default(),
        has_embedded: true,
        embedded_name: Some("VTE Terminal".to_string()),
    });

    // RDP - embedded via IronRDP, external fallback to xfreerdp
    core_clients.push(ClientInfo {
        title: "RDP Client".to_string(),
        name: detection_result.rdp.name.clone(),
        installed: detection_result.rdp.installed,
        version: detection_result.rdp.version.clone(),
        path: detection_result
            .rdp
            .path
            .as_ref()
            .map(|p| p.display().to_string()),
        install_hint: "Optional: Install freerdp3-wayland (freerdp) package".to_string(),
        has_embedded: true,
        embedded_name: Some("IronRDP".to_string()),
    });

    // VNC - embedded via vnc-rs, external fallback to vncviewer
    core_clients.push(ClientInfo {
        title: "VNC Client".to_string(),
        name: detection_result.vnc.name.clone(),
        installed: detection_result.vnc.installed,
        version: detection_result.vnc.version.clone(),
        path: detection_result
            .vnc
            .path
            .as_ref()
            .map(|p| p.display().to_string()),
        install_hint: "Optional: Install tigervnc-viewer (tigervnc) package".to_string(),
        has_embedded: true,
        embedded_name: Some("vnc-rs".to_string()),
    });

    // SPICE - external only via remote-viewer
    let spice_installed = std::process::Command::new("which")
        .arg("remote-viewer")
        .output()
        .is_ok_and(|output| output.status.success());

    let spice_path = if spice_installed {
        std::process::Command::new("which")
            .arg("remote-viewer")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
    } else {
        None
    };

    let spice_version = spice_path
        .as_ref()
        .and_then(|p| get_version(std::path::Path::new(p), "--version"));

    core_clients.push(ClientInfo {
        title: "SPICE Client".to_string(),
        name: "remote-viewer".to_string(),
        installed: spice_installed,
        version: spice_version,
        path: spice_path,
        install_hint: "Optional: Install virt-viewer package".to_string(),
        has_embedded: true,
        embedded_name: Some("spice-gtk".to_string()),
    });

    core_clients
}

/// Detects zero trust CLI clients in parallel
fn detect_zerotrust_clients() -> Vec<ClientInfo> {
    let zerotrust_configs: &[(&str, &str, &str, &str)] = &[
        ("AWS CLI", "aws", "--version", "Install awscli package"),
        (
            "AWS SSM Plugin",
            "session-manager-plugin",
            "--version",
            "Required for AWS SSM sessions",
        ),
        (
            "Google Cloud CLI",
            "gcloud",
            "--version",
            "Install google-cloud-cli package",
        ),
        ("Azure CLI", "az", "--version", "Install azure-cli package"),
        ("OCI CLI", "oci", "--version", "Install oci-cli package"),
        (
            "Cloudflare CLI",
            "cloudflared",
            "--version",
            "Install cloudflared package",
        ),
        ("Teleport CLI", "tsh", "version", "Install teleport package"),
        (
            "Tailscale CLI",
            "tailscale",
            "--version",
            "Install tailscale package",
        ),
        ("Boundary CLI", "boundary", "-v", "Install boundary package"),
    ];

    // Run all zero trust detections in parallel.
    // We must collect handles first to spawn all threads before joining.
    #[allow(clippy::needless_collect)]
    std::thread::scope(|s| {
        let handles: Vec<_> = zerotrust_configs
            .iter()
            .map(|(title, command, version_arg, install_hint)| {
                s.spawn(move || {
                    let command_path = find_command(command);
                    let installed = command_path.is_some();
                    let version = command_path
                        .as_ref()
                        .and_then(|p| get_version_with_env(p, version_arg, command));
                    let path_str = command_path.as_ref().map(|p| p.display().to_string());

                    ClientInfo {
                        title: (*title).to_string(),
                        name: (*command).to_string(),
                        installed,
                        version,
                        path: path_str,
                        install_hint: (*install_hint).to_string(),
                        has_embedded: false,
                        embedded_name: None,
                    }
                })
            })
            .collect();

        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| ClientInfo {
                    title: String::new(),
                    name: String::new(),
                    installed: false,
                    version: None,
                    path: None,
                    install_hint: String::new(),
                    has_embedded: false,
                    embedded_name: None,
                })
            })
            .collect()
    })
}

/// Finds a command in PATH, common user directories, or Flatpak CLI directory
fn find_command(command: &str) -> Option<PathBuf> {
    // First check Flatpak CLI directory (for components installed via Flatpak Components dialog)
    if let Some(path) = find_in_flatpak_cli_dir(command) {
        return Some(path);
    }

    // Try standard which
    if let Ok(output) = std::process::Command::new("which").arg(command).output() {
        if output.status.success() {
            if let Ok(path_str) = String::from_utf8(output.stdout) {
                let path = path_str.trim();
                if !path.is_empty() {
                    return Some(PathBuf::from(path));
                }
            }
        }
    }

    // Check common user directories
    if let Some(home) = dirs::home_dir() {
        let user_paths = [
            home.join("bin").join(command),
            home.join(".local/bin").join(command),
            home.join(".cargo/bin").join(command),
        ];

        for path in &user_paths {
            if path.exists() {
                return Some(path.clone());
            }
        }
    }

    None
}

/// Finds a command in Flatpak CLI installation directory
fn find_in_flatpak_cli_dir(command: &str) -> Option<PathBuf> {
    // Get CLI install directory from rustconn_core
    let cli_dir = rustconn_core::cli_download::get_cli_install_dir()?;

    // Check if the component is registered and find its binary
    if let Some(component) = rustconn_core::cli_download::get_component(command) {
        if let Some(path) = component.find_installed_binary() {
            return Some(path);
        }
    }

    // Also search common subdirectories in CLI dir
    // pip-installed CLIs (az, oci) are in python/bin
    // AWS CLI v2 is in aws-cli/bin or aws-cli/v2/current/bin
    // SSM Plugin is in ssm-plugin/usr/local/sessionmanagerplugin/bin
    let search_dirs = [
        cli_dir.join("python/bin"),
        cli_dir.join("aws-cli/bin"),
        cli_dir.join("aws-cli/v2/current/bin"),
        cli_dir.join("ssm-plugin/usr/local/sessionmanagerplugin/bin"),
        cli_dir.join("google-cloud-sdk/bin"),
        cli_dir.join("teleport"),
        cli_dir.join("tailscale"),
        cli_dir.join("cloudflared"),
        cli_dir.join("boundary"),
        cli_dir.join("bitwarden"),
        cli_dir.join("1password"),
        cli_dir.join("tigervnc"),
        cli_dir.join("tigervnc/usr/bin"),
    ];

    for dir in &search_dirs {
        let path = dir.join(command);
        if path.exists() {
            return Some(path);
        }
    }

    // Recursive search in CLI dir (limited depth)
    find_binary_recursive(&cli_dir, command, 5)
}

/// Recursively search for a binary in a directory
fn find_binary_recursive(
    dir: &std::path::Path,
    binary_name: &str,
    max_depth: u32,
) -> Option<PathBuf> {
    if max_depth == 0 || !dir.exists() {
        return None;
    }

    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(name) = path.file_name() {
                if name == binary_name {
                    return Some(path);
                }
            }
        } else if path.is_dir() {
            if let Some(found) = find_binary_recursive(&path, binary_name, max_depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

/// Gets version output from a command and parses it
fn get_version(command_path: &std::path::Path, version_arg: &str) -> Option<String> {
    get_version_with_env(command_path, version_arg, "")
}

/// CLI version check timeout (6 seconds)
///
/// Some CLIs (gcloud, az, oci) load Python runtimes and can take 3-5 seconds.
/// This timeout prevents a single slow CLI from blocking the entire detection.
const VERSION_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(6);

/// Gets version output from a command with proper environment setup and timeout
fn get_version_with_env(
    command_path: &std::path::Path,
    version_arg: &str,
    command_name: &str,
) -> Option<String> {
    // Build command with extended PATH for Flatpak CLI tools
    let extended_path = rustconn_core::cli_download::get_extended_path();

    let mut child = std::process::Command::new(command_path)
        .arg(version_arg)
        .env("PATH", &extended_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if start.elapsed() >= VERSION_CHECK_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Some("installed (timeout)".to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }

    let output = child.wait_with_output().ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let version_str = if stdout.trim().is_empty() {
        stderr.to_string()
    } else {
        stdout.to_string()
    };

    // Use command name if provided, otherwise extract from path
    let cmd_name = if command_name.is_empty() {
        command_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
    } else {
        command_name
    };

    parse_version_output(cmd_name, &version_str)
}

/// Parses version from command output based on command type
fn parse_version_output(command: &str, output: &str) -> Option<String> {
    match command {
        "aws" => {
            // AWS CLI v2 format: "aws-cli/2.22.35 Python/3.13.1 Linux/6.12.10-200.fc41.x86_64 exe/x86_64.fedora.41"
            // Extract version from "aws-cli/X.Y.Z"
            output.lines().next().and_then(|line| {
                for part in line.split_whitespace() {
                    if let Some(version) = part.strip_prefix("aws-cli/") {
                        return Some(version.to_string());
                    }
                }
                None
            })
        }

        "gcloud" => output.lines().find_map(|line| {
            line.strip_prefix("Google Cloud SDK ")
                .map(|v| v.trim().to_string())
        }),

        "az" => {
            // Format: "azure-cli                         2.82.0 *"
            // Find the version number (digit-starting word, not "*")
            output.lines().find_map(|line| {
                let trimmed = line.trim();
                if trimmed.starts_with("azure-cli") {
                    for part in trimmed.split_whitespace() {
                        if part != "azure-cli"
                            && part != "*"
                            && part.chars().next().is_some_and(|c| c.is_ascii_digit())
                        {
                            return Some(part.to_string());
                        }
                    }
                }
                None
            })
        }

        "cloudflared" => output.lines().next().and_then(|line| {
            line.split_whitespace()
                .nth(2)
                .map(|v| v.trim_end_matches(['(', ' ']).to_string())
        }),

        "tsh" => {
            // Format: "Teleport v18.6.5 git:v18.6.5-0-g4bc3277 go1.24.12"
            // Extract version like "v18.6.5"
            output.lines().next().and_then(|line| {
                for part in line.split_whitespace() {
                    if part.starts_with('v')
                        && part.chars().nth(1).is_some_and(|c| c.is_ascii_digit())
                    {
                        return Some(part.to_string());
                    }
                }
                None
            })
        }

        "boundary" => output.lines().find_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("Version Number:") {
                trimmed.split(':').nth(1).map(|s| s.trim().to_string())
            } else {
                None
            }
        }),

        "tailscale" => output
            .lines()
            .next()
            .map(|line| line.trim().to_string())
            .filter(|s| !s.is_empty()),

        "oci" => output
            .lines()
            .next()
            .map(|line| line.trim().to_string())
            .filter(|s| !s.is_empty()),

        "ssm-cli" => {
            // ssm-session-client pip package
            // Format: "ssm-cli X.Y.Z" or just version number
            output.lines().next().map(|line| {
                let trimmed = line.trim();
                // If it starts with "ssm-cli", extract version after it
                if let Some(rest) = trimmed.strip_prefix("ssm-cli") {
                    rest.trim().to_string()
                } else if trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    // If it's a version number (starts with digit), return it
                    trimmed.to_string()
                } else {
                    // Otherwise just indicate it's installed
                    "installed".to_string()
                }
            })
        }

        "session-manager-plugin" => {
            // AWS Session Manager Plugin
            // Format: "The Session Manager plugin was installed successfully. Use the AWS CLI to start a session."
            // or version output like "1.2.650.0"
            output.lines().next().map(|line| {
                let trimmed = line.trim();
                // If it's a version number (starts with digit), return it
                if trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    trimmed.to_string()
                } else if trimmed.contains("installed successfully") {
                    "installed".to_string()
                } else {
                    // Otherwise just indicate it's installed
                    "installed".to_string()
                }
            })
        }

        "remote-viewer" => {
            // Format: "remote-viewer, version 11.0" or localized "remote-viewer, версія 11.0"
            // The version number is always the last word on the first line
            output.lines().next().and_then(|line| {
                let trimmed = line.trim();
                if trimmed.starts_with("remote-viewer") {
                    // Extract the last word which should be the version number
                    trimmed.split_whitespace().last().map(String::from)
                } else {
                    Some(trimmed.to_string())
                }
            })
        }

        _ => output
            .lines()
            .find(|line| !line.trim().is_empty())
            .map(|s| s.trim().to_string()),
    }
}
