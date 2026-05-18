//! Predefined connection templates for common CLI tools.
//!
//! These templates provide quick-start configurations for tools that don't have
//! dedicated protocol support (RustDesk, Docker, IPMI, etc.). Each template
//! includes an emoji icon, a command template with `${variable}` placeholders,
//! and a description.

use crate::models::{
    ConnectionTemplate, GenericZeroTrustConfig, ProtocolConfig, ZeroTrustConfig, ZeroTrustProvider,
    ZeroTrustProviderConfig,
};

/// A predefined template definition (static data, no heap allocation at rest).
#[derive(Debug, Clone)]
pub struct PredefinedTemplate {
    /// Unique identifier for lookup
    pub id: &'static str,
    /// Display name
    pub name: &'static str,
    /// Short description
    pub description: &'static str,
    /// Emoji icon
    pub icon: &'static str,
    /// Command template with ${variable} placeholders
    pub command: &'static str,
    /// Category for grouping in the UI
    pub category: TemplateCategory,
}

/// Categories for predefined templates
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TemplateCategory {
    /// Remote desktop tools (RustDesk, AnyDesk, Remmina)
    RemoteDesktop,
    /// Container runtimes (Docker, Podman, LXC, Incus)
    Container,
    /// Virtualization (libvirt, Proxmox, QEMU)
    Virtualization,
    /// Hardware access (IPMI, Serial, BMC)
    Hardware,
    /// Cloud & zero-trust access (Teleport, Tailscale, WireGuard)
    CloudAccess,
    /// Automation & DevOps (Ansible, Nix, WoL+SSH)
    Automation,
}

impl TemplateCategory {
    /// Returns the display name for this category
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::RemoteDesktop => "Remote Desktop",
            Self::Container => "Containers",
            Self::Virtualization => "Virtualization",
            Self::Hardware => "Hardware",
            Self::CloudAccess => "Cloud Access",
            Self::Automation => "Automation",
        }
    }

    /// Returns all categories in display order
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::RemoteDesktop,
            Self::Container,
            Self::Virtualization,
            Self::Hardware,
            Self::CloudAccess,
            Self::Automation,
        ]
    }
}

/// All predefined templates
pub const PREDEFINED_TEMPLATES: &[PredefinedTemplate] = &[
    // === Remote Desktop ===
    PredefinedTemplate {
        id: "rustdesk",
        name: "RustDesk",
        description: "Remote desktop via RustDesk",
        icon: "🖥️",
        command: "rustdesk --connect ${id}",
        category: TemplateCategory::RemoteDesktop,
    },
    PredefinedTemplate {
        id: "anydesk",
        name: "AnyDesk",
        description: "Remote desktop via AnyDesk",
        icon: "🔴",
        command: "anydesk ${address}",
        category: TemplateCategory::RemoteDesktop,
    },
    PredefinedTemplate {
        id: "remmina",
        name: "Remmina",
        description: "Open Remmina connection file",
        icon: "🌐",
        command: "remmina -c ${file}",
        category: TemplateCategory::RemoteDesktop,
    },
    // === Containers ===
    PredefinedTemplate {
        id: "docker-exec",
        name: "Docker",
        description: "Shell into Docker container",
        icon: "🐳",
        command: "docker exec -it ${container} /bin/bash",
        category: TemplateCategory::Container,
    },
    PredefinedTemplate {
        id: "podman-exec",
        name: "Podman",
        description: "Shell into Podman container",
        icon: "🦭",
        command: "podman exec -it ${container} /bin/sh",
        category: TemplateCategory::Container,
    },
    PredefinedTemplate {
        id: "lxc-exec",
        name: "LXC / LXD",
        description: "Shell into LXC instance",
        icon: "📦",
        command: "lxc exec ${instance} -- /bin/bash",
        category: TemplateCategory::Container,
    },
    PredefinedTemplate {
        id: "incus-exec",
        name: "Incus",
        description: "Shell into Incus instance",
        icon: "🧊",
        command: "incus exec ${instance} -- /bin/bash",
        category: TemplateCategory::Container,
    },
    PredefinedTemplate {
        id: "distrobox",
        name: "Distrobox",
        description: "Enter Distrobox container",
        icon: "🗃️",
        command: "distrobox enter ${name}",
        category: TemplateCategory::Container,
    },
    // === Virtualization ===
    PredefinedTemplate {
        id: "virsh-console",
        name: "Virsh Console",
        description: "Serial console to libvirt VM",
        icon: "🖧",
        command: "virsh console ${domain}",
        category: TemplateCategory::Virtualization,
    },
    PredefinedTemplate {
        id: "proxmox-qm",
        name: "Proxmox VM",
        description: "Terminal to Proxmox QEMU VM",
        icon: "🟠",
        command: "ssh ${node} -- qm terminal ${vmid}",
        category: TemplateCategory::Virtualization,
    },
    PredefinedTemplate {
        id: "proxmox-pct",
        name: "Proxmox CT",
        description: "Enter Proxmox LXC container",
        icon: "🟡",
        command: "ssh ${node} -- pct enter ${ctid}",
        category: TemplateCategory::Virtualization,
    },
    // === Hardware ===
    PredefinedTemplate {
        id: "ipmi-sol",
        name: "IPMI SOL",
        description: "Serial-over-LAN via IPMI",
        icon: "🔌",
        command: "ipmitool -I lanplus -H ${bmc_ip} -U ${user} sol activate",
        category: TemplateCategory::Hardware,
    },
    PredefinedTemplate {
        id: "picocom",
        name: "Picocom",
        description: "Serial port (ESP32, Arduino, etc.)",
        icon: "🔧",
        command: "picocom -b ${baud} /dev/ttyUSB${n}",
        category: TemplateCategory::Hardware,
    },
    PredefinedTemplate {
        id: "redfish",
        name: "Redfish BMC",
        description: "BMC management via Redfish",
        icon: "🐟",
        command: "curl -sk -u ${user}:${pass} https://${bmc_ip}/redfish/v1/Systems/1",
        category: TemplateCategory::Hardware,
    },
    // === Cloud Access ===
    PredefinedTemplate {
        id: "wireguard-ssh",
        name: "WireGuard + SSH",
        description: "Bring up VPN then SSH",
        icon: "🛡️",
        command: "wg-quick up ${interface} && ssh ${user}@${host}",
        category: TemplateCategory::CloudAccess,
    },
    PredefinedTemplate {
        id: "teleport-app",
        name: "Teleport App",
        description: "Access internal app via Teleport",
        icon: "🚀",
        command: "tsh app login ${app} && tsh proxy app ${app}",
        category: TemplateCategory::CloudAccess,
    },
    PredefinedTemplate {
        id: "cockpit",
        name: "Cockpit",
        description: "Web console for Linux servers",
        icon: "🎛️",
        command: "xdg-open https://${host}:9090",
        category: TemplateCategory::CloudAccess,
    },
    // === Automation ===
    PredefinedTemplate {
        id: "ansible-adhoc",
        name: "Ansible",
        description: "Ad-hoc command on remote host",
        icon: "⚙️",
        command: "ansible ${host} -m shell -a '${command}'",
        category: TemplateCategory::Automation,
    },
    PredefinedTemplate {
        id: "wol-ssh",
        name: "WoL + SSH",
        description: "Wake server then connect",
        icon: "⏰",
        command: "wakeonlan ${mac} && sleep ${delay} && ssh ${user}@${host}",
        category: TemplateCategory::Automation,
    },
    PredefinedTemplate {
        id: "nix-remote",
        name: "Nix Remote Build",
        description: "Remote Nix build via SSH",
        icon: "❄️",
        command: "ssh ${builder} -- nix-store --serve",
        category: TemplateCategory::Automation,
    },
];

/// Returns all predefined templates
#[must_use]
pub fn all_predefined_templates() -> &'static [PredefinedTemplate] {
    PREDEFINED_TEMPLATES
}

/// Returns predefined templates filtered by category
#[must_use]
pub fn templates_by_category(category: TemplateCategory) -> Vec<&'static PredefinedTemplate> {
    PREDEFINED_TEMPLATES
        .iter()
        .filter(|t| t.category == category)
        .collect()
}

/// Finds a predefined template by its ID
#[must_use]
pub fn find_predefined_template(id: &str) -> Option<&'static PredefinedTemplate> {
    PREDEFINED_TEMPLATES.iter().find(|t| t.id == id)
}

impl PredefinedTemplate {
    /// Converts this predefined template into a `ConnectionTemplate`
    /// ready to be saved or used directly.
    #[must_use]
    pub fn to_connection_template(&self) -> ConnectionTemplate {
        let generic_config = GenericZeroTrustConfig {
            command_template: self.command.to_string(),
        };
        let zt_config = ZeroTrustConfig {
            provider: ZeroTrustProvider::Generic,
            provider_config: ZeroTrustProviderConfig::Generic(generic_config),
            custom_args: Vec::new(),
            detected_provider: None,
        };
        let protocol_config = ProtocolConfig::ZeroTrust(zt_config);

        ConnectionTemplate::new(self.name.to_string(), protocol_config)
            .with_icon(self.icon)
            .with_description(self.description)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_predefined_templates_have_unique_ids() {
        let ids: Vec<&str> = PREDEFINED_TEMPLATES.iter().map(|t| t.id).collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(ids.len(), unique.len(), "Duplicate template IDs found");
    }

    #[test]
    fn test_all_predefined_templates_have_emoji_icons() {
        for template in PREDEFINED_TEMPLATES {
            assert!(
                !template.icon.is_empty(),
                "Template '{}' has no icon",
                template.id
            );
            // Emoji should be non-ASCII
            assert!(
                template.icon.chars().next().is_some_and(|c| !c.is_ascii()),
                "Template '{}' icon '{}' is not an emoji",
                template.id,
                template.icon
            );
        }
    }

    #[test]
    fn test_all_categories_have_templates() {
        for category in TemplateCategory::all() {
            let templates = templates_by_category(*category);
            assert!(
                !templates.is_empty(),
                "Category '{:?}' has no templates",
                category
            );
        }
    }

    #[test]
    fn test_to_connection_template() {
        let predefined = find_predefined_template("docker-exec").unwrap();
        let template = predefined.to_connection_template();

        assert_eq!(template.name, "Docker");
        assert_eq!(template.icon, Some("\u{1f433}".to_string()));
        assert_eq!(
            template.description,
            Some("Shell into Docker container".to_string())
        );
        assert_eq!(template.protocol, crate::models::ProtocolType::ZeroTrust);
    }

    #[test]
    fn test_find_predefined_template() {
        assert!(find_predefined_template("rustdesk").is_some());
        assert!(find_predefined_template("nonexistent").is_none());
    }
}
