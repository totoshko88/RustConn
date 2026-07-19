use ironrdp::cliprdr::CliprdrClient;
use ironrdp::pdu::input::fast_path::FastPathInputEvent;
use ironrdp::pdu::rdp::headers::ShareDataPdu;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput};
use ironrdp_input::{Database, MouseButton, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp_tokio::FramedWrite;

use super::super::{RdpClientCommand, RdpClientError, RdpClientEvent};

#[expect(
    clippy::too_many_lines,
    reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
)]
pub(super) async fn process_command<W: FramedWrite>(
    cmd: RdpClientCommand,
    active_stage: &mut ActiveStage,
    image: &mut DecodedImage,
    writer: &mut W,
    event_tx: &std::sync::mpsc::Sender<RdpClientEvent>,
    input_db: &mut Database,
) -> Result<bool, RdpClientError> {
    match cmd {
        RdpClientCommand::Disconnect => {
            if let Ok(frames) = active_stage.graceful_shutdown() {
                for frame in frames {
                    if let ActiveStageOutput::ResponseFrame(data) = frame {
                        let _ = writer.write_all(&data).await;
                    }
                }
            }
            return Ok(true);
        }
        RdpClientCommand::KeyEvent {
            scancode,
            pressed,
            extended,
        } => {
            let sc = Scancode::from_u8(extended, scancode as u8);
            let op = if pressed {
                Operation::KeyPressed(sc)
            } else {
                Operation::KeyReleased(sc)
            };
            let events = input_db.apply(std::iter::once(op));
            send_input_events(active_stage, image, writer, &events).await;
        }
        RdpClientCommand::UnicodeEvent { character, pressed } => {
            let op = if pressed {
                Operation::UnicodeKeyPressed(character)
            } else {
                Operation::UnicodeKeyReleased(character)
            };
            let events = input_db.apply(std::iter::once(op));
            send_input_events(active_stage, image, writer, &events).await;
        }
        RdpClientCommand::PointerEvent { x, y, buttons: _ } => {
            let op = Operation::MouseMove(MousePosition { x, y });
            let events = input_db.apply(std::iter::once(op));
            send_input_events(active_stage, image, writer, &events).await;
        }
        RdpClientCommand::MouseButtonPress { x, y, button } => {
            let mb = gdk_button_to_ironrdp(button);
            let ops = [
                Operation::MouseMove(MousePosition { x, y }),
                Operation::MouseButtonPressed(mb),
            ];
            let events = input_db.apply(ops);
            send_input_events(active_stage, image, writer, &events).await;
        }
        RdpClientCommand::MouseButtonRelease { x, y, button } => {
            let mb = gdk_button_to_ironrdp(button);
            let ops = [
                Operation::MouseMove(MousePosition { x, y }),
                Operation::MouseButtonReleased(mb),
            ];
            let events = input_db.apply(ops);
            send_input_events(active_stage, image, writer, &events).await;
        }
        RdpClientCommand::SendCtrlAltDel => {
            let ctrl = Scancode::from_u8(false, 0x1D);
            let alt = Scancode::from_u8(false, 0x38);
            let del = Scancode::from_u8(true, 0x53);
            let ops = [
                Operation::KeyPressed(ctrl),
                Operation::KeyPressed(alt),
                Operation::KeyPressed(del),
                Operation::KeyReleased(del),
                Operation::KeyReleased(alt),
                Operation::KeyReleased(ctrl),
            ];
            let events = input_db.apply(ops);
            send_input_events(active_stage, image, writer, &events).await;
        }
        RdpClientCommand::SendKeySequence { keys } => {
            for (scancode, pressed, extended) in keys {
                let sc = Scancode::from_u8(extended, scancode as u8);
                let op = if pressed {
                    Operation::KeyPressed(sc)
                } else {
                    Operation::KeyReleased(sc)
                };
                let events = input_db.apply(std::iter::once(op));
                send_input_events(active_stage, image, writer, &events).await;
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            }
        }
        RdpClientCommand::WheelEvent {
            horizontal,
            vertical,
        } => {
            if vertical != 0 {
                let op = Operation::WheelRotations(WheelRotations {
                    is_vertical: true,
                    rotation_units: vertical,
                });
                let events = input_db.apply(std::iter::once(op));
                send_input_events(active_stage, image, writer, &events).await;
            }
            if horizontal != 0 {
                let op = Operation::WheelRotations(WheelRotations {
                    is_vertical: false,
                    rotation_units: horizontal,
                });
                let events = input_db.apply(std::iter::once(op));
                send_input_events(active_stage, image, writer, &events).await;
            }
        }
        RdpClientCommand::SetDesktopSize {
            width,
            height,
            scale_percent,
        } => {
            if let Some(result) =
                active_stage.encode_resize(u32::from(width), u32::from(height), scale_percent, None)
            {
                match result {
                    Ok(frame) => {
                        let _ = writer.write_all(&frame).await;
                        tracing::debug!("Resolution change requested: {}x{}", width, height);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to encode resize request: {}", e);
                    }
                }
            } else {
                tracing::debug!(
                    "Display Control not available for resize {}x{} — signaling GUI for reconnect",
                    width,
                    height
                );
                let _ = event_tx.send(RdpClientEvent::DisplayControlUnavailable { width, height });
            }
        }
        RdpClientCommand::RefreshScreen => {
            // Ask the server to redraw the whole desktop via a Refresh Rect PDU
            // (MS-RDPBCGR TS_REFRESH_RECT_PDU). After a connect or a
            // Deactivation-Reactivation the framebuffer is recreated blank and
            // the server only sends incremental updates, so any region it
            // considers unchanged keeps its initial fill — a visible seam that
            // only clears when its content later changes. A full-desktop refresh
            // forces a complete repaint. InclusiveRectangle is inclusive, so the
            // bottom-right corner is width-1 / height-1.
            let width = image.width();
            let height = image.height();
            if width > 0 && height > 0 {
                let area = ironrdp::pdu::geometry::InclusiveRectangle {
                    left: 0,
                    top: 0,
                    right: width.saturating_sub(1),
                    bottom: height.saturating_sub(1),
                };
                let pdu = ShareDataPdu::RefreshRectangle(
                    ironrdp::pdu::rdp::refresh_rectangle::RefreshRectanglePdu {
                        areas_to_refresh: vec![area],
                    },
                );
                let mut frame = ironrdp::core::WriteBuf::new();
                match active_stage.encode_static(&mut frame, pdu) {
                    Ok(_) => {
                        let _ = writer.write_all(frame.filled()).await;
                        tracing::debug!("Refresh Rect PDU sent for {}x{}", width, height);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to encode Refresh Rect PDU: {}", e);
                    }
                }
            }
        }
        RdpClientCommand::ClipboardText(text) => {
            // Announce CF_UNICODETEXT to the server via cliprdr, then store
            // the UTF-16LE payload so the backend can serve it when the
            // server requests the data (on_format_data_request).
            tracing::debug!(
                chars = text.len(),
                "Setting local clipboard via cliprdr channel"
            );
            let utf16_data: Vec<u8> = text
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .chain([0, 0]) // null terminator
                .collect();

            // Store pending data in the backend so on_format_data_request
            // can serve it immediately.
            if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>()
                && let Some(backend) = cliprdr
                    .downcast_backend_mut::<super::super::clipboard::RustConnClipboardBackend>()
            {
                backend.set_pending_copy_data(
                    ironrdp::cliprdr::pdu::ClipboardFormatId::CF_UNICODETEXT.value(),
                    utf16_data,
                );
            }

            // Announce the format list to the server — it will then request
            // the data via FormatDataRequest.
            let formats = vec![super::super::ClipboardFormatInfo::unicode_text()];
            handle_clipboard_copy(active_stage, writer, formats).await;
        }
        RdpClientCommand::Authenticate { .. } => {}
        RdpClientCommand::AutotypeText {
            text,
            inter_char_delay_ms,
            initial_delay_ms,
        } => {
            use unicode_segmentation::UnicodeSegmentation;

            // Initial delay gives the user time to focus the target field
            if initial_delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(u64::from(
                    initial_delay_ms,
                )))
                .await;
            }

            let delay = std::time::Duration::from_millis(u64::from(inter_char_delay_ms));

            // Iterate by grapheme clusters so composed characters (é = ´+e)
            // are sent as a single unit
            for grapheme in text.graphemes(true) {
                for ch in grapheme.chars() {
                    // Press
                    let events = input_db.apply(std::iter::once(Operation::UnicodeKeyPressed(ch)));
                    send_input_events(active_stage, image, writer, &events).await;
                    // Release
                    let events = input_db.apply(std::iter::once(Operation::UnicodeKeyReleased(ch)));
                    send_input_events(active_stage, image, writer, &events).await;
                }
                tokio::time::sleep(delay).await;
            }

            tracing::debug!(
                chars = text.len(),
                inter_char_delay_ms,
                "Autotype completed"
            );
        }
        RdpClientCommand::ClipboardData { format_id, data } => {
            handle_clipboard_data(active_stage, writer, format_id, data).await;
        }
        RdpClientCommand::ClipboardCopy(formats) => {
            handle_clipboard_copy(active_stage, writer, formats).await;
        }
        RdpClientCommand::RequestClipboardData { format_id } => {
            handle_clipboard_request(active_stage, writer, format_id).await;
        }
        RdpClientCommand::StoreLocalFiles { paths } => {
            if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>()
                && let Some(backend) = cliprdr
                    .downcast_backend_mut::<super::super::clipboard::RustConnClipboardBackend>()
            {
                backend.set_local_file_paths(paths);
            }
        }
        RdpClientCommand::RequestFileContents {
            stream_id,
            file_index,
            request_size,
            offset,
            length,
        } => {
            handle_file_contents_request(
                active_stage,
                writer,
                stream_id,
                file_index,
                request_size,
                offset,
                length,
            )
            .await;
        }
    }
    Ok(false)
}

/// Maps RDP button numbers to `ironrdp_input::MouseButton`.
///
/// The button values arrive pre-mapped by `gtk_button_to_rdp_button` in the GUI crate:
/// 1=Left, 2=Right, 3=Middle, 4=X1 (back), 5=X2 (forward).
fn gdk_button_to_ironrdp(button: u8) -> MouseButton {
    match button {
        2 => MouseButton::Right,
        3 => MouseButton::Middle,
        4 => MouseButton::X1,
        5 => MouseButton::X2,
        _ => MouseButton::Left,
    }
}

/// Sends a file-contents error response to the server via CLIPRDR.
///
/// Extracted to avoid repeating the 5-line get→submit→process→write pattern
/// at every error path in `handle_file_contents_request`.
async fn send_file_contents_error<W: FramedWrite>(
    active_stage: &mut ActiveStage,
    writer: &mut W,
    stream_id: u32,
) {
    if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() {
        let response = ironrdp::cliprdr::pdu::FileContentsResponse::new_error(stream_id);
        if let Ok(messages) = cliprdr.submit_file_contents(response)
            && let Ok(frame) = active_stage.process_svc_processor_messages(messages)
        {
            let _ = writer.write_all(&frame).await;
        }
    }
}

/// Sends input events to the RDP server
async fn send_input_events<W: FramedWrite>(
    active_stage: &mut ActiveStage,
    image: &mut DecodedImage,
    writer: &mut W,
    events: &[FastPathInputEvent],
) {
    if events.is_empty() {
        return;
    }
    if let Ok(outputs) = active_stage.process_fastpath_input(image, events) {
        for output in outputs {
            if let ActiveStageOutput::ResponseFrame(data) = output {
                let _ = writer.write_all(&data).await;
            }
        }
    }
}

async fn handle_clipboard_data<W: FramedWrite>(
    active_stage: &mut ActiveStage,
    writer: &mut W,
    format_id: u32,
    data: Vec<u8>,
) {
    if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() {
        let response = ironrdp::cliprdr::pdu::OwnedFormatDataResponse::new_data(data.clone());
        if let Ok(messages) = cliprdr.submit_format_data(response)
            && let Ok(frame) = active_stage.process_svc_processor_messages(messages)
        {
            let _ = writer.write_all(&frame).await;
            tracing::debug!(
                "Clipboard data sent for format {}: {} bytes",
                format_id,
                data.len()
            );
        }
    }
}

async fn handle_clipboard_copy<W: FramedWrite>(
    active_stage: &mut ActiveStage,
    writer: &mut W,
    formats: Vec<super::super::ClipboardFormatInfo>,
) {
    if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() {
        let clipboard_formats: Vec<ironrdp::cliprdr::pdu::ClipboardFormat> = formats
            .iter()
            .map(|f| {
                let mut format = ironrdp::cliprdr::pdu::ClipboardFormat::new(
                    ironrdp::cliprdr::pdu::ClipboardFormatId::new(f.id),
                );
                if let Some(ref name) = f.name {
                    format = format.with_name(ironrdp::cliprdr::pdu::ClipboardFormatName::new(
                        name.clone(),
                    ));
                }
                format
            })
            .collect();
        if let Ok(messages) = cliprdr.initiate_copy(&clipboard_formats)
            && let Ok(frame) = active_stage.process_svc_processor_messages(messages)
        {
            let _ = writer.write_all(&frame).await;
            tracing::debug!("Clipboard copy initiated with {} formats", formats.len());
        }
    }
}

async fn handle_clipboard_request<W: FramedWrite>(
    active_stage: &mut ActiveStage,
    writer: &mut W,
    format_id: u32,
) {
    tracing::debug!(
        "RequestClipboardData command received for format {}",
        format_id
    );
    if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() {
        let format = ironrdp::cliprdr::pdu::ClipboardFormatId::new(format_id);
        match cliprdr.initiate_paste(format) {
            Ok(messages) => {
                tracing::debug!("initiate_paste succeeded");
                if let Ok(frame) = active_stage.process_svc_processor_messages(messages) {
                    let _ = writer.write_all(&frame).await;
                    tracing::debug!("Clipboard paste request sent for format {}", format_id);
                }
            }
            Err(e) => {
                tracing::warn!("initiate_paste failed: {}", e);
            }
        }
    } else {
        tracing::warn!("CLIPRDR channel not available");
    }
}

async fn handle_file_contents_request<W: FramedWrite>(
    active_stage: &mut ActiveStage,
    writer: &mut W,
    stream_id: u32,
    file_index: u32,
    request_size: bool,
    offset: u64,
    length: u32,
) {
    tracing::debug!(
        "RequestFileContents: stream_id={}, index={}, size_request={}, offset={}, length={}",
        stream_id,
        file_index,
        request_size,
        offset,
        length
    );

    // Get the file path from the clipboard backend's stored local files
    let file_path = if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>()
        && let Some(backend) =
            cliprdr.downcast_backend_mut::<super::super::clipboard::RustConnClipboardBackend>()
    {
        backend.local_file_paths().get(file_index as usize).cloned()
    } else {
        None
    };

    let Some(path) = file_path else {
        tracing::warn!(
            "File contents request for unknown index {}: no local file stored",
            file_index
        );
        send_file_contents_error(active_stage, writer, stream_id).await;
        return;
    };

    if request_size {
        // Return file size as u64 — delegate to blocking thread to avoid
        // stalling the event loop on slow filesystems (NFS, FUSE).
        let path_clone = path.clone();
        let io_result = tokio::task::spawn_blocking(move || std::fs::metadata(&path_clone))
            .await
            .ok()
            .and_then(Result::ok);

        if let Some(meta) = io_result {
            let size = meta.len();
            tracing::debug!("File size response: index={}, size={}", file_index, size);
            if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() {
                let response =
                    ironrdp::cliprdr::pdu::FileContentsResponse::new_size_response(stream_id, size);
                if let Ok(messages) = cliprdr.submit_file_contents(response)
                    && let Ok(frame) = active_stage.process_svc_processor_messages(messages)
                {
                    let _ = writer.write_all(&frame).await;
                }
            }
        } else {
            tracing::warn!("Failed to get file metadata for index {}", file_index,);
            send_file_contents_error(active_stage, writer, stream_id).await;
        }
    } else {
        // Return file data chunk — delegate I/O to a blocking thread so
        // large clipboard file transfers don't stall the RDP event loop.
        let path_clone = path.clone();
        let io_result = tokio::task::spawn_blocking(move || {
            use std::io::{Read, Seek, SeekFrom};
            let mut file = std::fs::File::open(&path_clone)?;
            file.seek(SeekFrom::Start(offset))?;
            let mut buf = vec![0u8; length as usize];
            let bytes_read = file.read(&mut buf)?;
            buf.truncate(bytes_read);
            Ok::<Vec<u8>, std::io::Error>(buf)
        })
        .await
        .ok()
        .and_then(Result::ok);

        if let Some(buf) = io_result {
            tracing::debug!(
                "File data response: index={}, offset={}, bytes={}",
                file_index,
                offset,
                buf.len()
            );
            if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() {
                let response =
                    ironrdp::cliprdr::pdu::FileContentsResponse::new_data_response(stream_id, buf);
                if let Ok(messages) = cliprdr.submit_file_contents(response)
                    && let Ok(frame) = active_stage.process_svc_processor_messages(messages)
                {
                    let _ = writer.write_all(&frame).await;
                }
            }
        } else {
            tracing::warn!("Failed to read file index {}", file_index);
            send_file_contents_error(active_stage, writer, stream_id).await;
        }
    }
}
