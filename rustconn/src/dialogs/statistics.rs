//! Connection statistics dialog
//!
//! This module provides an `adw::Dialog` for viewing detailed connection statistics.
//! Migrated to libadwaita components for GNOME HIG compliance.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, Label, Orientation, ScrolledWindow};
use libadwaita as adw;
use rustconn_core::models::ConnectionStatistics;
use uuid::Uuid;

use crate::i18n::i18n;

/// Connection statistics dialog
pub struct StatisticsDialog {
    dialog: adw::Dialog,
    content_box: GtkBox,
    on_clear: Rc<RefCell<Option<Box<dyn Fn() + 'static>>>>,
    parent: Option<gtk4::Widget>,
}

impl StatisticsDialog {
    /// Creates a new statistics dialog
    #[must_use]
    pub fn new(parent: Option<&impl IsA<gtk4::Window>>) -> Self {
        let dialog = adw::Dialog::builder()
            .title(i18n("Connection Statistics"))
            .content_width(600)
            .content_height(500)
            .build();

        // Header bar with Reset icon button on the left (GNOME HIG)
        let header = adw::HeaderBar::new();
        let reset_btn = Button::from_icon_name("edit-clear-all-symbolic");
        reset_btn.add_css_class("flat");
        reset_btn.set_tooltip_text(Some(&i18n("Reset Statistics")));
        reset_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Reset Statistics"))]);
        header.pack_start(&reset_btn);

        // Scrolled content with clamp
        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .tightening_threshold(400)
            .build();

        let content_box = GtkBox::new(Orientation::Vertical, 12);
        content_box.set_margin_top(12);
        content_box.set_margin_bottom(12);
        content_box.set_margin_start(12);
        content_box.set_margin_end(12);

        clamp.set_child(Some(&content_box));
        scrolled.set_child(Some(&clamp));

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&scrolled));
        dialog.set_child(Some(&toolbar_view));

        let on_clear: Rc<RefCell<Option<Box<dyn Fn() + 'static>>>> = Rc::new(RefCell::new(None));

        // Reset button handler — confirmation dialog before destructive action
        let on_clear_clone = on_clear.clone();
        let dialog_weak = dialog.downgrade();
        reset_btn.connect_clicked(move |_| {
            let Some(dlg) = dialog_weak.upgrade() else {
                return;
            };
            let alert = adw::AlertDialog::builder()
                .heading(i18n("Reset Statistics?"))
                .body(i18n(
                    "All connection statistics will be permanently cleared.",
                ))
                .build();
            alert.add_response("cancel", &i18n("Cancel"));
            alert.add_response("reset", &i18n("Reset"));
            alert.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
            alert.set_default_response(Some("cancel"));
            alert.set_close_response("cancel");

            let on_clear = on_clear_clone.clone();
            let dlg_close = dlg.clone();
            alert.connect_response(None, move |_, response| {
                if response == "reset" {
                    if let Some(ref callback) = *on_clear.borrow() {
                        callback();
                    }
                    dlg_close.close();
                }
            });
            alert.present(Some(&dlg));
        });

        let parent_widget: Option<gtk4::Widget> =
            parent.map(|p| p.as_ref().clone().upcast::<gtk4::Widget>());

        Self {
            dialog,
            content_box,
            on_clear,
            parent: parent_widget,
        }
    }

    /// Connects a callback for when user clears statistics
    pub fn connect_on_clear<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        *self.on_clear.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the statistics to display for a single connection
    pub fn set_connection_statistics(&self, name: &str, stats: &ConnectionStatistics) {
        // Clear existing content
        while let Some(child) = self.content_box.first_child() {
            self.content_box.remove(&child);
        }

        // Empty state — no connections recorded yet
        if stats.total_connections == 0 {
            let status_page = adw::StatusPage::builder()
                .icon_name("chart-line-symbolic")
                .title(i18n("No Statistics Yet"))
                .description(i18n("Statistics will appear after your first connection."))
                .vexpand(true)
                .build();
            self.content_box.append(&status_page);
            return;
        }

        // Connection name header in PreferencesGroup
        let stats_group = adw::PreferencesGroup::builder()
            .title(name)
            .description(i18n("Connection Statistics"))
            .build();

        // Statistics rows
        let total_row = adw::ActionRow::builder()
            .title(i18n("Total connections"))
            .build();
        let total_label = Label::builder()
            .label(stats.total_connections.to_string())
            .css_classes(["dim-label"])
            .build();
        total_row.add_suffix(&total_label);
        stats_group.add(&total_row);

        let success_row = adw::ActionRow::builder().title(i18n("Successful")).build();
        let success_label = Label::builder()
            .label(stats.successful_connections.to_string())
            .css_classes(["success"])
            .build();
        success_row.add_suffix(&success_label);
        stats_group.add(&success_row);

        let failed_row = adw::ActionRow::builder().title(i18n("Failed")).build();
        let failed_label = Label::builder()
            .label(stats.failed_connections.to_string())
            .css_classes(["error"])
            .build();
        failed_row.add_suffix(&failed_label);
        stats_group.add(&failed_row);

        let rate_row = adw::ActionRow::builder()
            .title(i18n("Success rate"))
            .build();
        let rate_label = Label::builder()
            .label(format!("{:.1}%", stats.success_rate()))
            .css_classes(["dim-label"])
            .build();
        rate_row.add_suffix(&rate_label);
        stats_group.add(&rate_row);

        let duration_row = adw::ActionRow::builder()
            .title(i18n("Total time connected"))
            .build();
        let duration_label = Label::builder()
            .label(ConnectionStatistics::format_duration(
                stats.total_duration_seconds,
            ))
            .css_classes(["dim-label"])
            .build();
        duration_row.add_suffix(&duration_label);
        stats_group.add(&duration_row);

        if let Some(last) = &stats.last_connected {
            let last_row = adw::ActionRow::builder()
                .title(i18n("Last connected"))
                .build();
            let last_label = Label::builder()
                .label(last.format("%Y-%m-%d %H:%M").to_string())
                .css_classes(["dim-label"])
                .build();
            last_row.add_suffix(&last_label);
            stats_group.add(&last_row);
        }

        // Average session duration
        let avg = stats.average_duration();
        if avg.num_seconds() > 0 {
            let avg_row = adw::ActionRow::builder()
                .title(i18n("Average session"))
                .build();
            let avg_label = Label::builder()
                .label(ConnectionStatistics::format_duration(avg.num_seconds()))
                .css_classes(["dim-label"])
                .build();
            avg_row.add_suffix(&avg_label);
            stats_group.add(&avg_row);
        }

        self.content_box.append(&stats_group);

        // Success rate visualization
        let rate_box = self.create_success_rate_box(stats);
        self.content_box.append(&rate_box);
    }

    /// Sets statistics for multiple connections (overview)
    pub fn set_overview_statistics(&self, stats: &[(String, ConnectionStatistics, String)]) {
        // Clear existing content
        while let Some(child) = self.content_box.first_child() {
            self.content_box.remove(&child);
        }

        // Calculate totals
        let total_connections: u32 = stats.iter().map(|(_, s, _)| s.total_connections).sum();
        let total_successful: u32 = stats.iter().map(|(_, s, _)| s.successful_connections).sum();
        let total_failed: u32 = stats.iter().map(|(_, s, _)| s.failed_connections).sum();
        let total_duration: i64 = stats.iter().map(|(_, s, _)| s.total_duration_seconds).sum();

        // Summary group
        let summary_group = adw::PreferencesGroup::builder()
            .title(i18n("Overview"))
            .description(i18n("All connections summary"))
            .build();

        let total_row = adw::ActionRow::builder()
            .title(i18n("Total sessions"))
            .build();
        let total_label = Label::builder()
            .label(total_connections.to_string())
            .css_classes(["dim-label"])
            .build();
        total_row.add_suffix(&total_label);
        summary_group.add(&total_row);

        let success_row = adw::ActionRow::builder().title(i18n("Successful")).build();
        let success_label = Label::builder()
            .label(total_successful.to_string())
            .css_classes(["success"])
            .build();
        success_row.add_suffix(&success_label);
        summary_group.add(&success_row);

        let failed_row = adw::ActionRow::builder().title(i18n("Failed")).build();
        let failed_label = Label::builder()
            .label(total_failed.to_string())
            .css_classes(["error"])
            .build();
        failed_row.add_suffix(&failed_label);
        summary_group.add(&failed_row);

        let duration_row = adw::ActionRow::builder().title(i18n("Total time")).build();
        let duration_label = Label::builder()
            .label(ConnectionStatistics::format_duration(total_duration))
            .css_classes(["dim-label"])
            .build();
        duration_row.add_suffix(&duration_label);
        summary_group.add(&duration_row);

        self.content_box.append(&summary_group);

        // Most Used connections (top 3)
        if !stats.is_empty() {
            let mut sorted: Vec<_> = stats.iter().collect();
            sorted.sort_by_key(|b| std::cmp::Reverse(b.1.total_connections));

            let most_used_group = adw::PreferencesGroup::builder()
                .title(i18n("Most Used"))
                .description(i18n("Top connections by usage"))
                .build();

            for (name, stat, protocol) in sorted.iter().take(3) {
                let row = adw::ActionRow::builder()
                    .title(name.as_str())
                    .subtitle(format!(
                        "{} • {} {} • {:.0}%",
                        protocol.to_uppercase(),
                        stat.total_connections,
                        i18n("sessions"),
                        stat.success_rate()
                    ))
                    .build();

                let duration_label = Label::builder()
                    .label(ConnectionStatistics::format_duration(
                        stat.total_duration_seconds,
                    ))
                    .css_classes(["dim-label"])
                    .build();
                row.add_suffix(&duration_label);

                most_used_group.add(&row);
            }

            self.content_box.append(&most_used_group);
        }

        // Protocol Distribution
        if !stats.is_empty() {
            let mut protocol_totals: std::collections::HashMap<String, u32> =
                std::collections::HashMap::new();
            for (_, stat, protocol) in stats {
                *protocol_totals.entry(protocol.to_uppercase()).or_default() +=
                    stat.total_connections;
            }

            let mut protocol_list: Vec<_> = protocol_totals.into_iter().collect();
            protocol_list.sort_by_key(|b| std::cmp::Reverse(b.1));

            let protocol_group = adw::PreferencesGroup::builder()
                .title(i18n("Protocol Distribution"))
                .build();

            for (protocol, count) in &protocol_list {
                let fraction = if total_connections > 0 {
                    f64::from(*count) / f64::from(total_connections)
                } else {
                    0.0
                };

                let row = adw::ActionRow::builder()
                    .title(protocol)
                    .subtitle(format!(
                        "{} {} ({:.0}%)",
                        count,
                        i18n("sessions"),
                        fraction * 100.0
                    ))
                    .build();

                let progress = gtk4::ProgressBar::builder()
                    .fraction(fraction)
                    .valign(gtk4::Align::Center)
                    .width_request(80)
                    .build();
                row.add_suffix(&progress);

                protocol_group.add(&row);
            }

            self.content_box.append(&protocol_group);
        }

        // Per-connection breakdown
        if !stats.is_empty() {
            let breakdown_group = adw::PreferencesGroup::builder()
                .title(i18n("Per-Connection"))
                .build();

            for (name, stat, protocol) in stats {
                let row = adw::ActionRow::builder()
                    .title(name)
                    .subtitle(format!(
                        "{} • {} {} • {:.0}%",
                        protocol.to_uppercase(),
                        stat.total_connections,
                        i18n("sessions"),
                        stat.success_rate()
                    ))
                    .build();

                let duration_label = Label::builder()
                    .label(ConnectionStatistics::format_duration(
                        stat.total_duration_seconds,
                    ))
                    .css_classes(["dim-label"])
                    .build();
                row.add_suffix(&duration_label);

                breakdown_group.add(&row);
            }

            self.content_box.append(&breakdown_group);
        }
    }

    /// Creates a success rate visualization box
    fn create_success_rate_box(&self, stats: &ConnectionStatistics) -> GtkBox {
        let container = GtkBox::new(Orientation::Vertical, 8);
        container.set_margin_top(18);

        let label = Label::builder()
            .label(i18n("Success Rate"))
            .css_classes(["title-4"])
            .halign(gtk4::Align::Start)
            .build();
        container.append(&label);

        // Progress bar for success rate
        let progress = gtk4::ProgressBar::builder()
            .fraction(stats.success_rate() / 100.0)
            .show_text(true)
            .text(format!("{:.1}%", stats.success_rate()))
            .build();

        // Color based on success rate
        if stats.success_rate() >= 90.0 {
            progress.add_css_class("success");
        } else if stats.success_rate() >= 70.0 {
            progress.add_css_class("warning");
        } else {
            progress.add_css_class("error");
        }

        container.append(&progress);
        container
    }

    /// Shows the dialog
    pub fn present(&self) {
        self.dialog
            .present(self.parent.as_ref().map(|w| w as &gtk4::Widget));
    }
}

/// Creates statistics for a connection that has no history yet
#[must_use]
pub fn empty_statistics(connection_id: Uuid) -> ConnectionStatistics {
    ConnectionStatistics::new(connection_id)
}
