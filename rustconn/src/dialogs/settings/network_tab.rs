//! Global network settings — the outermost tier of bastion inheritance.
//!
//! A connection resolves its bastion from its own `ProxyJump` / Jump Host, then
//! from its group chain, then from here. This tier exists because a group cannot
//! stand in for "everything": there is no single implicit root group, and an
//! ungrouped connection has no chain to walk at all, so nothing set on a group
//! can ever reach it (issue
//! [#301](https://github.com/totoshko88/RustConn/issues/301)).
//!
//! A connection set to *Direct* in its editor stops before both inherited tiers.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{DropDown, StringList};
use libadwaita as adw;
use rustconn_core::config::NetworkSettings;
use rustconn_core::models::{Connection, ProtocolType};
use uuid::Uuid;

use crate::i18n::i18n;

/// Longest jump-host label shown in the dropdown before it is elided.
///
/// Matches the connection and group editors, so the same host reads the same way
/// in all three; without a cap a long `name (host)` pair stretches the dialog.
const MAX_LABEL_CHARS: usize = 50;

/// The Network preferences group and the widgets its values are read from.
pub struct NetworkPageWidgets {
    /// The group to add to a preferences page.
    pub group: adw::PreferencesGroup,
    /// Free-text `ProxyJump`, in OpenSSH syntax.
    pub proxy_jump_row: adw::EntryRow,
    /// Picker for a saved SSH connection to use as the bastion.
    pub jump_host_dropdown: DropDown,
    /// `(connection id, label)` parallel to the dropdown's model; index 0 is
    /// `(None, "(None)")`.
    ///
    /// Filled by [`load_network_settings`] rather than at construction: the
    /// dialog is built before the connection list is handed to it.
    pub jump_host_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
}

/// Builds the Network group with an empty jump-host picker.
#[must_use]
pub fn create_network_group() -> NetworkPageWidgets {
    let group = adw::PreferencesGroup::builder()
        .title(i18n("Network"))
        // One line, deliberately: xgettext reads this file as C, where a
        // backslash-newline splices the literal but keeps the indentation that
        // follows. Rust strips it. The two disagree, so a continued string ends
        // up in the catalogue with whitespace the running program never sends and
        // the lookup misses — the entry is translated and renders in English.
        .description(i18n("Applied to connections that inherit their jump host"))
        .build();

    let jump_host_data: Rc<RefCell<Vec<(Option<Uuid>, String)>>> =
        Rc::new(RefCell::new(vec![(None, i18n("(None)"))]));

    let jump_host_dropdown = DropDown::builder()
        .model(&StringList::new(&[i18n("(None)").as_str()]))
        .valign(gtk4::Align::Center)
        .build();
    // `enable_search(true)` alone shows a dead search box — the dropdown needs
    // an expression to match against (same fix as the connection-editor and
    // group jump-host pickers).
    crate::dialogs::widgets::enable_string_search(&jump_host_dropdown);
    jump_host_dropdown.set_size_request(200, -1);
    jump_host_dropdown.set_hexpand(false);
    jump_host_dropdown.set_tooltip_text(Some(&i18n(
        "A saved connection also carries its port, identity file and its own jump host",
    )));

    let jump_host_row = adw::ActionRow::builder()
        .title(i18n("Global Jump Host"))
        .subtitle(i18n("Reach every inheriting connection through this one"))
        .build();
    jump_host_row.add_suffix(&jump_host_dropdown);
    jump_host_row.set_activatable_widget(Some(&jump_host_dropdown));
    group.add(&jump_host_row);

    let proxy_jump_row = adw::EntryRow::builder()
        .title(i18n("Global ProxyJump"))
        .build();
    // Says "chained", not "used when no Global Jump Host is selected", which is
    // what this said until the two were traced through `resolve_jump_chain`: the
    // free-text value and the picker are resolved independently and both end up
    // in the chain. This one is the hop nearer the target — it is pushed first
    // and `JumpChain::hops` is ordered target-first — so `ssh -J` reaches it
    // *through* the Global Jump Host, which is what "after" means here. Setting
    // both is legitimate, and the old wording made a working configuration look
    // like a mistake.
    // Does not quote the "(None)" label: that label is itself translated, so
    // naming it here would leave the two to drift apart per locale.
    proxy_jump_row.set_tooltip_text(Some(&i18n(
        "OpenSSH syntax, for example user@bastion. Chained after the Global Jump Host when both are set",
    )));
    group.add(&proxy_jump_row);

    NetworkPageWidgets {
        group,
        proxy_jump_row,
        jump_host_dropdown,
        jump_host_data,
    }
}

/// Loads `settings` into the group, rebuilding the jump-host picker from
/// `connections`.
///
/// Only SSH connections are offered: a bastion is reached by `ssh -J`, so an RDP
/// or VNC entry could not serve as one. Matches the pickers in the connection and
/// group editors.
pub fn load_network_settings(
    widgets: &NetworkPageWidgets,
    settings: &NetworkSettings,
    connections: &[Connection],
) {
    let mut data: Vec<(Option<Uuid>, String)> = vec![(None, i18n("(None)"))];

    let mut ssh: Vec<&Connection> = connections
        .iter()
        .filter(|c| c.protocol == ProtocolType::Ssh)
        .collect();
    ssh.sort_by_key(|c| c.name.to_lowercase());

    for conn in ssh {
        let label = if conn.name == conn.host {
            conn.name.clone()
        } else {
            format!("{} ({})", conn.name, conn.host)
        };
        let label = if label.chars().count() > MAX_LABEL_CHARS {
            let truncated: String = label.chars().take(MAX_LABEL_CHARS - 1).collect();
            format!("{truncated}\u{2026}")
        } else {
            label
        };
        data.push((Some(conn.id), label));
    }

    let labels: Vec<&str> = data.iter().map(|(_, label)| label.as_str()).collect();
    widgets
        .jump_host_dropdown
        .set_model(Some(&StringList::new(&labels)));

    // A stored jump host whose connection has since been deleted falls back to
    // (None) rather than silently selecting whatever now sits at that index.
    let selected = settings
        .jump_host_id
        .and_then(|id| data.iter().position(|(entry, _)| *entry == Some(id)))
        .unwrap_or(0);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "index is bounded by the connection count, which fits u32"
    )]
    widgets.jump_host_dropdown.set_selected(selected as u32);

    widgets
        .proxy_jump_row
        .set_text(settings.proxy_jump.as_deref().unwrap_or_default());

    *widgets.jump_host_data.borrow_mut() = data;
}

/// Reads the group's widgets back into a [`NetworkSettings`].
#[must_use]
pub fn collect_network_settings(
    proxy_jump_row: &adw::EntryRow,
    jump_host_dropdown: &DropDown,
    jump_host_data: &Rc<RefCell<Vec<(Option<Uuid>, String)>>>,
) -> NetworkSettings {
    let text = proxy_jump_row.text();
    let trimmed = text.trim();
    NetworkSettings {
        // A blank entry means "no global bastion", not `ssh -J ""`.
        proxy_jump: if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        },
        jump_host_id: jump_host_data
            .borrow()
            .get(jump_host_dropdown.selected() as usize)
            .and_then(|(id, _)| *id),
    }
}
