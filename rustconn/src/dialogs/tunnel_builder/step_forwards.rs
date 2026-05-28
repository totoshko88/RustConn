//! Step 2: Port Forwards & Options page
//!
//! Allows the user to configure port forwarding rules (Local, Remote, Dynamic)
//! and tunnel options (auto-start, auto-reconnect). Each rule is displayed as
//! an `adw::ExpanderRow` with dynamic title showing the current configuration.

use crate::i18n::i18n;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::models::{PortForward, PortForwardDirection};
use std::cell::RefCell;
use std::rc::Rc;

use super::TunnelPathDiagram;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of port forwarding rules allowed
const MAX_FORWARDS: usize = 20;

// ---------------------------------------------------------------------------
// ForwardRuleWidgets — widgets for a single forwarding rule
// ---------------------------------------------------------------------------

struct ForwardRuleWidgets {
    expander: adw::ExpanderRow,
    direction_dropdown: gtk4::DropDown,
    local_port_spin: adw::SpinRow,
    remote_host_entry: adw::EntryRow,
    remote_port_spin: adw::SpinRow,
    #[allow(dead_code, reason = "Kept alive for GTK widget lifecycle")]
    local_port_warning: gtk4::Label,
    #[allow(dead_code, reason = "Kept alive for GTK widget lifecycle")]
    remote_host_error: gtk4::Label,
}

impl ForwardRuleWidgets {
    /// Extracts a `PortForward` from the current widget values
    fn to_port_forward(&self) -> PortForward {
        let direction = match self.direction_dropdown.selected() {
            0 => PortForwardDirection::Local,
            1 => PortForwardDirection::Remote,
            2 => PortForwardDirection::Dynamic,
            _ => PortForwardDirection::Local,
        };

        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let local_port = self.local_port_spin.value() as u16;
        let remote_host = self.remote_host_entry.text().trim().to_string();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let remote_port = self.remote_port_spin.value() as u16;

        PortForward {
            direction,
            local_port,
            remote_host,
            remote_port,
        }
    }

    /// Returns true if this rule passes validation
    fn is_valid(&self) -> bool {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let local_port = self.local_port_spin.value() as u16;
        if local_port == 0 {
            return false;
        }

        let direction = self.direction_dropdown.selected();
        // For Local/Remote, remote host is required
        if direction != 2 {
            let remote_host = self.remote_host_entry.text();
            if remote_host.trim().is_empty() {
                return false;
            }
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "value range fits the target type and is non-negative by construction in this code path"
            )]
            let remote_port = self.remote_port_spin.value() as u16;
            if remote_port == 0 {
                return false;
            }
        }

        true
    }
}

// ---------------------------------------------------------------------------
// StepForwardsPage — the full Step 2 page
// ---------------------------------------------------------------------------

/// Step 2 page: Port Forwards & Options
///
/// Allows configuring port forwarding rules and tunnel options.
/// Each rule is an expandable row with direction, ports, and remote host.
pub struct StepForwardsPage {
    pub page: adw::NavigationPage,
    forwards_group: adw::PreferencesGroup,
    add_button: gtk4::Button,
    auto_start_row: adw::SwitchRow,
    auto_reconnect_row: adw::SwitchRow,
    diagram: TunnelPathDiagram,
    rules: Rc<RefCell<Vec<ForwardRuleWidgets>>>,
    on_next: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    /// Bastion host label (set from step 1 wizard state)
    bastion_host: Rc<RefCell<Option<String>>>,
    /// Target host label (the SSH connection host, set from step 1)
    target_host: Rc<RefCell<Option<String>>>,
}

impl StepForwardsPage {
    /// Creates the port forwards & options page
    #[must_use]
    pub fn new() -> Self {
        let on_next: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let rules: Rc<RefCell<Vec<ForwardRuleWidgets>>> = Rc::new(RefCell::new(Vec::new()));
        let bastion_host: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let target_host: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

        // Main content
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // --- Port Forwards group ---
        let forwards_group = adw::PreferencesGroup::builder()
            .title(i18n("Port Forwards"))
            .build();

        // "Add Forward" button as header suffix (GNOME HIG pattern)
        let add_button = gtk4::Button::from_icon_name("list-add-symbolic");
        add_button.add_css_class("flat");
        add_button.set_valign(gtk4::Align::Center);
        add_button.set_tooltip_text(Some(&i18n("Add port forwarding rule")));
        add_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Add port forwarding rule",
        ))]);
        forwards_group.set_header_suffix(Some(&add_button));
        content.append(&forwards_group);

        // --- Options group ---
        let options_group = adw::PreferencesGroup::builder()
            .title(i18n("Options"))
            .build();

        let auto_start_row = adw::SwitchRow::builder()
            .title(i18n("Auto-start on launch"))
            .subtitle(i18n("Start this tunnel when the application launches"))
            .build();
        options_group.add(&auto_start_row);

        let auto_reconnect_row = adw::SwitchRow::builder()
            .title(i18n("Auto-reconnect on failure"))
            .subtitle(i18n("Automatically reconnect when the tunnel drops"))
            .build();
        options_group.add(&auto_reconnect_row);

        content.append(&options_group);

        // --- Path diagram ---
        let diagram_group = adw::PreferencesGroup::builder()
            .title(i18n("Path Preview"))
            .build();

        let diagram = TunnelPathDiagram::new();
        diagram.hide_status();
        let diagram_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        diagram_box.append(diagram.widget());
        diagram_group.add(&diagram_box);

        content.append(&diagram_group);

        // Wrap in clamp
        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .tightening_threshold(400)
            .child(&content)
            .build();

        let scrolled = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .child(&clamp)
            .vexpand(true)
            .build();

        // Footer with Next button
        let footer = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        footer.set_margin_top(6);
        footer.set_margin_bottom(6);
        footer.set_margin_start(12);
        footer.set_margin_end(12);

        let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        footer.append(&spacer);

        let next_button = gtk4::Button::with_label(&i18n("Next"));
        next_button.add_css_class("suggested-action");
        next_button.set_receives_default(true);
        footer.append(&next_button);

        // Assemble page
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&adw::HeaderBar::new());
        toolbar_view.set_content(Some(&scrolled));
        toolbar_view.add_bottom_bar(&footer);

        let page = adw::NavigationPage::builder()
            .title(i18n("Port Forwards"))
            .child(&toolbar_view)
            .build();

        // Wire Next button
        let on_next_clone = on_next.clone();
        next_button.connect_clicked(move |_| {
            if let Some(ref cb) = *on_next_clone.borrow() {
                cb();
            }
        });

        // Wire Add Forward button
        let forwards_group_c = forwards_group.clone();
        let rules_c = rules.clone();
        let add_button_c = add_button.clone();
        let diagram_c = diagram.clone();
        let bastion_host_c = bastion_host.clone();
        let target_host_c = target_host.clone();
        add_button.connect_clicked(move |_| {
            if rules_c.borrow().len() >= MAX_FORWARDS {
                return;
            }
            Self::add_forward_rule(
                &forwards_group_c,
                &rules_c,
                &add_button_c,
                None,
                &diagram_c,
                &bastion_host_c,
                &target_host_c,
            );
        });

        Self {
            page,
            forwards_group,
            add_button,
            auto_start_row,
            auto_reconnect_row,
            diagram,
            rules,
            on_next,
            bastion_host,
            target_host,
        }
    }

    /// Registers a callback for the "Next" button
    pub fn connect_next<F: Fn() + 'static>(&self, f: F) {
        *self.on_next.borrow_mut() = Some(Box::new(f));
    }

    /// Returns all configured port forwards
    #[must_use]
    pub fn forwards(&self) -> Vec<PortForward> {
        self.rules
            .borrow()
            .iter()
            .map(ForwardRuleWidgets::to_port_forward)
            .collect()
    }

    /// Sets the port forwards (replaces all existing rules)
    pub fn set_forwards(&self, forwards: &[PortForward]) {
        // Remove existing rules
        let rules = self.rules.borrow();
        for rule in rules.iter() {
            self.forwards_group.remove(&rule.expander);
        }
        drop(rules);
        self.rules.borrow_mut().clear();

        // Add new rules
        for fwd in forwards {
            Self::add_forward_rule(
                &self.forwards_group,
                &self.rules,
                &self.add_button,
                Some(fwd),
                &self.diagram,
                &self.bastion_host,
                &self.target_host,
            );
        }

        // Update add button visibility
        self.add_button
            .set_visible(self.rules.borrow().len() < MAX_FORWARDS);

        // Update diagram
        self.update_diagram();
    }

    /// Returns whether auto-start is enabled
    #[must_use]
    pub fn auto_start(&self) -> bool {
        self.auto_start_row.is_active()
    }

    /// Returns whether auto-reconnect is enabled
    #[must_use]
    pub fn auto_reconnect(&self) -> bool {
        self.auto_reconnect_row.is_active()
    }

    /// Sets the auto-start toggle
    pub fn set_auto_start(&self, val: bool) {
        self.auto_start_row.set_active(val);
    }

    /// Sets the auto-reconnect toggle
    pub fn set_auto_reconnect(&self, val: bool) {
        self.auto_reconnect_row.set_active(val);
    }

    /// Returns true if all configured forwards are valid
    ///
    /// Note: 0 forwards is valid (the user may not need any).
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.rules.borrow().iter().all(ForwardRuleWidgets::is_valid)
    }

    /// Updates the path diagram based on the first forward rule
    pub fn update_diagram(&self) {
        let rules = self.rules.borrow();
        let bastion = self.bastion_host.borrow();
        let bastion_str = bastion.as_deref();
        let target = self.target_host.borrow();
        let target_str = target.as_deref();
        if let Some(first) = rules.first() {
            let fwd = first.to_port_forward();
            let direction = match fwd.direction {
                PortForwardDirection::Dynamic => Some(PortForwardDirection::Dynamic),
                other => Some(other),
            };
            self.diagram.update(
                Some(fwd.local_port),
                bastion_str,
                target_str,
                Some(fwd.remote_port),
                direction,
            );
        } else {
            self.diagram
                .update(None, bastion_str, target_str, None, None);
        }
    }

    /// Returns a reference to the embedded diagram for external bastion updates
    #[must_use]
    pub fn diagram(&self) -> &TunnelPathDiagram {
        &self.diagram
    }

    /// Sets the bastion host label (called from step 1 when navigating to step 2)
    pub fn set_bastion(&self, bastion: Option<&str>) {
        *self.bastion_host.borrow_mut() = bastion.map(String::from);
        self.update_diagram();
    }

    /// Sets the target host label (the SSH connection host from step 1)
    pub fn set_target_host(&self, host: Option<&str>) {
        *self.target_host.borrow_mut() = host.map(String::from);
        self.update_diagram();
    }

    /// Moves keyboard focus to the first interactive element (Add Forward button)
    pub fn grab_initial_focus(&self) {
        self.add_button.grab_focus();
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Adds a forward rule to the group and wires all signals
    fn add_forward_rule(
        group: &adw::PreferencesGroup,
        rules: &Rc<RefCell<Vec<ForwardRuleWidgets>>>,
        add_button: &gtk4::Button,
        existing: Option<&PortForward>,
        diagram: &TunnelPathDiagram,
        bastion_host: &Rc<RefCell<Option<String>>>,
        target_host: &Rc<RefCell<Option<String>>>,
    ) {
        // Direction dropdown
        let direction_items = [
            i18n("Local (-L)"),
            i18n("Remote (-R)"),
            i18n("Dynamic (-D)"),
        ];
        let direction_string_list = gtk4::StringList::new(
            &direction_items
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        );

        let direction_dropdown = gtk4::DropDown::builder()
            .model(&direction_string_list)
            .valign(gtk4::Align::Center)
            .build();

        // Compute initial summary
        let summary = existing
            .map(PortForward::display_summary)
            .unwrap_or_else(|| i18n("New forward"));

        let expander = adw::ExpanderRow::builder().title(&summary).build();

        // Direction dropdown as suffix
        expander.add_suffix(&direction_dropdown);

        // Delete button
        let delete_btn = gtk4::Button::from_icon_name("edit-delete-symbolic");
        delete_btn.add_css_class("flat");
        delete_btn.add_css_class("destructive-action");
        delete_btn.set_valign(gtk4::Align::Center);
        delete_btn.set_tooltip_text(Some(&i18n("Remove this port forwarding rule")));
        delete_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Remove this port forwarding rule",
        ))]);
        expander.add_suffix(&delete_btn);

        // Local port
        let local_port_spin = adw::SpinRow::builder()
            .title(i18n("Local Port"))
            .adjustment(&gtk4::Adjustment::new(
                8080.0, 1.0, 65535.0, 1.0, 100.0, 0.0,
            ))
            .build();
        expander.add_row(&local_port_spin);

        // Local port warning label (for privileged ports <1024)
        let local_port_warning = gtk4::Label::builder()
            .label(i18n("Privileged port (requires elevated permissions)"))
            .css_classes(["caption", "warning"])
            .halign(gtk4::Align::Start)
            .margin_start(12)
            .visible(false)
            .build();
        expander.add_row(&local_port_warning);

        // Remote host
        let remote_host_entry = adw::EntryRow::builder().title(i18n("Remote Host")).build();
        remote_host_entry.set_text("localhost");
        expander.add_row(&remote_host_entry);

        // Remote host error label
        let remote_host_error = gtk4::Label::builder()
            .label(i18n("Remote host is required"))
            .css_classes(["caption", "error"])
            .halign(gtk4::Align::Start)
            .margin_start(12)
            .visible(false)
            .build();
        expander.add_row(&remote_host_error);

        // Remote port
        let remote_port_spin = adw::SpinRow::builder()
            .title(i18n("Remote Port"))
            .adjustment(&gtk4::Adjustment::new(80.0, 1.0, 65535.0, 1.0, 100.0, 0.0))
            .build();
        expander.add_row(&remote_port_spin);

        // Populate from existing forward
        if let Some(fwd) = existing {
            let dir_idx = match fwd.direction {
                PortForwardDirection::Local => 0,
                PortForwardDirection::Remote => 1,
                PortForwardDirection::Dynamic => 2,
            };
            direction_dropdown.set_selected(dir_idx);
            local_port_spin.set_value(f64::from(fwd.local_port));
            remote_host_entry.set_text(&fwd.remote_host);
            remote_port_spin.set_value(f64::from(fwd.remote_port));

            // Hide remote fields for Dynamic
            if fwd.direction == PortForwardDirection::Dynamic {
                remote_host_entry.set_visible(false);
                remote_host_error.set_visible(false);
                remote_port_spin.set_visible(false);
            }
        }

        // --- Wire direction change: show/hide remote fields ---
        {
            let rh_entry = remote_host_entry.clone();
            let rh_error = remote_host_error.clone();
            let rp_spin = remote_port_spin.clone();
            direction_dropdown.connect_selected_notify(move |dd| {
                let is_dynamic = dd.selected() == 2;
                rh_entry.set_visible(!is_dynamic);
                rh_error.set_visible(false); // reset error on direction change
                rp_spin.set_visible(!is_dynamic);
            });
        }

        // --- Wire title update + diagram refresh ---
        {
            let expander_c = expander.clone();
            let dir_c = direction_dropdown.clone();
            let lp_c = local_port_spin.clone();
            let remote_host_c = remote_host_entry.clone();
            let remote_port_c = remote_port_spin.clone();
            let rules_d = rules.clone();
            let diagram_d = diagram.clone();
            let bastion_d = bastion_host.clone();
            let target_d = target_host.clone();

            let update_title_and_diagram = Rc::new(move || {
                let dir = match dir_c.selected() {
                    0 => "L",
                    1 => "R",
                    2 => "D",
                    _ => "?",
                };
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "value range fits the target type and is non-negative by construction in this code path"
                )]
                let lp = lp_c.value() as u16;
                let rh = remote_host_c.text();
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "value range fits the target type and is non-negative by construction in this code path"
                )]
                let rp = remote_port_c.value() as u16;
                let title = if dir == "D" {
                    format!("D {lp} (SOCKS)")
                } else {
                    format!("{dir} {lp} → {rh}:{rp}")
                };
                expander_c.set_title(&title);

                // Update diagram using target host from step 1
                let rules_ref = rules_d.borrow();
                let bastion_ref = bastion_d.borrow();
                let bastion_str = bastion_ref.as_deref();
                let target_ref = target_d.borrow();
                let target_str = target_ref.as_deref();
                if let Some(first) = rules_ref.first() {
                    let fwd = first.to_port_forward();
                    let direction = match fwd.direction {
                        PortForwardDirection::Dynamic => Some(PortForwardDirection::Dynamic),
                        other => Some(other),
                    };
                    diagram_d.update(
                        Some(fwd.local_port),
                        bastion_str,
                        target_str,
                        Some(fwd.remote_port),
                        direction,
                    );
                } else {
                    diagram_d.update(None, bastion_str, target_str, None, None);
                }
            });

            let u1 = update_title_and_diagram.clone();
            direction_dropdown.connect_selected_notify(move |_| u1());
            let u2 = update_title_and_diagram.clone();
            local_port_spin.connect_changed(move |_| u2());
            let u3 = update_title_and_diagram.clone();
            remote_host_entry.connect_changed(move |_| u3());
            let u4 = update_title_and_diagram;
            remote_port_spin.connect_changed(move |_| u4());
        }

        // --- Wire port validation (warning for <1024) ---
        {
            let warning_label = local_port_warning.clone();
            local_port_spin.connect_changed(move |spin| {
                #[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "value range fits the target type and is non-negative by construction in this code path"
)]
                let port = spin.value() as u16;
                warning_label.set_visible(port > 0 && port < 1024);
            });
        }

        // --- Wire remote host validation ---
        {
            let error_label = remote_host_error.clone();
            let dir_dd = direction_dropdown.clone();
            remote_host_entry.connect_changed(move |entry| {
                // Only validate for Local/Remote
                if dir_dd.selected() != 2 {
                    let text = entry.text();
                    let is_empty = text.trim().is_empty();
                    error_label.set_visible(is_empty);
                    if is_empty {
                        entry.add_css_class("error");
                    } else {
                        entry.remove_css_class("error");
                    }
                }
            });
        }

        // --- Wire delete button ---
        {
            let group_c = group.clone();
            let expander_c = expander.clone();
            let rules_c = rules.clone();
            let add_btn_c = add_button.clone();
            let diagram_del = diagram.clone();
            let bastion_del = bastion_host.clone();
            let target_del = target_host.clone();
            delete_btn.connect_clicked(move |_| {
                group_c.remove(&expander_c);
                rules_c.borrow_mut().retain(|r| r.expander != expander_c);
                add_btn_c.set_visible(rules_c.borrow().len() < MAX_FORWARDS);

                // Refresh diagram after deletion
                let rules_ref = rules_c.borrow();
                let bastion_ref = bastion_del.borrow();
                let bastion_str = bastion_ref.as_deref();
                let target_ref = target_del.borrow();
                let target_str = target_ref.as_deref();
                if let Some(first) = rules_ref.first() {
                    let fwd = first.to_port_forward();
                    let direction = match fwd.direction {
                        PortForwardDirection::Dynamic => Some(PortForwardDirection::Dynamic),
                        other => Some(other),
                    };
                    diagram_del.update(
                        Some(fwd.local_port),
                        bastion_str,
                        target_str,
                        Some(fwd.remote_port),
                        direction,
                    );
                } else {
                    diagram_del.update(None, bastion_str, target_str, None, None);
                }
            });
        }

        // Check initial port warning
        if let Some(fwd) = existing
            && fwd.local_port > 0
            && fwd.local_port < 1024
        {
            local_port_warning.set_visible(true);
        }

        group.add(&expander);

        rules.borrow_mut().push(ForwardRuleWidgets {
            expander,
            direction_dropdown,
            local_port_spin,
            remote_host_entry,
            remote_port_spin,
            local_port_warning,
            remote_host_error,
        });

        add_button.set_visible(rules.borrow().len() < MAX_FORWARDS);
    }
}

impl Default for StepForwardsPage {
    fn default() -> Self {
        Self::new()
    }
}
