//! Log viewer dialog for browsing and viewing session logs
//!
//! Provides a GTK4 dialog for browsing log files and viewing their contents.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, Label, ListBox, ListBoxRow, Orientation, ScrolledWindow, TextView,
};
use libadwaita as adw;

use crate::i18n::i18n;

/// Log viewer dialog for browsing and viewing session logs
pub struct LogViewerDialog {
    dialog: adw::Dialog,
    log_list: ListBox,
    log_content: TextView,
    log_dir: PathBuf,
    selected_file: Rc<RefCell<Option<PathBuf>>>,
    /// Maps row index to file path
    file_paths: Rc<RefCell<Vec<PathBuf>>>,
    parent: Option<gtk4::Widget>,
}

impl LogViewerDialog {
    /// Creates a new log viewer dialog for the given log directory.
    ///
    /// `log_dir` is shown as the dialog subtitle: knowing where the files live
    /// is half of what the feature is for (issue #247).
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>, log_dir: PathBuf) -> Self {
        let dialog = adw::Dialog::builder()
            .title(i18n("Session Logs"))
            .content_width(600)
            .content_height(500)
            .build();

        // Create UI components
        let (toolbar_view, split, refresh_btn) = Self::create_header_and_layout(&log_dir);
        dialog.set_child(Some(&toolbar_view));

        let (log_list, list_scrolled) = Self::create_log_list();
        let (log_content, content_scrolled) = Self::create_content_view();

        Self::assemble_split_layout(&split, list_scrolled, content_scrolled);

        // Below this width the two panes cannot both be useful, so the list
        // becomes an overlay reachable from the header toggle. Same mechanism as
        // the main window's adaptive tiers.
        let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            500.0,
            adw::LengthUnit::Sp,
        ));
        breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
        dialog.add_breakpoint(breakpoint);

        let selected_file: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
        let file_paths: Rc<RefCell<Vec<PathBuf>>> = Rc::new(RefCell::new(Vec::new()));

        let stored_parent: Option<gtk4::Widget> =
            parent.map(|p| p.clone().upcast::<gtk4::Widget>());

        let viewer = Self {
            dialog,
            log_list,
            log_content,
            log_dir,
            selected_file,
            file_paths,
            parent: stored_parent,
        };

        // Connect refresh button
        let log_list_clone = viewer.log_list.clone();
        let log_dir_clone = viewer.log_dir.clone();
        let file_paths_clone = viewer.file_paths.clone();
        refresh_btn.connect_clicked(move |_| {
            Self::populate_log_list_static(&log_list_clone, &log_dir_clone, &file_paths_clone);
        });

        // Connect list selection
        let content_clone = viewer.log_content.clone();
        let selected_clone = viewer.selected_file.clone();
        let file_paths_for_select = viewer.file_paths.clone();
        let split_for_select = split.clone();
        viewer.log_list.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                let index = row.index();
                if index >= 0 {
                    let paths = file_paths_for_select.borrow();
                    #[expect(
                        clippy::cast_sign_loss,
                        reason = "value is non-negative by construction in this code path"
                    )]
                    if let Some(path) = paths.get(index as usize) {
                        *selected_clone.borrow_mut() = Some(path.clone());
                        Self::load_log_content(&content_clone, path);
                        // While collapsed the list is an overlay covering the
                        // content, so picking a file has to dismiss it or the log
                        // just opened stays hidden behind it.
                        if split_for_select.is_collapsed() {
                            split_for_select.set_show_sidebar(false);
                        }
                    }
                }
            }
        });

        // Initial population
        viewer.populate_log_list();

        viewer
    }

    /// Creates the header bar and main layout components
    ///
    /// The list/content split is an [`adw::OverlaySplitView`] rather than a
    /// `gtk::Paned`: the HIG notes call for the former precisely because it can
    /// collapse, and a fixed `Paned` pinned both panes on screen at any width, so
    /// this dialog could not be made narrow enough to sit beside anything.
    fn create_header_and_layout(
        log_dir: &Path,
    ) -> (adw::ToolbarView, adw::OverlaySplitView, Button) {
        // Header bar with standard window close button (×) and Refresh icon (GNOME HIG)
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&adw::WindowTitle::new(
            &i18n("Session Logs"),
            &log_dir.display().to_string(),
        )));
        let refresh_btn = Button::from_icon_name("view-refresh-symbolic");
        refresh_btn.add_css_class("flat");
        refresh_btn.set_tooltip_text(Some(&i18n("Refresh log list")));
        refresh_btn
            .update_property(&[gtk4::accessible::Property::Label(&i18n("Refresh log list"))]);
        header.pack_start(&refresh_btn);

        let split = adw::OverlaySplitView::builder()
            .min_sidebar_width(200.0)
            .max_sidebar_width(280.0)
            .sidebar_width_fraction(0.35)
            .build();

        // Sidebar toggle, shown only once the split has collapsed — otherwise the
        // list is already on screen and the button would do nothing visible.
        let sidebar_toggle = gtk4::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text(i18n("Show log file list"))
            .build();
        sidebar_toggle.add_css_class("flat");
        sidebar_toggle.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Show log file list",
        ))]);
        split
            .bind_property("show-sidebar", &sidebar_toggle, "active")
            .bidirectional()
            .sync_create()
            .build();
        split
            .bind_property("collapsed", &sidebar_toggle, "visible")
            .sync_create()
            .build();
        header.pack_start(&sidebar_toggle);

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&split));

        (toolbar_view, split, refresh_btn)
    }

    /// Creates the log file list component
    fn create_log_list() -> (ListBox, ScrolledWindow) {
        let list_scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();

        let log_list = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .build();
        log_list.set_placeholder(Some(&Label::new(Some(&i18n("No log files found")))));
        list_scrolled.set_child(Some(&log_list));

        (log_list, list_scrolled)
    }

    /// Creates the log content view component
    fn create_content_view() -> (TextView, ScrolledWindow) {
        let content_scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Automatic)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .hexpand(true)
            .build();

        let log_content = TextView::builder()
            .editable(false)
            .monospace(true)
            .wrap_mode(gtk4::WrapMode::None)
            .build();
        content_scrolled.set_child(Some(&log_content));

        (log_content, content_scrolled)
    }

    /// Assembles the split layout: file list in the sidebar, contents beside it.
    ///
    /// The 12px margins live on the two children. They used to be set on the
    /// `Paned` itself, which an `OverlaySplitView` cannot carry the same way: its
    /// sidebar slides over the content, so a margin on the container would leave
    /// a gap the overlay animates across.
    fn assemble_split_layout(
        split: &adw::OverlaySplitView,
        list_scrolled: ScrolledWindow,
        content_scrolled: ScrolledWindow,
    ) {
        // Sidebar: Log file list
        let left_box = GtkBox::new(Orientation::Vertical, 8);
        left_box.set_margin_top(12);
        left_box.set_margin_bottom(12);
        left_box.set_margin_start(12);
        left_box.set_margin_end(12);
        let list_label = Label::builder()
            .label(i18n("Log Files"))
            .halign(gtk4::Align::Start)
            .css_classes(["heading"])
            .build();
        left_box.append(&list_label);
        left_box.append(&list_scrolled);
        split.set_sidebar(Some(&left_box));

        // Content: Log content viewer
        let right_box = GtkBox::new(Orientation::Vertical, 8);
        right_box.set_margin_top(12);
        right_box.set_margin_bottom(12);
        right_box.set_margin_start(12);
        right_box.set_margin_end(12);
        let content_label = Label::builder()
            .label(i18n("Log Content"))
            .halign(gtk4::Align::Start)
            .css_classes(["heading"])
            .build();
        right_box.append(&content_label);
        right_box.append(&content_scrolled);
        split.set_content(Some(&right_box));
    }

    /// Populates the log file list
    fn populate_log_list(&self) {
        Self::populate_log_list_static(&self.log_list, &self.log_dir, &self.file_paths);
    }

    /// Populates the log list from the given directory (static version for callbacks)
    fn populate_log_list_static(
        log_list: &ListBox,
        log_dir: &Path,
        file_paths: &Rc<RefCell<Vec<PathBuf>>>,
    ) {
        // Clear existing items
        while let Some(row) = log_list.row_at_index(0) {
            log_list.remove(&row);
        }
        file_paths.borrow_mut().clear();

        // Read log directory
        if !log_dir.exists() {
            return;
        }

        let Ok(entries) = fs::read_dir(log_dir) else {
            return;
        };

        // Collect and sort log files by modification time (newest first)
        let mut log_files: Vec<_> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "log"))
            .collect();

        log_files.sort_by(|a, b| {
            let a_time = a.metadata().and_then(|m| m.modified()).ok();
            let b_time = b.metadata().and_then(|m| m.modified()).ok();
            b_time.cmp(&a_time) // Reverse order (newest first)
        });

        // Add rows for each log file
        for entry in log_files {
            let path = entry.path();
            let filename = path.file_name().map_or_else(
                || "Unknown".to_string(),
                |n| n.to_string_lossy().to_string(),
            );

            // Get file size and modification time
            let metadata = entry.metadata().ok();
            let size_str = metadata
                .as_ref()
                .map(|m| Self::format_file_size(m.len()))
                .unwrap_or_default();
            let time_str = metadata
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(Self::format_time)
                .unwrap_or_default();

            let row_box = GtkBox::new(Orientation::Vertical, 2);
            row_box.set_margin_top(6);
            row_box.set_margin_bottom(6);
            row_box.set_margin_start(12);
            row_box.set_margin_end(12);

            let name_label = Label::builder()
                .label(&filename)
                .halign(gtk4::Align::Start)
                .ellipsize(gtk4::pango::EllipsizeMode::Middle)
                .build();

            let info_label = Label::builder()
                .label(format!("{size_str} • {time_str}"))
                .halign(gtk4::Align::Start)
                .css_classes(["dim-label"])
                .build();

            row_box.append(&name_label);
            row_box.append(&info_label);

            let row = ListBoxRow::builder().child(&row_box).build();

            // Store the path in our vector (index matches row index)
            file_paths.borrow_mut().push(path);

            log_list.append(&row);
        }
    }

    /// Loads log content into the text view asynchronously
    ///
    /// Uses `spawn_blocking_with_callback` to avoid blocking the GTK main thread
    /// when reading large log files.
    fn load_log_content(text_view: &TextView, path: &Path) {
        let buffer = text_view.buffer();

        // Show loading indicator
        buffer.set_text(&i18n("Loading..."));

        // Clone path for the background thread
        let path_clone = path.to_path_buf();
        let buffer_clone = buffer.clone();

        // Read file in background thread to avoid blocking UI
        crate::utils::spawn_blocking_with_callback(
            move || fs::read_to_string(&path_clone),
            move |result: Result<String, std::io::Error>| match result {
                Ok(content) => {
                    buffer_clone.set_text(&content);
                }
                Err(e) => {
                    buffer_clone.set_text(&crate::i18n::i18n_f(
                        "Could not read the log file: {}. The file may have been moved or deleted.",
                        &[&e.to_string()],
                    ));
                }
            },
        );
    }

    /// Formats a file size in human-readable format
    fn format_file_size(bytes: u64) -> String {
        const KB: u64 = 1024;
        const MB: u64 = KB * 1024;
        const GB: u64 = MB * 1024;

        if bytes >= GB {
            format!("{:.1} GB", bytes as f64 / GB as f64)
        } else if bytes >= MB {
            format!("{:.1} MB", bytes as f64 / MB as f64)
        } else if bytes >= KB {
            format!("{:.1} KB", bytes as f64 / KB as f64)
        } else {
            format!("{bytes} B")
        }
    }

    /// Formats a system time as a human-readable string
    fn format_time(time: std::time::SystemTime) -> String {
        use chrono::{DateTime, Local};

        let datetime: DateTime<Local> = time.into();
        datetime.format("%Y-%m-%d %H:%M").to_string()
    }

    /// Shows the dialog
    pub fn show(&self) {
        self.dialog
            .present(self.parent.as_ref().map(|w| w as &gtk4::Widget));
    }

    /// Returns a reference to the underlying dialog
    #[must_use]
    pub const fn dialog(&self) -> &adw::Dialog {
        &self.dialog
    }
}
