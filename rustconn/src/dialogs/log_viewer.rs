//! Log viewer dialog for browsing and viewing session logs
//!
//! Provides a GTK4 dialog for browsing log files and viewing their contents.

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, Label, ListBox, ListBoxRow, Orientation, Paned, ScrolledWindow, TextView,
};
use libadwaita as adw;
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Log viewer dialog for browsing and viewing session logs
pub struct LogViewerDialog {
    window: adw::Window,
    log_list: ListBox,
    log_content: TextView,
    log_dir: PathBuf,
    selected_file: Rc<RefCell<Option<PathBuf>>>,
    /// Maps row index to file path
    file_paths: Rc<RefCell<Vec<PathBuf>>>,
}

impl LogViewerDialog {
    /// Creates a new log viewer dialog
    #[must_use]
    pub fn new(parent: Option<&gtk4::Window>) -> Self {
        let window = adw::Window::builder()
            .title("Session Logs")
            .modal(true)
            .default_width(900)
            .default_height(600)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        // Create UI components
        let (toolbar_view, paned, close_btn, refresh_btn) = Self::create_header_and_layout();
        window.set_content(Some(&toolbar_view));

        let (log_list, list_scrolled) = Self::create_log_list();
        let (log_content, content_scrolled) = Self::create_content_view();

        // Assemble paned layout
        Self::assemble_paned_layout(&paned, list_scrolled, content_scrolled);

        // Get default log directory
        let log_dir = Self::get_default_log_dir();
        let selected_file: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));
        let file_paths: Rc<RefCell<Vec<PathBuf>>> = Rc::new(RefCell::new(Vec::new()));

        // Connect close button
        let window_clone = window.clone();
        close_btn.connect_clicked(move |_| {
            window_clone.close();
        });

        let dialog = Self {
            window,
            log_list,
            log_content,
            log_dir,
            selected_file,
            file_paths,
        };

        // Connect refresh button
        let log_list_clone = dialog.log_list.clone();
        let log_dir_clone = dialog.log_dir.clone();
        let file_paths_clone = dialog.file_paths.clone();
        refresh_btn.connect_clicked(move |_| {
            Self::populate_log_list_static(&log_list_clone, &log_dir_clone, &file_paths_clone);
        });

        // Connect list selection
        let content_clone = dialog.log_content.clone();
        let selected_clone = dialog.selected_file.clone();
        let file_paths_for_select = dialog.file_paths.clone();
        dialog.log_list.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                let index = row.index();
                if index >= 0 {
                    let paths = file_paths_for_select.borrow();
                    #[allow(clippy::cast_sign_loss)]
                    if let Some(path) = paths.get(index as usize) {
                        *selected_clone.borrow_mut() = Some(path.clone());
                        Self::load_log_content(&content_clone, path);
                    }
                }
            }
        });

        // Initial population
        dialog.populate_log_list();

        dialog
    }

    /// Creates the header bar and main layout components
    fn create_header_and_layout() -> (adw::ToolbarView, Paned, Button, Button) {
        let header = adw::HeaderBar::new();
        header.set_show_end_title_buttons(false);
        header.set_show_start_title_buttons(false);

        let close_btn = Button::builder().label("Close").build();
        let refresh_btn = Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Refresh log list")
            .build();
        header.pack_start(&close_btn);
        header.pack_end(&refresh_btn);

        let paned = Paned::new(Orientation::Horizontal);
        paned.set_position(250);
        paned.set_margin_top(12);
        paned.set_margin_bottom(12);
        paned.set_margin_start(12);
        paned.set_margin_end(12);

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&paned));

        (toolbar_view, paned, close_btn, refresh_btn)
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
        log_list.set_placeholder(Some(&Label::new(Some("No log files found"))));
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

    /// Assembles the paned layout with left and right panels
    fn assemble_paned_layout(
        paned: &Paned,
        list_scrolled: ScrolledWindow,
        content_scrolled: ScrolledWindow,
    ) {
        // Left side: Log file list
        let left_box = GtkBox::new(Orientation::Vertical, 8);
        let list_label = Label::builder()
            .label("Log Files")
            .halign(gtk4::Align::Start)
            .css_classes(["heading"])
            .build();
        left_box.append(&list_label);
        left_box.append(&list_scrolled);
        paned.set_start_child(Some(&left_box));

        // Right side: Log content viewer
        let right_box = GtkBox::new(Orientation::Vertical, 8);
        let content_label = Label::builder()
            .label("Log Content")
            .halign(gtk4::Align::Start)
            .css_classes(["heading"])
            .build();
        right_box.append(&content_label);
        right_box.append(&content_scrolled);
        paned.set_end_child(Some(&right_box));
    }

    /// Gets the default log directory
    fn get_default_log_dir() -> PathBuf {
        // Use XDG data directory or fallback to home directory
        std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|_| {
                std::env::var("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
            })
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("rustconn")
            .join("logs")
    }

    /// Sets the log directory to browse
    pub fn set_log_dir(&mut self, dir: PathBuf) {
        self.log_dir = dir;
        self.populate_log_list();
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
            row_box.set_margin_top(4);
            row_box.set_margin_bottom(4);
            row_box.set_margin_start(8);
            row_box.set_margin_end(8);

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
        buffer.set_text("Loading...");

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
                    buffer_clone.set_text(&format!("Error loading log file: {e}"));
                }
            },
        );
    }

    /// Formats a file size in human-readable format
    #[allow(clippy::cast_precision_loss)]
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
        self.window.present();
    }

    /// Returns a reference to the underlying window
    #[must_use]
    pub const fn window(&self) -> &adw::Window {
        &self.window
    }
}
