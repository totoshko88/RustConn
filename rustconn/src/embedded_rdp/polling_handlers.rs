//! Event handler helpers for `setup_ironrdp_polling`
//!
//! Extracted from the 1000+ line polling closure to improve readability
//! and reduce stack pressure from captured variables.
//!
//! The handlers take a borrowed context struct rather than a long parameter
//! list: the polling closure builds one per tick (all fields are references
//! to variables it already owns, so this is free) and hands it to every
//! frame- or file-related arm of the event match.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::glib;
use gtk4::prelude::*;
use rustconn_core::rdp_client::RdpClientCommand;

use super::CairoBackedBuffer;
use crate::i18n::{i18n, i18n_f};

/// Shared state the frame-related handlers operate on.
///
/// Built once per polling tick from the closure's captured variables.
pub(super) struct FrameContext<'a> {
    pub cairo_buffer: &'a Rc<RefCell<CairoBackedBuffer>>,
    pub rdp_width: &'a Rc<RefCell<u32>>,
    pub rdp_height: &'a Rc<RefCell<u32>>,
    pub first_frame_received: &'a Rc<RefCell<bool>>,
    pub connected_at: &'a Rc<RefCell<Option<std::time::Instant>>>,
    pub stale_frame: &'a Rc<Cell<bool>>,
    pub stale_label: &'a gtk4::Label,
    pub stale_banner: &'a gtk4::Box,
    pub ironrdp_tx: &'a Rc<RefCell<Option<tokio::sync::mpsc::UnboundedSender<RdpClientCommand>>>>,
}

/// Shared state the clipboard file-transfer handlers operate on.
pub(super) struct FileTransferContext<'a> {
    pub file_transfer: &'a Rc<RefCell<super::ClipboardFileTransfer>>,
    pub save_files_button: &'a gtk4::Button,
    pub status_label: &'a gtk4::Label,
    pub on_file_progress: &'a Rc<RefCell<Option<Box<dyn Fn(f64, &str) + 'static>>>>,
    pub on_file_complete: &'a Rc<RefCell<Option<Box<dyn Fn(usize, &str) + 'static>>>>,
    /// Command channel, used to send the follow-up RANGE requests that pull a
    /// clipboard file down chunk by chunk.
    pub ironrdp_tx: &'a Rc<
        RefCell<
            Option<tokio::sync::mpsc::UnboundedSender<rustconn_core::rdp_client::RdpClientCommand>>,
        >,
    >,
}

/// Records that a displayable frame arrived from the server.
///
/// Logs the connect-to-first-frame latency once and, because a frame proves
/// the socket outlived a suspend, clears the stale-session banner.
fn note_frame_arrived(ctx: &FrameContext<'_>) {
    if !*ctx.first_frame_received.borrow()
        && let Some(t) = *ctx.connected_at.borrow()
    {
        tracing::info!(
            protocol = "rdp",
            elapsed_ms = u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX),
            "[IronRDP] First displayable frame received"
        );
    }
    *ctx.first_frame_received.borrow_mut() = true;

    if ctx.stale_frame.replace(false) {
        ctx.stale_label.set_text(&i18n("Session disconnected"));
        ctx.stale_banner.set_visible(false);
    }
}

/// Handles a partial frame update by blitting the region into the Cairo buffer.
///
/// The legacy `PixelBuffer` is deliberately left untouched: it is only read by
/// the FreeRDP fallback path (populated via `on_end_paint`), so on the IronRDP
/// path a second per-frame copy was pure overhead (~33 MB/frame at 4K).
pub(super) fn handle_frame_update(
    ctx: &FrameContext<'_>,
    rect: rustconn_core::rdp_client::RdpRect,
    data: &[u8],
) {
    let mut cbuf = ctx.cairo_buffer.borrow_mut();
    cbuf.update_region(
        u32::from(rect.x),
        u32::from(rect.y),
        u32::from(rect.width),
        u32::from(rect.height),
        data,
        u32::from(rect.width) * 4,
    );
    drop(cbuf);

    note_frame_arrived(ctx);
}

/// Handles a full-screen frame update, resizing the Cairo buffer if needed.
pub(super) fn handle_full_frame_update(
    ctx: &FrameContext<'_>,
    width: u16,
    height: u16,
    data: &[u8],
) {
    {
        let mut cbuf = ctx.cairo_buffer.borrow_mut();
        if cbuf.width() != u32::from(width) || cbuf.height() != u32::from(height) {
            cbuf.resize(u32::from(width), u32::from(height));
            *ctx.rdp_width.borrow_mut() = u32::from(width);
            *ctx.rdp_height.borrow_mut() = u32::from(height);
        }
        cbuf.update_region(
            0,
            0,
            u32::from(width),
            u32::from(height),
            data,
            u32::from(width) * 4,
        );
    }

    note_frame_arrived(ctx);
}

/// Handles a server-initiated resolution change.
pub(super) fn handle_resolution_changed(ctx: &FrameContext<'_>, width: u16, height: u16) {
    tracing::debug!(protocol = "rdp", width, height, "Resolution changed");
    *ctx.rdp_width.borrow_mut() = u32::from(width);
    *ctx.rdp_height.borrow_mut() = u32::from(height);
    {
        let mut cbuf = ctx.cairo_buffer.borrow_mut();
        cbuf.resize(u32::from(width), u32::from(height));
        cbuf.fill_solid(0x1E, 0x1E, 0x1E, 0xFF);
    }
    // The reactivated desktop is repainted by the server only where content
    // changed, leaving the gray fill above as a seam on untouched regions.
    // Request a full repaint so the whole desktop is resent (no-op if the
    // server ignores the Refresh Rect).
    if let Some(ref sender) = *ctx.ironrdp_tx.borrow() {
        let _ = sender.send(RdpClientCommand::RefreshScreen);
    }
}

/// Handles server clipboard text received event (Phase 2 auto-sync).
pub(super) fn handle_clipboard_text(
    drawing_area: &gtk4::DrawingArea,
    remote_clipboard_text: &Rc<RefCell<Option<String>>>,
    copy_button: &gtk4::Button,
    clipboard_sync_suppressed: &Rc<RefCell<bool>>,
    text: String,
) {
    tracing::debug!(
        protocol = "rdp",
        chars = text.len(),
        "Received clipboard text from server"
    );
    *remote_clipboard_text.borrow_mut() = Some(text.clone());
    copy_button.set_sensitive(true);
    copy_button.set_tooltip_text(Some(&i18n("Copy remote clipboard to local")));

    // Phase 2: Auto-sync server clipboard to local GTK clipboard.
    // Use the root native surface for reliable Wayland clipboard ownership.
    *clipboard_sync_suppressed.borrow_mut() = true;
    let clipboard = if let Some(root) = drawing_area.root()
        && let Some(window) = root.downcast_ref::<gtk4::Window>()
    {
        gtk4::prelude::WidgetExt::display(window).clipboard()
    } else {
        drawing_area.display().clipboard()
    };
    clipboard.set_text(&text);
    let suppressed = clipboard_sync_suppressed.clone();
    // 100 ms is long enough for GTK to deliver its own owner-change signal for
    // the text we just set, which must not be echoed back to the server.
    glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
        *suppressed.borrow_mut() = false;
    });
    tracing::debug!(
        chars = text.len(),
        "[Clipboard] Auto-synced server text to local clipboard"
    );
}

/// Handles the clipboard file list advertised by the server.
pub(super) fn handle_clipboard_file_list(
    ctx: &FileTransferContext<'_>,
    files: Vec<rustconn_core::rdp_client::ClipboardFileInfo>,
) {
    tracing::info!(
        protocol = "rdp",
        file_count = files.len(),
        "Clipboard file list received"
    );
    for file in &files {
        tracing::debug!(
            protocol = "rdp",
            name = %file.name,
            size = file.size,
            is_dir = file.is_directory(),
            "Clipboard file entry"
        );
    }
    // Drop directories before they reach the offered count. A File Contents
    // Request for a directory is refused by the server, so keeping them made the
    // button promise files it could never deliver and turned the refusal path
    // into the ordinary case for any folder copy. Transferring a tree would mean
    // walking the descriptor hierarchy and creating directories locally, which
    // this code does not do — so a directory is not something to offer at all.
    let announced = files.len();
    let files: Vec<_> = files
        .into_iter()
        .filter(|file| !file.is_directory())
        .collect();
    let file_count = files.len();
    if file_count < announced {
        tracing::info!(
            protocol = "rdp",
            skipped = announced - file_count,
            "Skipping directories in the clipboard file list; only files can be transferred"
        );
    }
    ctx.file_transfer.borrow_mut().set_available_files(files);
    if file_count > 0 {
        ctx.save_files_button
            .set_label(&i18n_f("Save {} Files", &[&file_count.to_string()]));
        ctx.save_files_button.set_tooltip_text(Some(&i18n_f(
            "Save {} files from remote clipboard",
            &[&file_count.to_string()],
        )));
        ctx.save_files_button.set_visible(true);
        ctx.save_files_button.set_sensitive(true);
    } else {
        ctx.save_files_button.set_visible(false);
    }
}

/// Sends a RANGE File Contents request for the next chunk of a download.
fn send_range_request(ctx: &FileTransferContext<'_>, stream_id: u32, file_index: u32, offset: u64) {
    if let Some(ref sender) = *ctx.ironrdp_tx.borrow() {
        let _ = sender.send(RdpClientCommand::RequestFileContents {
            stream_id,
            file_index,
            request_size: false,
            offset,
            length: super::thread::FILE_DOWNLOAD_CHUNK_SIZE,
        });
    }
}

/// Handles the file size reply by kicking off the first data range.
///
/// The download asks only for the size up front; the size tells us how much to
/// pull, so the first RANGE request is issued here and each subsequent one from
/// [`handle_clipboard_file_contents`] as chunks arrive.
pub(super) fn handle_clipboard_file_size(ctx: &FileTransferContext<'_>, stream_id: u32, size: u64) {
    tracing::debug!(protocol = "rdp", stream_id, size, "Clipboard file size");
    let file_index = {
        let mut transfer = ctx.file_transfer.borrow_mut();
        transfer.update_size(stream_id, size);
        transfer.file_index_of(stream_id)
    };
    let Some(file_index) = file_index else {
        tracing::warn!(
            protocol = "rdp",
            stream_id,
            "size reply for an unknown download; ignoring"
        );
        return;
    };
    send_range_request(ctx, stream_id, file_index, 0);
}

/// Handles a clipboard file contents chunk from the server.
///
/// Accumulates the chunk and, if the file is not yet complete, requests the next
/// range so a file larger than a single response is pulled in full. Completion
/// is decided from the known size (or a short chunk), not a per-chunk flag.
pub(super) fn handle_clipboard_file_contents(
    ctx: &FileTransferContext<'_>,
    stream_id: u32,
    data: &[u8],
) {
    // A chunk shorter than we asked for means the server has no more to give,
    // even if its size estimate was larger — a guard against an endless loop.
    let short_chunk = (data.len() as u32) < super::thread::FILE_DOWNLOAD_CHUNK_SIZE;
    tracing::debug!(
        protocol = "rdp",
        stream_id,
        bytes = data.len(),
        short_chunk,
        "Clipboard file contents"
    );
    let (outcome, file_index) = {
        let mut transfer = ctx.file_transfer.borrow_mut();
        let outcome = transfer.append_data(stream_id, data, short_chunk);
        (outcome, transfer.file_index_of(stream_id))
    };

    let (progress, completed, total) = {
        let transfer = ctx.file_transfer.borrow();
        (
            transfer.overall_progress(),
            transfer.completed_count,
            transfer.total_files,
        )
    };

    if let Some(ref callback) = *ctx.on_file_progress.borrow() {
        callback(
            progress,
            &i18n_f(
                "Downloaded {}/{} files",
                &[&completed.to_string(), &total.to_string()],
            ),
        );
    }

    match outcome {
        super::thread::ChunkOutcome::NeedMore { next_offset } => {
            // Pull the next slice; the file is not complete yet.
            if let Some(file_index) = file_index {
                send_range_request(ctx, stream_id, file_index, next_offset);
            }
            return;
        }
        super::thread::ChunkOutcome::TooLarge => {
            // Already counted as refused and its buffer dropped; the batch may
            // now be settled, so fall through to the summary.
            tracing::warn!(
                protocol = "rdp",
                stream_id,
                limit_bytes = super::thread::MAX_DOWNLOAD_BYTES,
                "Clipboard file exceeded the download limit; skipping it"
            );
            finish_batch_if_settled(ctx);
            return;
        }
        super::thread::ChunkOutcome::Unknown => return,
        super::thread::ChunkOutcome::Complete => {}
    }

    // The bytes arrived; writing them is a separate step that can fail on a
    // full disk, a read-only target, or a filename the server sent that cannot
    // be written safely. A failed write is a lost file the user asked for, so it
    // must reach the status line — not only the log — and the batch summary must
    // not paper over it with "Saved N files".
    //
    // The result is bound before the `match` on purpose. A `match` scrutinee's
    // temporaries live until the end of the whole `match`, so borrowing inline
    // would keep the `Ref` alive inside the arms and the `borrow_mut()` below
    // would panic with `RefCell already borrowed` — on precisely the disk-full
    // path this reporting exists for.
    let save_result = ctx.file_transfer.borrow().save_download(stream_id);
    match save_result {
        Ok(path) => {
            tracing::info!(protocol = "rdp", path = %path.display(), "Saved clipboard file");
        }
        Err(e) => {
            tracing::error!(protocol = "rdp", error = %e, "Failed to save clipboard file");
            ctx.file_transfer.borrow_mut().record_save_failure();
        }
    }

    finish_batch_if_settled(ctx);
}

/// Reports the batch result once every file is settled, and frees the button.
///
/// Called from both the completion path and the refusal path: either can be the
/// event that settles the last file, and before this was shared a refusal left
/// the batch permanently unresolvable — no summary, no completion callback.
fn finish_batch_if_settled(ctx: &FileTransferContext<'_>) {
    let Some((saved, missing, target, file_count)) = ({
        let transfer = ctx.file_transfer.borrow();
        transfer.all_complete().then(|| {
            (
                transfer.saved_count(),
                // A file the server refused and a file whose write failed are
                // the same thing to the user: they asked for it and it is not
                // on disk. The log tells the two apart; the summary counts them
                // together rather than making the user parse two numbers.
                transfer.save_failures() + transfer.refused_count(),
                transfer
                    .target_directory
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                transfer.available_files.len(),
            )
        })
    }) else {
        return;
    };

    ctx.save_files_button.set_sensitive(true);
    ctx.save_files_button
        .set_label(&i18n_f("Save {} Files", &[&file_count.to_string()]));

    // When every file landed, say so; when some did not, name how many, so the
    // user is not told "saved" about a file that is not there.
    let summary = if missing == 0 {
        i18n_f("Saved {} files", &[&saved.to_string()])
    } else {
        i18n_f(
            "Saved {} files, {} could not be saved",
            &[&saved.to_string(), &missing.to_string()],
        )
    };
    ctx.status_label.set_text(&summary);
    ctx.status_label.set_visible(true);
    let status_hide = ctx.status_label.clone();
    // The confirmation is transient — 3 s is the same dwell time the other
    // inline status messages use.
    glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
        status_hide.set_visible(false);
    });

    if let Some(ref callback) = *ctx.on_file_complete.borrow() {
        callback(saved, &target);
    }
}

/// Handles a file the server refused to hand over during a "Save N Files" batch.
///
/// Drops the affected download so the batch is not left waiting on a stream that
/// will never deliver, and restores the button and status line rather than
/// leaving them stuck on "Downloading…". The rest of the batch continues.
pub(super) fn handle_clipboard_file_error(ctx: &FileTransferContext<'_>, stream_id: u32) {
    let removed = ctx.file_transfer.borrow_mut().cancel_download(stream_id);
    tracing::warn!(
        protocol = "rdp",
        stream_id,
        removed,
        "Server refused a clipboard file; skipping it"
    );

    // The refusal is now counted, so this may have settled the last file. The
    // button and the status line are left to the shared summary rather than
    // being reset here: resetting them per refusal freed the button while other
    // files were still downloading, and each refusal overwrote the status line
    // with a transient message that the final summary then overwrote again.
    finish_batch_if_settled(ctx);
}

/// Handles an RTT measurement reported by the server's Auto-Detect sequence.
pub(super) fn handle_rtt(
    config: &Rc<RefCell<Option<super::types::RdpConfig>>>,
    status_label: &gtk4::Label,
    rtt_ms: u32,
    active_graphics_mode: rustconn_core::rdp_client::GraphicsMode,
) {
    let mptcp_active = config.borrow().as_ref().is_some_and(|c| c.mptcp);
    let status_text = if mptcp_active {
        i18n_f(
            "RTT: {} ms | {} | MPTCP",
            &[&rtt_ms.to_string(), active_graphics_mode.display_name()],
        )
    } else {
        i18n_f(
            "RTT: {} ms | {}",
            &[&rtt_ms.to_string(), active_graphics_mode.display_name()],
        )
    };
    status_label.set_text(&status_text);
    status_label.set_visible(true);
    tracing::debug!(
        protocol = "rdp",
        rtt_ms,
        graphics_mode = active_graphics_mode.display_name(),
        mptcp_active,
        "RTT measurement from server Auto-Detect"
    );
}

/// Handles `DisplayControlUnavailable` — the server does not support MS-RDPEDISP.
///
/// The only way to change resolution is then a full reconnect. This always
/// reconnects: the "Reconnect on Resize" toggle controls whether `resize.rs`
/// sends the initial `SetDesktopSize` attempt at all, but once dynamic resize
/// has been tried and refused, reconnect is the correct fallback.
pub(super) fn handle_display_control_unavailable(
    config: &Rc<RefCell<Option<super::types::RdpConfig>>>,
    ironrdp_tx: &Rc<RefCell<Option<tokio::sync::mpsc::UnboundedSender<RdpClientCommand>>>>,
    on_reconnect: &Rc<RefCell<Option<Box<dyn Fn() + 'static>>>>,
    width: u16,
    height: u16,
) {
    tracing::info!(
        protocol = "rdp",
        width,
        height,
        "Display Control Channel unavailable — reconnecting with new resolution"
    );
    // Update config with the requested resolution
    {
        let current_config = config.borrow().clone();
        if let Some(mut cfg) = current_config {
            cfg = cfg.with_resolution(u32::from(width), u32::from(height));
            *config.borrow_mut() = Some(cfg);
        }
    }
    // Disconnect current session
    if let Some(ref sender) = *ironrdp_tx.borrow() {
        let _ = sender.send(RdpClientCommand::Disconnect);
    }
    // Trigger reconnect via callback, after giving the disconnect above time
    // to unwind the session task before a new one is started.
    let reconnect_cb_clone = on_reconnect.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
        if let Some(ref callback) = *reconnect_cb_clone.borrow() {
            callback();
        }
    });
}
