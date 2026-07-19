//! Playback controller and UI for recorded terminal sessions.
//!
//! Reads chunks from a [`RecordingReader`] and feeds them to a VTE terminal
//! widget with the original timing delays, using `glib::timeout_add_local_once`
//! for non-blocking scheduling on the GTK main loop.
//!
//! The UI portion provides a toolbar with play/stop/repeat/clear controls,
//! a quick-search filter for switching recordings, and a completion indicator.

use std::cell::{Cell, RefCell};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, Label, ListBox, ListBoxRow, Orientation, Popover, ScrolledWindow,
    SearchEntry, glib,
};
use rustconn_core::session::recording::{
    RecordingEntry, RecordingManager, RecordingReader, default_recordings_dir,
};
use vte4::prelude::*;

use crate::i18n::i18n;

// ---------------------------------------------------------------------------
// PlaybackState
// ---------------------------------------------------------------------------

/// Current state of the playback controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    /// No recording loaded or playback not started.
    Idle,
    /// Actively playing back chunks.
    Playing,
    /// Playback was stopped by the user.
    Stopped,
    /// All chunks have been played.
    Completed,
}

// ---------------------------------------------------------------------------
// PlaybackController
// ---------------------------------------------------------------------------

/// Controls playback of a recorded session with timing delays.
///
/// The controller owns a [`RecordingReader`] and schedules chunks to be fed
/// into a `vte4::Terminal` using `glib::timeout_add_local_once`.  A cancel
/// handle allows [`stop`](Self::stop) to abort a pending timeout.
pub struct PlaybackController {
    /// The recording reader (set after [`load`](Self::load)).
    reader: Rc<RefCell<Option<RecordingReader>>>,
    /// Current playback state.
    state: Rc<Cell<PlaybackState>>,
    /// Handle to the pending `glib` timeout source so it can be cancelled.
    cancel_handle: Rc<Cell<Option<glib::SourceId>>>,
    /// Stored data-file path for [`repeat`](Self::repeat).
    current_data_path: Rc<RefCell<Option<PathBuf>>>,
    /// Stored timing-file path for [`repeat`](Self::repeat).
    current_timing_path: Rc<RefCell<Option<PathBuf>>>,
    /// Optional callback invoked when playback state changes.
    on_state_changed: Rc<RefCell<Option<Box<dyn Fn(PlaybackState)>>>>,
}

impl PlaybackController {
    /// Creates a new controller in the [`Idle`](PlaybackState::Idle) state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            reader: Rc::new(RefCell::new(None)),
            state: Rc::new(Cell::new(PlaybackState::Idle)),
            cancel_handle: Rc::new(Cell::new(None)),
            current_data_path: Rc::new(RefCell::new(None)),
            current_timing_path: Rc::new(RefCell::new(None)),
            on_state_changed: Rc::new(RefCell::new(None)),
        }
    }

    /// Loads a recording for playback.
    ///
    /// Opens the data and timing files via [`RecordingReader::open`] and
    /// stores the paths so that [`repeat`](Self::repeat) can reload later.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if the files cannot be read or the timing file
    /// is malformed.
    pub fn load(&self, data_path: &Path, timing_path: &Path) -> io::Result<()> {
        let rr = RecordingReader::open(data_path, timing_path)?;
        *self.reader.borrow_mut() = Some(rr);
        *self.current_data_path.borrow_mut() = Some(data_path.to_path_buf());
        *self.current_timing_path.borrow_mut() = Some(timing_path.to_path_buf());
        self.set_state(PlaybackState::Idle);
        Ok(())
    }

    /// Sets the callback invoked whenever the playback state changes.
    pub fn set_on_state_changed<F: Fn(PlaybackState) + 'static>(&self, cb: F) {
        *self.on_state_changed.borrow_mut() = Some(Box::new(cb));
    }

    /// Starts playback, feeding chunks to the VTE terminal with timing delays.
    ///
    /// Does nothing if the state is already [`Playing`](PlaybackState::Playing)
    /// or no recording has been loaded.
    pub fn play(&self, vte: &vte4::Terminal) {
        if self.state.get() == PlaybackState::Playing {
            return;
        }
        if self.reader.borrow().is_none() {
            return;
        }

        self.set_state(PlaybackState::Playing);
        Self::schedule_next(
            self.reader.clone(),
            self.state.clone(),
            self.cancel_handle.clone(),
            self.on_state_changed.clone(),
            vte.clone(),
        );
    }

    /// Stops playback and cancels any pending timeout.
    ///
    /// Sets the state to [`Stopped`](PlaybackState::Stopped).
    pub fn stop(&self) {
        if let Some(source_id) = self.cancel_handle.take() {
            source_id.remove();
        }
        if self.state.get() == PlaybackState::Playing {
            self.set_state(PlaybackState::Stopped);
        }
    }

    /// Resets the recording to the beginning and starts playing from scratch.
    ///
    /// Reloads the reader from the stored file paths, resets the VTE terminal,
    /// and calls [`play`](Self::play).
    pub fn repeat(&self, vte: &vte4::Terminal) {
        self.stop();

        let data_path = self.current_data_path.borrow().clone();
        let timing_path = self.current_timing_path.borrow().clone();

        if let (Some(dp), Some(tp)) = (data_path, timing_path) {
            if let Err(e) = self.load(&dp, &tp) {
                tracing::warn!("Failed to reload recording for repeat: {e}");
                return;
            }
        } else {
            return;
        }

        vte.reset(true, true);
        self.play(vte);
    }

    /// Updates state and fires the on_state_changed callback.
    fn set_state(&self, new_state: PlaybackState) {
        self.state.set(new_state);
        if let Some(ref cb) = *self.on_state_changed.borrow() {
            cb(new_state);
        }
    }

    /// Schedules the next chunk from the reader onto the GTK main loop.
    fn schedule_next(
        reader: Rc<RefCell<Option<RecordingReader>>>,
        state: Rc<Cell<PlaybackState>>,
        cancel_handle: Rc<Cell<Option<glib::SourceId>>>,
        on_state_changed: Rc<RefCell<Option<Box<dyn Fn(PlaybackState)>>>>,
        vte: vte4::Terminal,
    ) {
        let chunk = {
            let mut reader_ref = reader.borrow_mut();
            reader_ref.as_mut().and_then(|r| r.next_chunk())
        };

        if let Some((delay, data)) = chunk {
            let reader_c = reader.clone();
            let state_c = state.clone();
            let handle_c = cancel_handle.clone();
            let cb_c = on_state_changed.clone();
            let vte_c = vte.clone();

            let source_id = glib::timeout_add_local_once(delay, move || {
                if state_c.get() != PlaybackState::Playing {
                    return;
                }
                vte_c.feed(&data);
                Self::schedule_next(reader_c, state_c, handle_c, cb_c, vte_c);
            });

            cancel_handle.set(Some(source_id));
        } else {
            // No more chunks — playback is complete.
            state.set(PlaybackState::Completed);
            cancel_handle.set(None);
            if let Some(ref cb) = *on_state_changed.borrow() {
                cb(PlaybackState::Completed);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PlaybackToolbar (10.1)
// ---------------------------------------------------------------------------

/// Holds references to the playback toolbar widgets.
pub struct PlaybackToolbar {
    /// The toolbar container.
    pub toolbar_box: GtkBox,
    /// Clear button.
    pub clear_btn: Button,
    /// Play button.
    pub play_btn: Button,
    /// Stop button.
    pub stop_btn: Button,
    /// Repeat button.
    pub repeat_btn: Button,
    /// Status label shown after playback completes.
    pub status_label: Label,
    /// Popover containing the filtered recording list.
    pub search_popover: Popover,
    /// ListBox inside the popover.
    pub search_list: ListBox,
}

impl Drop for PlaybackToolbar {
    fn drop(&mut self) {
        // Unparent the popover so GTK doesn't warn about dangling children
        self.search_popover.unparent();
    }
}

/// Creates the playback control toolbar with quick search filter.
///
/// Returns a [`PlaybackToolbar`] containing all widget references needed
/// for wiring up callbacks.
#[must_use]
pub fn create_playback_toolbar(recordings: &[RecordingEntry]) -> PlaybackToolbar {
    let toolbar_box = GtkBox::new(Orientation::Horizontal, 4);
    toolbar_box.add_css_class("playback-toolbar");
    toolbar_box.set_margin_start(6);
    toolbar_box.set_margin_end(6);

    // --- Control buttons (10.1) ---

    let clear_btn = Button::from_icon_name("edit-clear-symbolic");
    clear_btn.set_tooltip_text(Some(&i18n("Clear")));
    clear_btn.set_accessible_role(gtk4::AccessibleRole::Button);
    update_accessible_label(&clear_btn, &i18n("Clear terminal"));

    let play_btn = Button::from_icon_name("media-playback-start-symbolic");
    play_btn.set_tooltip_text(Some(&i18n("Play")));
    update_accessible_label(&play_btn, &i18n("Play recording"));

    let stop_btn = Button::from_icon_name("media-playback-stop-symbolic");
    stop_btn.set_tooltip_text(Some(&i18n("Stop")));
    update_accessible_label(&stop_btn, &i18n("Stop playback"));

    let repeat_btn = Button::from_icon_name("media-playlist-repeat-symbolic");
    repeat_btn.set_tooltip_text(Some(&i18n("Repeat")));
    update_accessible_label(&repeat_btn, &i18n("Repeat recording from start"));

    toolbar_box.append(&clear_btn);
    toolbar_box.append(&play_btn);
    toolbar_box.append(&stop_btn);
    toolbar_box.append(&repeat_btn);

    // --- Status label (10.7) ---

    let status_label = Label::new(None);
    status_label.add_css_class("playback-status-label");
    status_label.set_hexpand(true);
    status_label.set_halign(gtk4::Align::Center);
    // Mark as a status element for assistive technologies
    status_label.set_accessible_role(gtk4::AccessibleRole::Status);
    toolbar_box.append(&status_label);

    // --- Quick search filter (10.2) ---

    let search_entry = SearchEntry::new();
    search_entry.set_placeholder_text(Some(&i18n("Search recordings…")));
    search_entry.set_tooltip_text(Some(&i18n("Search recordings")));
    update_accessible_label_widget(&search_entry, &i18n("Search recordings"));
    search_entry.set_hexpand(false);
    search_entry.set_width_chars(20);

    let search_list = ListBox::new();
    search_list.set_selection_mode(gtk4::SelectionMode::Single);
    search_list.set_activate_on_single_click(true);

    // Populate the list with recording names
    populate_search_list(&search_list, recordings);

    let scrolled = ScrolledWindow::builder()
        .child(&search_list)
        .min_content_height(120)
        .max_content_height(300)
        .min_content_width(250)
        .build();

    let popover = Popover::new();
    popover.set_child(Some(&scrolled));
    popover.set_parent(&search_entry);
    popover.set_autohide(true);

    // Show popover when search entry gains focus
    let popover_for_focus = popover.clone();
    search_entry.connect_search_started(move |_| {
        popover_for_focus.popup();
    });

    // Filter list when text changes
    let search_list_for_filter = search_list.clone();
    let popover_for_change = popover.clone();
    search_entry.connect_search_changed(move |entry| {
        let query = entry.text().to_string().to_lowercase();
        popover_for_change.popup();
        let mut idx = 0;
        while let Some(row) = search_list_for_filter.row_at_index(idx) {
            let visible = if query.is_empty() {
                true
            } else if let Some(name) = row.widget_name().strip_prefix("rec-") {
                name.to_lowercase().contains(&query)
            } else {
                true
            };
            row.set_visible(visible);
            idx += 1;
        }
    });

    toolbar_box.append(&search_entry);

    PlaybackToolbar {
        toolbar_box,
        clear_btn,
        play_btn,
        stop_btn,
        repeat_btn,
        status_label,
        search_popover: popover,
        search_list,
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Populates the search list with recording entry labels.
fn populate_search_list(list: &ListBox, recordings: &[RecordingEntry]) {
    for entry in recordings {
        let display = entry
            .metadata
            .display_name
            .as_deref()
            .unwrap_or(&entry.metadata.connection_name);
        let label = Label::new(Some(display));
        label.set_halign(gtk4::Align::Start);
        label.set_margin_start(12);
        label.set_margin_end(12);
        label.set_margin_top(6);
        label.set_margin_bottom(6);

        let row = ListBoxRow::new();
        row.set_child(Some(&label));
        // Store the display name in the widget name for filtering.
        row.set_widget_name(&format!("rec-{display}"));
        list.append(&row);
    }
}

/// Sets an accessible label on a [`Button`].
fn update_accessible_label(btn: &Button, label: &str) {
    btn.update_property(&[gtk4::accessible::Property::Label(label)]);
}

/// Sets an accessible label on a [`SearchEntry`].
fn update_accessible_label_widget(widget: &SearchEntry, label: &str) {
    widget.update_property(&[gtk4::accessible::Property::Label(label)]);
}

/// Returns the display name for a [`RecordingEntry`].
fn recording_display_name(entry: &RecordingEntry) -> String {
    entry
        .metadata
        .display_name
        .clone()
        .unwrap_or_else(|| entry.metadata.connection_name.clone())
}

// ---------------------------------------------------------------------------
// Playback Tab Widget (10.4, 10.5, 10.6, 10.7)
// ---------------------------------------------------------------------------

/// Creates a complete playback tab widget containing a VTE terminal, toolbar,
/// and wired-up playback controls.
///
/// The returned [`GtkBox`] can be added directly to a `TabView` page.
/// The initial recording is loaded and playback starts automatically.
#[must_use]
pub fn create_playback_tab_widget(initial_entry: &RecordingEntry) -> GtkBox {
    let container = GtkBox::new(Orientation::Vertical, 0);
    container.set_hexpand(true);
    container.set_vexpand(true);
    container.add_css_class("playback-tab");

    // Load available recordings for the quick search list.
    let recordings = load_recordings_list();

    // Build toolbar (10.1 + 10.2).
    let toolbar = create_playback_toolbar(&recordings);
    container.append(&toolbar.toolbar_box);

    // Create VTE terminal for playback (read-only, no shell spawned).
    let vte = vte4::Terminal::new();
    vte.set_hexpand(true);
    vte.set_vexpand(true);
    vte.set_input_enabled(false);
    container.append(&vte);

    // Create PlaybackController and load the initial recording.
    let controller = Rc::new(PlaybackController::new());
    if let Err(e) = controller.load(&initial_entry.data_path, &initial_entry.timing_path) {
        tracing::warn!("Failed to load recording for playback: {e}");
    }

    // --- Wire state change callback (10.7) ---
    let play_btn_for_state = toolbar.play_btn.clone();
    let status_label_for_state = toolbar.status_label.clone();
    controller.set_on_state_changed(move |state| match state {
        PlaybackState::Idle => {
            play_btn_for_state.set_icon_name("media-playback-start-symbolic");
            status_label_for_state.set_text("");
            status_label_for_state.update_property(&[gtk4::accessible::Property::Label("")]);
        }
        PlaybackState::Playing => {
            play_btn_for_state.set_icon_name("media-playback-start-symbolic");
            let text = i18n("Playing…");
            status_label_for_state.set_text(&text);
            status_label_for_state.update_property(&[gtk4::accessible::Property::Label(&text)]);
        }
        PlaybackState::Stopped => {
            play_btn_for_state.set_icon_name("media-playback-start-symbolic");
            let text = i18n("Stopped");
            status_label_for_state.set_text(&text);
            status_label_for_state.update_property(&[gtk4::accessible::Property::Label(&text)]);
        }
        PlaybackState::Completed => {
            play_btn_for_state.set_icon_name("media-playback-start-symbolic");
            let text = i18n("Playback complete");
            status_label_for_state.set_text(&text);
            status_label_for_state.update_property(&[gtk4::accessible::Property::Label(&text)]);
        }
    });

    // Set initial status.
    toolbar
        .status_label
        .set_text(&recording_display_name(initial_entry));

    // --- Wire toolbar buttons (10.5) ---

    // Clear button: reset VTE content.
    let vte_for_clear = vte.clone();
    let controller_for_clear = controller.clone();
    toolbar.clear_btn.connect_clicked(move |_| {
        controller_for_clear.stop();
        vte_for_clear.reset(true, true);
    });

    // Play button.
    let vte_for_play = vte.clone();
    let controller_for_play = controller.clone();
    toolbar.play_btn.connect_clicked(move |_| {
        controller_for_play.play(&vte_for_play);
    });

    // Stop button.
    let controller_for_stop = controller.clone();
    toolbar.stop_btn.connect_clicked(move |_| {
        controller_for_stop.stop();
    });

    // Repeat button.
    let vte_for_repeat = vte.clone();
    let controller_for_repeat = controller.clone();
    toolbar.repeat_btn.connect_clicked(move |_| {
        controller_for_repeat.repeat(&vte_for_repeat);
    });

    // --- Wire quick search selection (10.6) ---
    let recordings_for_select = Rc::new(recordings);
    let controller_for_select = controller.clone();
    let vte_for_select = vte.clone();
    let popover_for_select = toolbar.search_popover.clone();
    let status_for_select = toolbar.status_label.clone();
    toolbar.search_list.connect_row_activated(move |_, row| {
        let idx = row.index();
        if idx < 0 {
            return;
        }
        let idx = idx as usize;
        if let Some(entry) = recordings_for_select.get(idx) {
            // Stop current playback, clear terminal, load new recording.
            controller_for_select.stop();
            vte_for_select.reset(true, true);
            if let Err(e) = controller_for_select.load(&entry.data_path, &entry.timing_path) {
                tracing::warn!("Failed to load recording: {e}");
                return;
            }
            status_for_select.set_text(&recording_display_name(entry));
            controller_for_select.play(&vte_for_select);
        }
        popover_for_select.popdown();
    });

    // Auto-play the initial recording.
    controller.play(&vte);

    container
}

/// Loads the list of available recordings from the default directory.
fn load_recordings_list() -> Vec<RecordingEntry> {
    let Some(dir) = default_recordings_dir() else {
        return Vec::new();
    };
    let manager = RecordingManager::new(dir);
    manager.list().unwrap_or_default()
}
