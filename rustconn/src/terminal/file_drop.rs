//! Drag-and-drop file path insertion for VTE terminals
//!
//! When files are dragged from a file manager onto a VTE terminal,
//! their paths are shell-escaped and inserted as text — matching
//! the behavior of GNOME Terminal and other modern terminal emulators.

use gtk4::gdk;
use gtk4::prelude::*;
use rustconn_core::shell_escape::escape_path;
use vte4::Terminal;
use vte4::prelude::*;

/// Sets up a file drop target on a VTE terminal widget.
///
/// When files are dropped, their paths are shell-escaped (single-quoted)
/// and fed into the terminal separated by spaces. This allows the user
/// to drag files from Nautilus/Thunar/etc. to quickly insert paths into
/// commands.
///
/// Visual feedback is provided via CSS class `"terminal-drop-highlight"`.
pub fn setup_file_drop_target(terminal: &Terminal) {
    // Accept file lists (GdkFileList) from drag sources
    let drop_target = gtk4::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);

    let terminal_for_enter = terminal.clone();
    drop_target.connect_enter(move |_target, _x, _y| {
        terminal_for_enter.add_css_class("terminal-drop-highlight");
        gdk::DragAction::COPY
    });

    let terminal_for_leave = terminal.clone();
    drop_target.connect_leave(move |_target| {
        terminal_for_leave.remove_css_class("terminal-drop-highlight");
    });

    let terminal_for_drop = terminal.clone();
    drop_target.connect_drop(move |_target, value, _x, _y| {
        terminal_for_drop.remove_css_class("terminal-drop-highlight");

        // Extract GdkFileList from the drop value
        let file_list = match value.get::<gdk::FileList>() {
            Ok(list) => list,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to get FileList from drop value");
                return false;
            }
        };

        let files = file_list.files();
        if files.is_empty() {
            return false;
        }

        // Build escaped path string
        let mut paths_text = String::new();
        for (i, file) in files.iter().enumerate() {
            if let Some(path) = file.path()
                && let Some(path_str) = path.to_str()
            {
                if i > 0 {
                    paths_text.push(' ');
                }
                paths_text.push_str(&escape_path(path_str));
            }
        }

        if paths_text.is_empty() {
            tracing::debug!("No valid file paths in drop");
            return false;
        }

        // Feed the escaped paths into the terminal
        terminal_for_drop.feed_child(paths_text.as_bytes());

        tracing::debug!(
            file_count = files.len(),
            "Inserted file paths via drag-and-drop"
        );

        true
    });

    terminal.add_controller(drop_target);
}
