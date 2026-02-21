//! Connection statistics dialog
//!
//! This module provides a dialog for viewing detailed connection statistics.
//! Migrated to libadwaita components for GNOME HIG compliance.

use crate::i18n::i18n;
use adw::prelude::*;
use gtk4::Box as GtkBox;
use gtk4::prelude::*;
use gtk4::{Label, Orientation, ScrolledWindow};
use libadwaita as adw;
use rustconn_core::models::ConnectionStatistics;
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

/// Connection statistics dialog
pub struct StatisticsDialog {
    window: adw::Window,
    content_box: GtkBox,
    on_clear: Rc<RefCell<Option<Box<dyn Fn() + 'static>>>>,
}

impl StatisticsDialog {
    /// Creates a new statistics dialog
    #[must_use]
    pub fn new(parent: Option<&impl IsA<gtk4::Window>>) -> Self {
        let window = adw::Window::builder()
            .title(i18n("Connection Statistics"))
            .default_width(500)
            .default_height(400)
            .modal(true)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        window.set_size_request(320, 280);

        // Header bar (GNOME HIG)
        let (header, close_btn, reset_btn) =
            crate::dialogs::widgets::dialog_header("Close", "Reset");
        reset_btn.remove_css_class("suggested-action");
        reset_btn.add_css_class("destructive-action");

        // Close button handler
        let window_clone = window.clone();
        close_btn.connect_clicked(move |_| {
            window_clone.close();
        });

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
        window.set_content(Some(&toolbar_view));

        let on_clear: Rc<RefCell<Option<Box<dyn Fn() + 'static>>>> = Rc::new(RefCell::new(None));

        // Reset button handler
        let on_clear_clone = on_clear.clone();
        let window_clone = window.clone();
        reset_btn.connect_clicked(move |_| {
            if let Some(ref callback) = *on_clear_clone.borrow() {
                callback();
            }
            window_clone.close();
        });

        Self {
            window,
            content_box,
            on_clear,
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
            .label(&stats.total_connections.to_string())
            .css_classes(["dim-label"])
            .build();
        total_row.add_suffix(&total_label);
        stats_group.add(&total_row);

        let success_row = adw::ActionRow::builder().title(i18n("Successful")).build();
        let success_label = Label::builder()
            .label(&stats.successful_connections.to_string())
            .css_classes(["success"])
            .build();
        success_row.add_suffix(&success_label);
        stats_group.add(&success_row);

        let failed_row = adw::ActionRow::builder().title(i18n("Failed")).build();
        let failed_label = Label::builder()
            .label(&stats.failed_connections.to_string())
            .css_classes(["error"])
            .build();
        failed_row.add_suffix(&failed_label);
        stats_group.add(&failed_row);

        let rate_row = adw::ActionRow::builder()
            .title(i18n("Success rate"))
            .build();
        let rate_label = Label::builder()
            .label(&format!("{:.1}%", stats.success_rate()))
            .css_classes(["dim-label"])
            .build();
        rate_row.add_suffix(&rate_label);
        stats_group.add(&rate_row);

        let duration_row = adw::ActionRow::builder()
            .title(i18n("Total time connected"))
            .build();
        let duration_label = Label::builder()
            .label(&ConnectionStatistics::format_duration(
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
                .label(&last.format("%Y-%m-%d %H:%M").to_string())
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
                .label(&ConnectionStatistics::format_duration(avg.num_seconds()))
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
    pub fn set_overview_statistics(&self, stats: &[(String, ConnectionStatistics)]) {
        // Clear existing content
        while let Some(child) = self.content_box.first_child() {
            self.content_box.remove(&child);
        }

        // Calculate totals
        let total_connections: u32 = stats.iter().map(|(_, s)| s.total_connections).sum();
        let total_successful: u32 = stats.iter().map(|(_, s)| s.successful_connections).sum();
        let total_failed: u32 = stats.iter().map(|(_, s)| s.failed_connections).sum();
        let total_duration: i64 = stats.iter().map(|(_, s)| s.total_duration_seconds).sum();

        // Summary group
        let summary_group = adw::PreferencesGroup::builder()
            .title(i18n("Overview"))
            .description(i18n("All connections summary"))
            .build();

        let total_row = adw::ActionRow::builder()
            .title(i18n("Total sessions"))
            .build();
        let total_label = Label::builder()
            .label(&total_connections.to_string())
            .css_classes(["dim-label"])
            .build();
        total_row.add_suffix(&total_label);
        summary_group.add(&total_row);

        let success_row = adw::ActionRow::builder().title(i18n("Successful")).build();
        let success_label = Label::builder()
            .label(&total_successful.to_string())
            .css_classes(["success"])
            .build();
        success_row.add_suffix(&success_label);
        summary_group.add(&success_row);

        let failed_row = adw::ActionRow::builder().title(i18n("Failed")).build();
        let failed_label = Label::builder()
            .label(&total_failed.to_string())
            .css_classes(["error"])
            .build();
        failed_row.add_suffix(&failed_label);
        summary_group.add(&failed_row);

        let duration_row = adw::ActionRow::builder().title(i18n("Total time")).build();
        let duration_label = Label::builder()
            .label(&ConnectionStatistics::format_duration(total_duration))
            .css_classes(["dim-label"])
            .build();
        duration_row.add_suffix(&duration_label);
        summary_group.add(&duration_row);

        self.content_box.append(&summary_group);

        // Per-connection breakdown
        if !stats.is_empty() {
            let breakdown_group = adw::PreferencesGroup::builder()
                .title(i18n("Per-Connection"))
                .build();

            for (name, stat) in stats {
                let row = adw::ActionRow::builder()
                    .title(name)
                    .subtitle(&format!(
                        "{} sessions • {:.0}% success",
                        stat.total_connections,
                        stat.success_rate()
                    ))
                    .build();

                let duration_label = Label::builder()
                    .label(&ConnectionStatistics::format_duration(
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
        container.set_margin_top(16);

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
        self.window.present();
    }
}

/// Creates statistics for a connection that has no history yet
#[must_use]
pub fn empty_statistics(connection_id: Uuid) -> ConnectionStatistics {
    ConnectionStatistics::new(connection_id)
}
