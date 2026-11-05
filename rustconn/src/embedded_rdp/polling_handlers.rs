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
use rustconn_core::rdp_client::{ClipboardFormatInfo, RdpClientCommand};

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
    /// Command channel, used to send the follow-up RANGE requests that pull a
    /// clipboard file down chunk by chunk.
    pub ironrdp_tx: &'a Rc<
        RefCell<
            Option<tokio::sync::mpsc::UnboundedSender<rustconn_core::rdp_client::RdpClientCommand>>,
        >,
    >,
    /// True while a file drag-and-drop upload offer is outstanding.
    ///
    /// Threaded through so [`release_announce_hold`] can apply the same guard
    /// the local-clipboard monitor itself applies: announcing here while a
    /// DnD offer is live would replace that offer before the server pulls it
    /// (issue #390, review finding: release path skipping the monitor's
    /// guards).
    pub file_offer_active: &'a Rc<Cell<bool>>,
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
    file_transfer: &Rc<RefCell<super::ClipboardFileTransfer>>,
    text: String,
) {
    tracing::debug!(
        protocol = "rdp",
        chars = text.len(),
        "Received clipboard text from server"
    );
    // The remote just announced a newer clipboard of its own. A local change
    // deferred earlier is now stale: sending it once the hold comes down
    // would replace the text that just arrived with the older local one
    // (issue #390, review finding: release path overriding a newer remote
    // clipboard).
    file_transfer.borrow_mut().drop_deferred_announce();
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
    // An empty list is the backend withdrawing the offer: the remote clipboard
    // changed to something that is not files, which happens on every remote
    // text copy. Hide the button and withdraw the offer. A file already in
    // flight may finish, but nothing new is asked for, since the server now
    // answers from a different clipboard (see `withdraw_offer`); the next real
    // list replaces the offer.
    if files.is_empty() {
        tracing::debug!(protocol = "rdp", "Remote clipboard no longer offers files");
        ctx.file_transfer.borrow_mut().withdraw_offer();
        ctx.save_files_button.set_visible(false);
        // The files the batch had not started are settled now; with none in
        // flight either, this is what reports the batch.
        finish_batch_if_settled(ctx);
        return;
    }
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
    // Drops a deferred announce from a batch this list just superseded; see
    // `ClipboardFileTransfer::set_available_files` (issue #390).
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

/// Sends a deferred local-clipboard announce to the server once nothing holds
/// the announce back anymore.
///
/// Called everywhere [`super::ClipboardFileTransfer::holds_local_announce`]
/// can turn false: the folder picker closing without a batch starting, and a
/// batch settling (via [`finish_batch_if_settled`]). A local clipboard change
/// seen while the hold was up is not lost — it goes out once, here — and
/// nothing is sent if no change happened while the hold was up, so closing the
/// picker or finishing a batch does not itself cause an announce
/// (issue #390).
pub(super) fn release_announce_hold(
    file_transfer: &Rc<RefCell<super::ClipboardFileTransfer>>,
    file_offer_active: &Rc<Cell<bool>>,
    ironrdp_tx: &Rc<RefCell<Option<tokio::sync::mpsc::UnboundedSender<RdpClientCommand>>>>,
) {
    // The monitor itself refuses to announce while a DnD upload offer is
    // outstanding (`offer_active` guard, checked before this hold is even
    // reached) — releasing the hold must not bypass that guard. The flag is
    // left deferred rather than taken: it is not lost, only postponed to the
    // next release once the offer has been consumed or timed out.
    if file_offer_active.get() {
        return;
    }
    let should_announce = {
        let mut transfer = file_transfer.borrow_mut();
        !transfer.holds_local_announce() && transfer.take_deferred_announce()
    };
    if !should_announce {
        return;
    }
    if let Some(ref sender) = *ironrdp_tx.borrow() {
        let _ = sender.send(RdpClientCommand::AnnounceClipboardFormats(vec![
            ClipboardFormatInfo::unicode_text(),
        ]));
        tracing::debug!(
            protocol = "rdp",
            "[Clipboard] Released local announce hold; sent deferred text format to server"
        );
    }
}

/// Starts the batch's next file, or reports the batch once none is left.
///
/// A batch fetches one file at a time, so whatever settles a file — its save,
/// a refusal, the size cap — hands over to the next one here.
fn start_next_or_finish(ctx: &FileTransferContext<'_>) {
    let next = ctx.file_transfer.borrow_mut().start_next_download();
    match next {
        Some((stream_id, file_index)) => send_size_request(ctx, stream_id, file_index),
        None => finish_batch_if_settled(ctx),
    }
}

/// Sends the SIZE File Contents request that starts a download.
///
/// The first data range follows the size reply (see
/// [`handle_clipboard_file_size`]), so a file larger than one response is
/// pulled in full instead of being cut short by a single all-at-once request.
fn send_size_request(ctx: &FileTransferContext<'_>, stream_id: u32, file_index: u32) {
    if let Some(ref sender) = *ctx.ironrdp_tx.borrow() {
        let _ = sender.send(RdpClientCommand::RequestFileContents {
            stream_id,
            file_index,
            request_size: true,
            offset: 0,
            length: 0,
        });
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
    // Over the cap: settle it now rather than fetch it up to the cap and drop it.
    let oversized = ctx
        .file_transfer
        .borrow_mut()
        .refuse_if_oversized(stream_id, size);
    if oversized {
        tracing::warn!(
            protocol = "rdp",
            stream_id,
            size,
            limit_bytes = super::thread::MAX_DOWNLOAD_BYTES,
            "Clipboard file is over the download limit; skipping it"
        );
        start_next_or_finish(ctx);
        return;
    }
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
            start_next_or_finish(ctx);
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
    let save_result = ctx.file_transfer.borrow_mut().save_download(stream_id);
    match save_result {
        Ok(path) => {
            tracing::info!(protocol = "rdp", path = %path.display(), "Saved clipboard file");
        }
        Err(e) => {
            tracing::error!(protocol = "rdp", error = %e, "Failed to save clipboard file");
            ctx.file_transfer.borrow_mut().record_save_failure();
        }
    }

    start_next_or_finish(ctx);
}

/// Reports the batch result once every file is settled, and frees the button.
///
/// Called from both the completion path and the refusal path: either can be the
/// event that settles the last file, and before this was shared a refusal left
/// the batch permanently unresolvable — no summary, no completion callback.
fn finish_batch_if_settled(ctx: &FileTransferContext<'_>) {
    let Some((saved, missing, file_count)) = ({
        let transfer = ctx.file_transfer.borrow();
        transfer.all_complete().then(|| {
            (
                transfer.saved_count(),
                // A file the server refused and a file whose write failed are
                // the same thing to the user: they asked for it and it is not
                // on disk. The log tells the two apart; the summary counts them
                // together rather than making the user parse two numbers.
                transfer.save_failures() + transfer.refused_count(),
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

    // The batch just settled, so the local-clipboard monitor's hold
    // (`holds_local_announce`) may now be down; send on whatever change it
    // deferred while the batch was running (issue #390).
    release_announce_hold(ctx.file_transfer, ctx.file_offer_active, ctx.ironrdp_tx);
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

    // The refusal is now counted, so the batch moves on to its next file, or
    // this settled the last one. The button and the status line are left to the
    // shared summary rather than being reset here: resetting them per refusal
    // freed the button while other files were still downloading, and each
    // refusal overwrote the status line with a transient message that the final
    // summary then overwrote again. An error for a stream that is not tracked —
    // a duplicate, or one from a superseded batch — must not start a file, or
    // two would be in flight at once.
    if removed {
        start_next_or_finish(ctx);
    } else {
        finish_batch_if_settled(ctx);
    }
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
///
/// `layout_ignored` means the layout was sent and the server did not apply it
/// in time. Server OS `Windows` keeps the seamless path then; otherwise the
/// runtime config remembers it (`layout_resize_ignored`), so this reconnect
/// and every later resize of the session go straight to a reconnect instead of
/// waiting for the server again.
pub(super) fn handle_display_control_unavailable(
    config: &Rc<RefCell<Option<super::types::RdpConfig>>>,
    ironrdp_tx: &Rc<RefCell<Option<tokio::sync::mpsc::UnboundedSender<RdpClientCommand>>>>,
    on_reconnect: &Rc<RefCell<Option<Box<dyn Fn() + 'static>>>>,
    width: u16,
    height: u16,
    layout_ignored: bool,
) {
    // Update config with the requested resolution (and the ignored layout).
    let current_config = config.borrow().clone();
    if let Some(cfg) = current_config {
        let Some(next) = cfg.after_display_control_unavailable(width, height, layout_ignored)
        else {
            tracing::info!(
                protocol = "rdp",
                width,
                height,
                "Windows server has not applied the monitor layout yet — keeping Display Control"
            );
            return;
        };
        *config.borrow_mut() = Some(next);
    }
    tracing::info!(
        protocol = "rdp",
        width,
        height,
        layout_ignored,
        "Display Control resize not possible — reconnecting with new resolution"
    );
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

#[cfg(test)]
mod release_announce_hold_tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use rustconn_core::rdp_client::RdpClientCommand;

    use super::super::ClipboardFileTransfer;
    use super::release_announce_hold;

    /// A sender/receiver pair standing in for the real IronRDP command
    /// channel, wrapped the same way [`super::FileTransferContext::ironrdp_tx`]
    /// is: behind `Rc<RefCell<Option<...>>>`.
    fn channel() -> (
        Rc<RefCell<Option<tokio::sync::mpsc::UnboundedSender<RdpClientCommand>>>>,
        tokio::sync::mpsc::UnboundedReceiver<RdpClientCommand>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (Rc::new(RefCell::new(Some(tx))), rx)
    }

    /// With nothing holding the announce and no DnD offer outstanding, a
    /// deferred change goes out and is taken exactly once.
    #[test]
    fn releases_and_sends_once_nothing_holds_and_no_offer_is_outstanding() {
        let transfer = Rc::new(RefCell::new(ClipboardFileTransfer::new()));
        transfer.borrow_mut().defer_announce();
        let file_offer_active = Rc::new(Cell::new(false));
        let (tx, mut rx) = channel();

        release_announce_hold(&transfer, &file_offer_active, &tx);

        assert!(
            matches!(
                rx.try_recv(),
                Ok(RdpClientCommand::AnnounceClipboardFormats(_))
            ),
            "the deferred change is announced"
        );
        assert!(!transfer.borrow_mut().take_deferred_announce());
    }

    /// A DnD upload offer outstanding must postpone the release, not drop it:
    /// the monitor itself would refuse to announce right now, and the release
    /// path must apply the same guard rather than clobbering the file offer
    /// before the server pulls it (issue #390, review finding: release path
    /// skipping the monitor's guards).
    #[test]
    fn a_file_offer_in_flight_postpones_the_release() {
        let transfer = Rc::new(RefCell::new(ClipboardFileTransfer::new()));
        transfer.borrow_mut().defer_announce();
        let file_offer_active = Rc::new(Cell::new(true));
        let (tx, mut rx) = channel();

        release_announce_hold(&transfer, &file_offer_active, &tx);

        assert!(
            rx.try_recv().is_err(),
            "nothing is sent while the DnD offer is live"
        );
        assert!(
            transfer.borrow_mut().take_deferred_announce(),
            "the deferral is kept, not lost"
        );
    }

    /// No deferred change means nothing to send, DnD offer or not.
    #[test]
    fn nothing_is_sent_with_no_deferred_change() {
        let transfer = Rc::new(RefCell::new(ClipboardFileTransfer::new()));
        let file_offer_active = Rc::new(Cell::new(false));
        let (tx, mut rx) = channel();

        release_announce_hold(&transfer, &file_offer_active, &tx);

        assert!(rx.try_recv().is_err());
    }

    /// A batch still running (the hold itself, not a DnD offer) must also
    /// keep the deferral rather than sending it.
    #[test]
    fn a_running_batch_also_postpones_the_release() {
        let transfer = Rc::new(RefCell::new(ClipboardFileTransfer::new()));
        transfer.borrow_mut().begin_folder_pick();
        transfer.borrow_mut().defer_announce();
        let file_offer_active = Rc::new(Cell::new(false));
        let (tx, mut rx) = channel();

        release_announce_hold(&transfer, &file_offer_active, &tx);

        assert!(rx.try_recv().is_err());
        assert!(transfer.borrow_mut().take_deferred_announce());
    }
}
