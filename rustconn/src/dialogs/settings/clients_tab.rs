//! Clients detection tab

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Frame, Label, Orientation, ScrolledWindow};
use rustconn_core::protocol::ClientDetectionResult;

/// Creates the clients detection tab
pub fn create_clients_tab() -> ScrolledWindow {
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();

    let main_vbox = GtkBox::new(Orientation::Vertical, 12);
    main_vbox.set_margin_top(12);
    main_vbox.set_margin_bottom(12);
    main_vbox.set_margin_start(12);
    main_vbox.set_margin_end(12);
    main_vbox.set_valign(gtk4::Align::Start);

    // Detect all clients
    let detection_result = ClientDetectionResult::detect_all();

    // SSH Client
    let ssh_frame = Frame::builder()
        .label("SSH Client")
        .margin_bottom(12)
        .build();

    let ssh_vbox = GtkBox::new(Orientation::Vertical, 6);
    ssh_vbox.set_margin_top(6);
    ssh_vbox.set_margin_bottom(6);
    ssh_vbox.set_margin_start(6);
    ssh_vbox.set_margin_end(6);

    let ssh_status = if detection_result.ssh.installed {
        format!("✓ {} detected", detection_result.ssh.name)
    } else {
        "✗ SSH client not found".to_string()
    };

    let ssh_status_label = Label::builder()
        .label(&ssh_status)
        .halign(gtk4::Align::Start)
        .build();

    if detection_result.ssh.installed {
        ssh_status_label.add_css_class("success");
        if let Some(path) = &detection_result.ssh.path {
            let ssh_path_label = Label::builder()
                .label(&format!("Path: {}", path.display()))
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .build();
            ssh_vbox.append(&ssh_path_label);
        }
        if let Some(version) = &detection_result.ssh.version {
            let ssh_version_label = Label::builder()
                .label(&format!("Version: {version}"))
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .build();
            ssh_vbox.append(&ssh_version_label);
        }
    } else {
        ssh_status_label.add_css_class("error");
        if let Some(install_hint) = &detection_result.ssh.install_hint {
            let ssh_help_label = Label::builder()
                .label(install_hint)
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .wrap(true)
                .build();
            ssh_vbox.append(&ssh_help_label);
        }
    }

    ssh_vbox.append(&ssh_status_label);
    ssh_frame.set_child(Some(&ssh_vbox));

    // RDP Client
    let rdp_frame = Frame::builder()
        .label("RDP Client")
        .margin_bottom(12)
        .build();

    let rdp_vbox = GtkBox::new(Orientation::Vertical, 6);
    rdp_vbox.set_margin_top(6);
    rdp_vbox.set_margin_bottom(6);
    rdp_vbox.set_margin_start(6);
    rdp_vbox.set_margin_end(6);

    let rdp_status = if detection_result.rdp.installed {
        format!("✓ {} detected", detection_result.rdp.name)
    } else {
        "✗ RDP client not found".to_string()
    };

    let rdp_status_label = Label::builder()
        .label(&rdp_status)
        .halign(gtk4::Align::Start)
        .build();

    if detection_result.rdp.installed {
        rdp_status_label.add_css_class("success");
        if let Some(path) = &detection_result.rdp.path {
            let rdp_path_label = Label::builder()
                .label(&format!("Path: {}", path.display()))
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .build();
            rdp_vbox.append(&rdp_path_label);
        }
        if let Some(version) = &detection_result.rdp.version {
            let rdp_version_label = Label::builder()
                .label(&format!("Version: {version}"))
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .build();
            rdp_vbox.append(&rdp_version_label);
        }
    } else {
        rdp_status_label.add_css_class("error");
        if let Some(install_hint) = &detection_result.rdp.install_hint {
            let rdp_help_label = Label::builder()
                .label(install_hint)
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .wrap(true)
                .build();
            rdp_vbox.append(&rdp_help_label);
        }
    }

    rdp_vbox.append(&rdp_status_label);
    rdp_frame.set_child(Some(&rdp_vbox));

    // VNC Client
    let vnc_frame = Frame::builder()
        .label("VNC Client")
        .margin_bottom(12)
        .build();

    let vnc_vbox = GtkBox::new(Orientation::Vertical, 6);
    vnc_vbox.set_margin_top(6);
    vnc_vbox.set_margin_bottom(6);
    vnc_vbox.set_margin_start(6);
    vnc_vbox.set_margin_end(6);

    let vnc_status = if detection_result.vnc.installed {
        format!("✓ {} detected", detection_result.vnc.name)
    } else {
        "✗ VNC client not found".to_string()
    };

    let vnc_status_label = Label::builder()
        .label(&vnc_status)
        .halign(gtk4::Align::Start)
        .build();

    if detection_result.vnc.installed {
        vnc_status_label.add_css_class("success");
        if let Some(path) = &detection_result.vnc.path {
            let vnc_path_label = Label::builder()
                .label(&format!("Path: {}", path.display()))
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .build();
            vnc_vbox.append(&vnc_path_label);
        }
        if let Some(version) = &detection_result.vnc.version {
            let vnc_version_label = Label::builder()
                .label(&format!("Version: {version}"))
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .build();
            vnc_vbox.append(&vnc_version_label);
        }
    } else {
        vnc_status_label.add_css_class("error");
        if let Some(install_hint) = &detection_result.vnc.install_hint {
            let vnc_help_label = Label::builder()
                .label(install_hint)
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .wrap(true)
                .build();
            vnc_vbox.append(&vnc_help_label);
        }
    }

    vnc_vbox.append(&vnc_status_label);
    vnc_frame.set_child(Some(&vnc_vbox));

    main_vbox.append(&ssh_frame);
    main_vbox.append(&rdp_frame);
    main_vbox.append(&vnc_frame);

    scrolled.set_child(Some(&main_vbox));
    scrolled
}
