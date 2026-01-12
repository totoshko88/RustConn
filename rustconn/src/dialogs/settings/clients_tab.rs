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

/// Client detection info for async loading
#[derive(Clone)]
struct ClientInfo {
    title: String,
    name: String,
    installed: bool,
    version: Option<String>,
    path: Option<String>,
    install_hint: String,
}

/// Creates the clients detection page using AdwPreferencesPage
/// Client detection is performed asynchronously after the page is shown.
pub fn create_clients_page() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Clients")
        .icon_name("preferences-system-symbolic")
        .build();

    // === Core Clients Group ===
    let core_group = adw::PreferencesGroup::builder()
        .title("Core Clients")
        .description("Essential connection clients")
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
        .title("Zero Trust Clients")
        .description("Cloud provider CLI tools")
        .build();

    let zerotrust_names = [
        "AWS CLI (SSM)",
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

    glib::spawn_future_local(async move {
        // Run detection in a thread pool to avoid blocking
        let (core_clients, zerotrust_clients) =
            glib::spawn_future(async move { detect_all_clients() })
                .await
                .unwrap_or_else(|_| (Vec::new(), Vec::new()));

        // Update core clients
        if core_clients.len() >= 4 {
            update_client_row(&core_group_clone, &ssh_row_clone, &core_clients[0]);
            update_client_row(&core_group_clone, &rdp_row_clone, &core_clients[1]);
            update_client_row(&core_group_clone, &vnc_row_clone, &core_clients[2]);
            update_client_row(&core_group_clone, &spice_row_clone, &core_clients[3]);
        }

        // Update zero trust clients
        for (i, client) in zerotrust_clients.iter().enumerate() {
            if i < zerotrust_rows_clone.len() {
                update_client_row(&zerotrust_group_clone, &zerotrust_rows_clone[i], client);
            }
        }
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

    // Update subtitle
    let subtitle = if client.installed {
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

    // Status icon
    let status_label = Label::builder()
        .label(if client.installed { "✓" } else { "✗" })
        .valign(gtk4::Align::Center)
        .css_classes([if client.installed { "success" } else { "error" }])
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

/// Detects all clients in a background thread
fn detect_all_clients() -> (Vec<ClientInfo>, Vec<ClientInfo>) {
    let mut core_clients = Vec::new();
    let mut zerotrust_clients = Vec::new();

    // Detect core clients
    let detection_result = ClientDetectionResult::detect_all();

    // SSH
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
    });

    // RDP
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
        install_hint: detection_result
            .rdp
            .install_hint
            .clone()
            .unwrap_or_default(),
    });

    // VNC
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
        install_hint: detection_result
            .vnc
            .install_hint
            .clone()
            .unwrap_or_default(),
    });

    // SPICE
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
        install_hint: "Install virt-viewer package".to_string(),
    });

    // Detect zero trust clients
    let zerotrust_configs = [
        (
            "AWS CLI (SSM)",
            "aws",
            "--version",
            "Install awscli package",
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
        (
            "Teleport CLI",
            "teleport",
            "version",
            "Install teleport package",
        ),
        (
            "Tailscale CLI",
            "tailscale",
            "--version",
            "Install tailscale package",
        ),
        ("Boundary CLI", "boundary", "-v", "Install boundary package"),
    ];

    for (title, command, version_arg, install_hint) in &zerotrust_configs {
        let command_path = find_command(command);
        let installed = command_path.is_some();
        let version = command_path
            .as_ref()
            .and_then(|p| get_version(p, version_arg));
        let path_str = command_path.as_ref().map(|p| p.display().to_string());

        zerotrust_clients.push(ClientInfo {
            title: (*title).to_string(),
            name: (*command).to_string(),
            installed,
            version,
            path: path_str,
            install_hint: (*install_hint).to_string(),
        });
    }

    (core_clients, zerotrust_clients)
}

/// Finds a command in PATH or common user directories
fn find_command(command: &str) -> Option<PathBuf> {
    // First try standard which
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

/// Gets version output from a command and parses it
fn get_version(command_path: &std::path::Path, version_arg: &str) -> Option<String> {
    let output = std::process::Command::new(command_path)
        .arg(version_arg)
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let version_str = if stdout.trim().is_empty() {
        stderr.to_string()
    } else {
        stdout.to_string()
    };

    let cmd_name = command_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    parse_version_output(cmd_name, &version_str)
}

/// Parses version from command output based on command type
fn parse_version_output(command: &str, output: &str) -> Option<String> {
    match command {
        "aws" => output
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().next())
            .and_then(|part| part.strip_prefix("aws-cli/"))
            .map(String::from),

        "gcloud" => output.lines().find_map(|line| {
            line.strip_prefix("Google Cloud SDK ")
                .map(|v| v.trim().to_string())
        }),

        "az" => output.lines().find_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("azure-cli") {
                trimmed.split_whitespace().last().map(String::from)
            } else {
                None
            }
        }),

        "cloudflared" => output.lines().next().and_then(|line| {
            line.split_whitespace()
                .nth(2)
                .map(|v| v.trim_end_matches(['(', ' ']).to_string())
        }),

        "teleport" => output.lines().next().and_then(|line| {
            line.split_whitespace()
                .nth(1)
                .map(|v| v.trim_start_matches('v').to_string())
        }),

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
