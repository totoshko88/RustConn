//! Static i18n markers for strings that are passed dynamically at runtime.
//!
//! These strings come from `rustconn-core` (predefined templates, category names)
//! and are wrapped in `i18n()` at the call site in the GUI, but xgettext cannot
//! extract them because the argument is a variable, not a string literal.
//!
//! This file is never executed — it exists solely so that `xgettext --keyword=i18n`
//! picks up these strings during POT generation.
//!
//! Keep in sync with:
//! - `rustconn-core/src/template/predefined.rs` (descriptions + category names)

#![allow(
    dead_code,
    unreachable_code,
    reason = "module-wide override for legacy code; refactored case by case"
)]

fn _never_called() {
    return;

    // === Template category display names ===
    crate::i18n::i18n("Remote Desktop");
    crate::i18n::i18n("Containers");
    crate::i18n::i18n("Virtualization");
    crate::i18n::i18n("Hardware");
    crate::i18n::i18n("Cloud Access");
    crate::i18n::i18n("Automation");

    // === Predefined template descriptions ===
    crate::i18n::i18n("Remote desktop via RustDesk");
    crate::i18n::i18n("Remote desktop via AnyDesk");
    crate::i18n::i18n("Open Remmina connection file");
    crate::i18n::i18n("Shell into Docker container");
    crate::i18n::i18n("Shell into Podman container");
    crate::i18n::i18n("Shell into LXC instance");
    crate::i18n::i18n("Shell into Incus instance");
    crate::i18n::i18n("Enter Distrobox container");
    crate::i18n::i18n("Serial console to libvirt VM");
    crate::i18n::i18n("Terminal to Proxmox QEMU VM");
    crate::i18n::i18n("Enter Proxmox LXC container");
    crate::i18n::i18n("Serial-over-LAN via IPMI");
    crate::i18n::i18n("Serial port (ESP32, Arduino, etc.)");
    crate::i18n::i18n("BMC management via Redfish");
    crate::i18n::i18n("Bring up VPN then SSH");
    crate::i18n::i18n("Access internal app via Teleport");
    crate::i18n::i18n("Web console for Linux servers");
    crate::i18n::i18n("Ad-hoc command on remote host");
    crate::i18n::i18n("Wake server then connect");
    crate::i18n::i18n("Remote Nix build via SSH");
}
