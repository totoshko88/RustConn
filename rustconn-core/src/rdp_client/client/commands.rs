use super::super::{RdpClientCommand, RdpClientError};
use ironrdp::cliprdr::CliprdrClient;
use ironrdp::pdu::input::MousePdu;
use ironrdp::pdu::input::fast_path::{FastPathInputEvent, KeyboardFlags};
use ironrdp::pdu::input::mouse::PointerFlags;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput};
use ironrdp_tokio::FramedWrite;

#[allow(clippy::too_many_lines)]
pub async fn process_command<W: FramedWrite>(
    cmd: RdpClientCommand,
    active_stage: &mut ActiveStage,
    image: &mut DecodedImage,
    writer: &mut W,
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
            let event = create_keyboard_event(scancode, pressed, extended);
            send_input_events(active_stage, image, writer, &[event]).await;
        }
        RdpClientCommand::UnicodeEvent { character, pressed } => {
            let event = create_unicode_event(character, pressed);
            send_input_events(active_stage, image, writer, &[event]).await;
        }
        RdpClientCommand::PointerEvent { x, y, buttons } => {
            let event = create_pointer_event(x, y, buttons);
            send_input_events(active_stage, image, writer, &[event]).await;
        }
        RdpClientCommand::MouseButtonPress { x, y, button } => {
            let event = create_button_press_event(x, y, button);
            send_input_events(active_stage, image, writer, &[event]).await;
        }
        RdpClientCommand::MouseButtonRelease { x, y, button } => {
            let event = create_button_release_event(x, y, button);
            send_input_events(active_stage, image, writer, &[event]).await;
        }
        RdpClientCommand::SendCtrlAltDel => {
            let events = create_ctrl_alt_del_sequence();
            send_input_events(active_stage, image, writer, &events).await;
        }
        RdpClientCommand::WheelEvent {
            horizontal,
            vertical,
        } => {
            if vertical != 0 {
                let event = create_wheel_event(vertical, false);
                send_input_events(active_stage, image, writer, &[event]).await;
            }
            if horizontal != 0 {
                let event = create_wheel_event(horizontal, true);
                send_input_events(active_stage, image, writer, &[event]).await;
            }
        }
        RdpClientCommand::SetDesktopSize { width, height } => {
            if let Some(result) =
                active_stage.encode_resize(u32::from(width), u32::from(height), None, None)
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
                    "Display Control not available for resize {}x{}",
                    width,
                    height
                );
            }
        }
        RdpClientCommand::RefreshScreen => {
            tracing::debug!("Screen refresh requested");
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
        RdpClientCommand::ClipboardData { format_id, data } => {
            handle_clipboard_data(active_stage, writer, format_id, data).await;
        }
        RdpClientCommand::ClipboardCopy(formats) => {
            handle_clipboard_copy(active_stage, writer, formats).await;
        }
        RdpClientCommand::RequestClipboardData { format_id } => {
            handle_clipboard_request(active_stage, writer, format_id).await;
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
            );
        }
    }
    Ok(false)
}

/// Creates a keyboard `FastPath` event
fn create_keyboard_event(scancode: u16, pressed: bool, extended: bool) -> FastPathInputEvent {
    let mut flags = KeyboardFlags::empty();
    if !pressed {
        flags |= KeyboardFlags::RELEASE;
    }
    if extended {
        flags |= KeyboardFlags::EXTENDED;
    }
    // RDP scancodes are 8-bit, but we use u16 to preserve the value during transmission
    // The actual scancode is in the lower 8 bits
    FastPathInputEvent::KeyboardEvent(flags, scancode as u8)
}

/// Creates a Unicode keyboard `FastPath` event for non-ASCII characters
fn create_unicode_event(character: char, pressed: bool) -> FastPathInputEvent {
    let mut flags = KeyboardFlags::empty();
    if !pressed {
        flags |= KeyboardFlags::RELEASE;
    }
    // Unicode events use the character's code point as u16
    // Characters outside BMP (> 0xFFFF) are truncated, but most keyboard input is within BMP
    let code_point = character as u32 as u16;
    FastPathInputEvent::UnicodeKeyboardEvent(flags, code_point)
}

/// Creates a pointer/mouse motion `FastPath` event (no button state change)
const fn create_pointer_event(x: u16, y: u16, _buttons: u8) -> FastPathInputEvent {
    // For motion events, only send MOVE flag - no button state
    FastPathInputEvent::MouseEvent(MousePdu {
        flags: PointerFlags::MOVE,
        number_of_wheel_rotation_units: 0,
        x_position: x,
        y_position: y,
    })
}

/// Creates a mouse button press `FastPath` event
fn create_button_press_event(x: u16, y: u16, button: u8) -> FastPathInputEvent {
    let button_flag = match button {
        2 => PointerFlags::RIGHT_BUTTON,
        3 => PointerFlags::MIDDLE_BUTTON_OR_WHEEL,
        _ => PointerFlags::LEFT_BUTTON,
    };

    // Button press: button flag + DOWN, no MOVE
    FastPathInputEvent::MouseEvent(MousePdu {
        flags: button_flag | PointerFlags::DOWN,
        number_of_wheel_rotation_units: 0,
        x_position: x,
        y_position: y,
    })
}

/// Creates a mouse button release `FastPath` event
const fn create_button_release_event(x: u16, y: u16, button: u8) -> FastPathInputEvent {
    let button_flag = match button {
        2 => PointerFlags::RIGHT_BUTTON,
        3 => PointerFlags::MIDDLE_BUTTON_OR_WHEEL,
        _ => PointerFlags::LEFT_BUTTON,
    };

    // Button release: only button flag, no DOWN, no MOVE
    FastPathInputEvent::MouseEvent(MousePdu {
        flags: button_flag,
        number_of_wheel_rotation_units: 0,
        x_position: x,
        y_position: y,
    })
}

/// Creates Ctrl+Alt+Del key sequence
fn create_ctrl_alt_del_sequence() -> [FastPathInputEvent; 6] {
    [
        // Ctrl down
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x1D),
        // Alt down
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x38),
        // Delete down (extended)
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::EXTENDED, 0x53),
        // Delete up
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE | KeyboardFlags::EXTENDED, 0x53),
        // Alt up
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x38),
        // Ctrl up
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x1D),
    ]
}

/// Creates a mouse wheel event
const fn create_wheel_event(delta: i16, horizontal: bool) -> FastPathInputEvent {
    let flags = if horizontal {
        PointerFlags::HORIZONTAL_WHEEL
    } else {
        PointerFlags::VERTICAL_WHEEL
    };

    FastPathInputEvent::MouseEvent(MousePdu {
        flags,
        number_of_wheel_rotation_units: delta,
        x_position: 0,
        y_position: 0,
    })
}

/// Sends input events to the RDP server
async fn send_input_events<W: FramedWrite>(
    active_stage: &mut ActiveStage,
    image: &mut DecodedImage,
    writer: &mut W,
    events: &[FastPathInputEvent],
) {
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

#[allow(clippy::too_many_arguments)]
fn handle_file_contents_request<W: FramedWrite>(
    active_stage: &mut ActiveStage,
    _writer: &mut W,
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

    if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() {
        // Build the file contents request
        let flags = if request_size {
            ironrdp::cliprdr::pdu::FileContentsFlags::SIZE
        } else {
            ironrdp::cliprdr::pdu::FileContentsFlags::DATA
        };

        let request = ironrdp::cliprdr::pdu::FileContentsRequest {
            stream_id,
            index: file_index,
            flags,
            position: offset,
            requested_size: length,
            data_id: None,
        };

        // Try to send the request through the cliprdr client
        // Note: The cliprdr API may not expose file contents request directly,
        // so we log the attempt for now
        tracing::debug!("File contents request prepared: {:?}", request);

        // The actual sending depends on the cliprdr API
        // For now, we acknowledge the request was processed
        let _ = cliprdr;
    } else {
        tracing::warn!("CLIPRDR channel not available for file contents request");
    }
}
