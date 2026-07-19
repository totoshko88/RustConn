//! Step 3: Review & Confirm page
//!
//! Displays a summary of the tunnel configuration, the full path diagram
//! with status indicators (in edit mode), a monospace SSH command preview,
//! a copy-to-clipboard button with Toast feedback, and a "Create"/"Save" button.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::models::PortForward;

use super::TunnelPathDiagram;
use crate::i18n::i18n;

// ---------------------------------------------------------------------------
// StepReviewPage — the full Step 3 page
// ---------------------------------------------------------------------------

/// Step 3 page: Review & Confirm
///
/// Shows a summary of the tunnel configuration, SSH command preview,
/// and a save/create button. The page receives all data externally via
/// `update_summary()` and `update_preview_command()`.
pub struct StepReviewPage {
    pub page: adw::NavigationPage,
    diagram: TunnelPathDiagram,
    name_row: adw::ActionRow,
    connection_row: adw::ActionRow,
    bastion_row: adw::ActionRow,
    forwards_row: adw::ActionRow,
    auto_start_row: adw::ActionRow,
    auto_reconnect_row: adw::ActionRow,
    command_buffer: gtk4::TextBuffer,
    no_forwards_label: gtk4::Label,
    save_button: gtk4::Button,
    #[expect(
        dead_code,
        reason = "Kept alive for GTK widget lifecycle (ToastOverlay must outlive Toasts)"
    )]
    toast_overlay: adw::ToastOverlay,
    on_save: Rc<RefCell<Option<Box<dyn Fn()>>>>,
}

impl StepReviewPage {
    /// Creates the review & confirm page
    #[must_use]
    pub fn new() -> Self {
        let on_save: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));

        // Main content
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);

        // --- Path diagram ---
        let diagram_group = adw::PreferencesGroup::builder()
            .title(i18n("Tunnel Path"))
            .build();

        let diagram = TunnelPathDiagram::new();
        let diagram_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        diagram_box.append(diagram.widget());
        diagram_group.add(&diagram_box);

        content.append(&diagram_group);

        // --- Summary group ---
        let summary_group = adw::PreferencesGroup::builder()
            .title(i18n("Summary"))
            .build();

        let name_row = adw::ActionRow::builder()
            .title(i18n("Name"))
            .activatable(false)
            .build();
        summary_group.add(&name_row);

        let connection_row = adw::ActionRow::builder()
            .title(i18n("Connection"))
            .activatable(false)
            .build();
        summary_group.add(&connection_row);

        let bastion_row = adw::ActionRow::builder()
            .title(i18n("Jump Host"))
            .activatable(false)
            .visible(false)
            .build();
        summary_group.add(&bastion_row);

        let forwards_row = adw::ActionRow::builder()
            .title(i18n("Forwards"))
            .activatable(false)
            .build();
        summary_group.add(&forwards_row);

        let auto_start_row = adw::ActionRow::builder()
            .title(i18n("Auto-start"))
            .activatable(false)
            .build();
        summary_group.add(&auto_start_row);

        let auto_reconnect_row = adw::ActionRow::builder()
            .title(i18n("Auto-reconnect"))
            .activatable(false)
            .build();
        summary_group.add(&auto_reconnect_row);

        content.append(&summary_group);

        // --- SSH Command preview group ---
        let command_group = adw::PreferencesGroup::builder()
            .title(i18n("SSH Command"))
            .build();

        let command_buffer = gtk4::TextBuffer::new(None::<&gtk4::TextTagTable>);

        let text_view = gtk4::TextView::builder()
            .buffer(&command_buffer)
            .editable(false)
            .cursor_visible(false)
            .monospace(true)
            .wrap_mode(gtk4::WrapMode::WordChar)
            .top_margin(8)
            .bottom_margin(8)
            .left_margin(12)
            .right_margin(12)
            .build();
        text_view.add_css_class("card");

        // Copy button
        let copy_button = gtk4::Button::from_icon_name("edit-copy-symbolic");
        copy_button.add_css_class("flat");
        copy_button.set_valign(gtk4::Align::Start);
        copy_button.set_margin_top(12);
        copy_button.set_tooltip_text(Some(&i18n("Copy SSH command to clipboard")));
        copy_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Copy SSH command to clipboard",
        ))]);

        // Command box: text_view + copy button
        let command_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        command_box.append(&text_view);
        command_box.append(&copy_button);
        text_view.set_hexpand(true);

        command_group.add(&command_box);

        // Info message when no forwards configured
        let no_forwards_label = gtk4::Label::builder()
            .label(i18n("No port forwarding rules configured"))
            .css_classes(["caption", "dim-label"])
            .halign(gtk4::Align::Start)
            .margin_top(6)
            .visible(false)
            .build();
        command_group.add(&no_forwards_label);

        content.append(&command_group);

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

        // Footer with Save/Create button
        let footer = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        footer.set_margin_top(6);
        footer.set_margin_bottom(6);
        footer.set_margin_start(12);
        footer.set_margin_end(12);

        let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        footer.append(&spacer);

        let save_button = gtk4::Button::with_label(&i18n("Create"));
        save_button.add_css_class("suggested-action");
        save_button.set_receives_default(true);
        footer.append(&save_button);

        // Toast overlay wraps the scrolled content
        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&scrolled));

        // Assemble page
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&adw::HeaderBar::new());
        toolbar_view.set_content(Some(&toast_overlay));
        toolbar_view.add_bottom_bar(&footer);

        let page = adw::NavigationPage::builder()
            .title(i18n("Review"))
            .child(&toolbar_view)
            .build();

        // Wire Save button
        let on_save_clone = on_save.clone();
        save_button.connect_clicked(move |_| {
            if let Some(ref cb) = *on_save_clone.borrow() {
                cb();
            }
        });

        // Wire Copy button
        let buffer_c = command_buffer.clone();
        let toast_overlay_c = toast_overlay.clone();
        let text_view_c = text_view.clone();
        copy_button.connect_clicked(move |_| {
            let text = buffer_c.text(&buffer_c.start_iter(), &buffer_c.end_iter(), false);
            let command_text = text.to_string();
            if !command_text.is_empty() {
                text_view_c.clipboard().set_text(&command_text);
                let toast = adw::Toast::new(&i18n("Copied"));
                toast_overlay_c.add_toast(toast);
            }
        });

        Self {
            page,
            diagram,
            name_row,
            connection_row,
            bastion_row,
            forwards_row,
            auto_start_row,
            auto_reconnect_row,
            command_buffer,
            no_forwards_label,
            save_button,
            toast_overlay,
            on_save,
        }
    }

    /// Registers a callback for the "Create"/"Save" button
    pub fn connect_save<F: Fn() + 'static>(&self, f: F) {
        *self.on_save.borrow_mut() = Some(Box::new(f));
    }

    /// Updates the summary section with current tunnel configuration
    pub fn update_summary(
        &self,
        name: &str,
        connection_label: &str,
        bastion_label: Option<&str>,
        forwards: &[PortForward],
        auto_start: bool,
        auto_reconnect: bool,
    ) {
        self.name_row.set_subtitle(name);
        self.connection_row.set_subtitle(connection_label);

        if let Some(bastion) = bastion_label {
            self.bastion_row.set_subtitle(bastion);
            self.bastion_row.set_visible(true);
        } else {
            self.bastion_row.set_visible(false);
        }

        let forwards_text = if forwards.is_empty() {
            i18n("None")
        } else {
            forwards
                .iter()
                .map(PortForward::display_summary)
                .collect::<Vec<_>>()
                .join(", ")
        };
        self.forwards_row.set_subtitle(&forwards_text);

        let yes = i18n("Yes");
        let no = i18n("No");
        self.auto_start_row
            .set_subtitle(if auto_start { &yes } else { &no });
        self.auto_reconnect_row
            .set_subtitle(if auto_reconnect { &yes } else { &no });

        // Show/hide no-forwards info message
        self.no_forwards_label.set_visible(forwards.is_empty());
    }

    /// Updates the SSH command preview text
    pub fn update_preview_command(&self, command: &str) {
        self.command_buffer.set_text(command);
    }

    /// Sets the save button label ("Create" or "Save")
    pub fn set_save_button_label(&self, label: &str) {
        self.save_button.set_label(label);
    }

    /// Returns a reference to the embedded path diagram
    #[must_use]
    pub fn diagram(&self) -> &TunnelPathDiagram {
        &self.diagram
    }

    /// Moves keyboard focus to the save/create button
    pub fn grab_initial_focus(&self) {
        self.save_button.grab_focus();
    }
}

impl Default for StepReviewPage {
    fn default() -> Self {
        Self::new()
    }
}
