use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ironrdp_connector::ConnectionResult;
use ironrdp_connector::connection_activation::{
    ConnectionActivationFactory, ConnectionActivationState,
};
use ironrdp_graphics::image_processing::PixelFormat as IronPixelFormat;
use ironrdp_pdu::WriteBuf;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_tokio::{
    Framed, FramedRead, FramedWrite, single_sequence_step_read, split_tokio_framed,
};

use super::super::{RdpClientCommand, RdpClientError, RdpClientEvent, RdpRect};
use super::commands::process_command;
use super::connection::UpgradedFramed;

/// Runs the active RDP session, processing framebuffer updates and input
///
/// Graphics-pipeline frames need nothing of their own here: `ActiveStage`
/// composites them into `image` and reports them as `GraphicsUpdate`, like
/// bitmap updates.
#[expect(
    clippy::too_many_lines,
    reason = "long session loop over frame, command and EGFX dispatch; splitting it would only relocate the shared mutable state"
)]
pub(super) async fn run_active_session(
    framed: UpgradedFramed,
    connection_result: ConnectionResult,
    event_tx: std::sync::mpsc::Sender<RdpClientEvent>,
    mut command_rx: tokio::sync::mpsc::UnboundedReceiver<RdpClientCommand>,
    shutdown_signal: Arc<AtomicBool>,
    microphone: Option<super::connection::MicrophoneLink>,
) -> Result<(), RdpClientError> {
    let (mut reader, mut writer) = split_tokio_framed(framed);

    // Create decoded image buffer
    let mut image = DecodedImage::new(
        IronPixelFormat::BgrA32,
        connection_result.desktop_size.width,
        connection_result.desktop_size.height,
    );

    // Performance monitoring: FrameStatistics tracks decode times and drop rates.
    //
    // `active_graphics_mode` is deliberately left at its default here. It used
    // to be set from `cfg!(feature = "gfx-h264")`, i.e. from a compile-time
    // constant, so the GUI status bar claimed "GFX + H.264" even on sessions
    // that never negotiated the GFX channel. The real mode now comes from the
    // EGFX capability-confirm callback as `RdpClientEvent::GraphicsModeActive`
    // (issue #262).
    //
    // Recorded on every painted frame: the RemoteFX/legacy `GraphicsUpdate`
    // path (always compiled) and, under `gfx-h264`, the GFX blit path. Before
    // that wiring `record_frame` had no caller, so `current_fps` stayed 0.0
    // even in a working session — the other half of issue #262.
    let mut frame_stats = super::super::graphics::FrameStatistics::new();

    // Build ActiveStage from ConnectionResult fields (ironrdp-session builder)
    let activation_factory = connection_result.activation_factory;
    // Kept for the error path below: a PDU that fails to decode is only
    // survivable when we can prove it arrived on the message channel.
    let message_channel_id = connection_result.message_channel_id;
    let mut active_stage = ActiveStageBuilder {
        static_channels: connection_result.static_channels,
        user_channel_id: connection_result.user_channel_id,
        io_channel_id: connection_result.io_channel_id,
        message_channel_id: connection_result.message_channel_id,
        share_id: connection_result.share_id,
        compression_type: connection_result.compression_type,
        enable_server_pointer: connection_result.enable_server_pointer,
        pointer_software_rendering: connection_result.pointer_software_rendering,
    }
    .build();

    // Input state database — tracks which keys/buttons are pressed to avoid
    // duplicate releases and enables X1/X2 (browser back/forward) buttons.
    let mut input_db = ironrdp_input::Database::new();

    // Clipboard timeouts are driven from the loop, as `Cliprdr::drive_timeouts`
    // asks. Five seconds is its own suggestion and well inside the 60 s after
    // which an unanswered file request is given up; a missed tick is not made up.
    let mut clipboard_timeouts = tokio::time::interval(std::time::Duration::from_secs(5));
    clipboard_timeouts.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Shared-folder changes are answered once a second: soon enough for
    // Explorer to look live, rare enough to cost nothing on an idle session.
    let mut directory_changes = tokio::time::interval(std::time::Duration::from_secs(1));
    directory_changes.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let (microphone_sink, mut microphone_uplink) = match microphone {
        Some(link) => (Some(link.sink), Some(link.uplink_rx)),
        None => (None, None),
    };

    loop {
        // Check shutdown signal
        if shutdown_signal.load(Ordering::SeqCst) {
            if let Ok(frames) = active_stage.graceful_shutdown() {
                for frame in frames {
                    if let ActiveStageOutput::ResponseFrame(data) = frame {
                        let _ = writer.write_all(&data).await;
                    }
                }
            }
            break;
        }

        // Use tokio::select! to wait for either a command or a server PDU
        // with zero-latency input delivery. No more 0–50ms polling delay.
        tokio::select! {
            // Branch 1: Command from GUI (keyboard, mouse, clipboard, disconnect)
            Some(cmd) = command_rx.recv() => {
                // Microphone packets go to the AUDIO_INPUT channel, which
                // queues the resulting messages for the branch below.
                if let RdpClientCommand::MicrophoneData(packet) = cmd {
                    if let Some(sink) = microphone_sink.as_ref()
                        && let Ok(mut slot) = sink.lock()
                        && let Some(sink) = slot.as_mut()
                    {
                        sink(packet);
                    }
                } else if process_command(
                    cmd,
                    &mut active_stage,
                    &mut image,
                    &mut writer,
                    &event_tx,
                    &mut input_db,
                )
                .await?
                {
                    return Ok(());
                }
            }

            // Branch 1b: microphone audio the AUDIO_INPUT channel has encoded
            Some(messages) = next_microphone_messages(&mut microphone_uplink) => {
                match active_stage.encode_dvc_messages(messages) {
                    Ok(frame) => {
                        if let Err(e) = writer.write_all(&frame).await {
                            return Err(RdpClientError::ServerDisconnected(format!(
                                "Write error: {e}"
                            )));
                        }
                    }
                    Err(e) => tracing::warn!(error = %e, "Failed to encode microphone audio"),
                }
            }

            // Branch 2: PDU from RDP server (framebuffer update, cursor, etc.)
            result = reader.read_pdu() => {
                match result {
                    Ok((action, payload)) => {
                        match active_stage.process(&mut image, action, &payload, None) {
                            Ok(outputs) => {
                                for output in outputs {
                                    if handle_active_stage_output(
                                        output,
                                        &mut writer,
                                        &mut reader,
                                        &event_tx,
                                        &mut image,
                                        &mut active_stage,
                                        &activation_factory,
                                        &mut frame_stats,
                                    )
                                    .await?
                                    {
                                        return Ok(());
                                    }
                                }
                            }
                            Err(e) if is_ignorable_message_channel_pdu(
                                action,
                                &payload,
                                message_channel_id,
                            ) => {
                                // Not fatal. See `is_ignorable_message_channel_pdu`.
                                tracing::debug!(
                                    error = %e,
                                    "ignoring an unsupported MCS message-channel PDU"
                                );
                            }
                            Err(e) => {
                                return Err(RdpClientError::ProtocolError(format!("Session error: {e}")));
                            }
                        }
                    }
                    Err(e) => {
                        // The session was up, so this is the link dropping, not
                        // a connection failing to start; see `ConnectionLost`.
                        return Err(RdpClientError::ServerDisconnected(format!("Read error: {e}")));
                    }
                }
            }

            // Branch 3: expire clipboard requests the server never answered.
            _ = directory_changes.tick() => {
                send_directory_changes(&mut active_stage, &mut writer).await;
            }

            _ = clipboard_timeouts.tick() => {
                super::commands::drive_clipboard_timeouts(&mut active_stage, &mut writer).await;
            }
        }
    }

    Ok(())
}

/// Whether a PDU that `ActiveStage::process` rejected arrived on the MCS message
/// channel, and may therefore be ignored instead of ending the session.
///
/// # Why this is protocol-correct rather than a workaround
///
/// The MCS message channel carries three kinds of PDU, distinguished by the flags
/// in their Basic Security Header: auto-detect ([MS-RDPBCGR] 2.2.14.3), Heartbeat
/// (`SEC_HEARTBEAT`) and Initiate Multitransport (`SEC_TRANSPORT_REQ`).
/// `ironrdp-session` 0.11 implements only the first, and its
/// `process_message_channel` decodes *every* message-channel PDU as an
/// `AutoDetectReqPdu`:
///
/// ```text
/// let req = decode::<AutoDetectReqPdu>(data_ctx.user_data).map_err(SessionError::decode)?;
/// ```
///
/// Windows 11 sends a Heartbeat on that channel every one to two seconds. The
/// first one fails the security-header check, the error is propagated as
/// `SessionError::decode`, and the caller below used to turn *any* error from
/// `process` into a dead session — so an embedded RDP session against a Windows 11
/// host would drop seconds after connecting. RustConn always registers `drdynvc`
/// as a static channel, and one static channel is enough to make Windows allocate
/// a real message channel instead of refusing with id 0, so this is reachable on
/// every such connection rather than being an exotic configuration.
///
/// Ignoring an unparseable message-channel PDU is what the specification asks for:
/// a client that does not implement Heartbeat or multitransport is not required to
/// answer them, and neither is optional-but-fatal. Auto-detect itself is advisory —
/// it feeds the server's bandwidth estimate — so even a genuinely malformed
/// auto-detect request is not worth a disconnect.
///
/// # Why the channel is re-derived here instead of matching on the error
///
/// `SessionError` is `Error<SessionErrorKind>`, and `SessionErrorKind::Decode` is
/// produced at several sites in `ironrdp-session`, including the outer
/// `decode_send_data_indication`. Matching on the kind would therefore also
/// swallow a failure to parse the MCS framing itself, which *is* fatal. Re-reading
/// the frame costs one MCS header decode and only on the error path, and it
/// answers the precise question: which channel was this on.
///
/// Remove this on the next `ironrdp-session` bump. Upstream [IronRDP#1814] merged
/// the real fix on 2026-08-27 — the demux now peeks the security-header flags and
/// ignores what it does not recognise — but it is not on crates.io yet, where
/// `ironrdp-session` is still 0.11.0 with the unconditional decode. Check
/// `x224::Processor::process_message_channel` after the bump: once it dispatches on
/// the flags, this guard is dead weight.
///
/// The guard is deliberately narrow: it returns `false` for a fast-path PDU, for a
/// connection with no message channel, and for anything whose MCS framing does not
/// parse or names a different channel.
///
/// Not covered here, and still open upstream: [IronRDP#1629], where an auto-detect
/// request arriving *after* the connector's connect-time phase kills the
/// connection during the licensing exchange. That is `ironrdp-connector`, before
/// this session loop exists, so it needs its own fix.
///
/// [IronRDP#1814]: https://github.com/Devolutions/IronRDP/pull/1814
/// [IronRDP#1629]: https://github.com/Devolutions/IronRDP/issues/1629
/// Answers the shared-folder change notifications the directory watcher has
/// collected (`RustConnRdpdrBackend::poll_directory_changes`).
async fn send_directory_changes(active_stage: &mut ActiveStage, writer: &mut impl FramedWrite) {
    let Some(backend) = active_stage
        .get_svc_processor_mut::<ironrdp_rdpdr::Rdpdr>()
        .and_then(|rdpdr| {
            rdpdr.downcast_backend_mut::<crate::rdp_client::rdpdr::RustConnRdpdrBackend>()
        })
    else {
        return;
    };
    let messages = backend.poll_directory_changes();
    if messages.is_empty() {
        return;
    }
    let messages = ironrdp_svc::SvcProcessorMessages::<ironrdp_rdpdr::Rdpdr>::new(messages);
    match active_stage.process_svc_processor_messages(messages) {
        Ok(frame) => {
            if let Err(e) = writer.write_all(&frame).await {
                tracing::warn!(error = %e, "Failed to send directory change notifications");
            }
        }
        Err(e) => tracing::warn!(error = %e, "Failed to encode directory change notifications"),
    }
}

/// Logs the session outputs a desktop session has no use for: no RemoteApp
/// windows, one monitor, and no automatic reconnection cookie — the GUI
/// reconnects with the stored credentials instead.
fn log_session_notice(output: &ActiveStageOutput) {
    match output {
        ActiveStageOutput::SaveSessionInfo { .. } | ActiveStageOutput::SessionId(_) => {
            tracing::debug!("Server sent session information");
        }
        ActiveStageOutput::LogonErrors(info) => {
            tracing::debug!(?info, "Server reported a logon notification");
        }
        _ => {}
    }
}

/// Waits for the next batch of microphone messages, or forever when the
/// connection has no microphone, so the `select!` branch never fires.
async fn next_microphone_messages(
    uplink: &mut Option<tokio::sync::mpsc::UnboundedReceiver<Vec<ironrdp_svc::SvcMessage>>>,
) -> Option<Vec<ironrdp_svc::SvcMessage>> {
    match uplink {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

fn is_ignorable_message_channel_pdu(
    action: ironrdp_pdu::Action,
    payload: &[u8],
    message_channel_id: Option<u16>,
) -> bool {
    // Auto-detect, Heartbeat and multitransport all travel over X224; a fast-path
    // failure is a framebuffer problem and has nothing to do with this.
    if action != ironrdp_pdu::Action::X224 {
        return false;
    }
    let Some(message_channel_id) = message_channel_id else {
        return false;
    };
    ironrdp_pdu::mcs::decode_send_data_indication(payload)
        .is_ok_and(|ctx| ctx.channel_id == message_channel_id)
}

#[expect(
    clippy::too_many_arguments,
    reason = "stage-output dispatcher: adds the per-frame statistics sink to the existing writer/reader/image/stage/factory set"
)]
async fn handle_active_stage_output<S>(
    output: ActiveStageOutput,
    writer: &mut impl FramedWrite,
    reader: &mut Framed<S>,
    event_tx: &std::sync::mpsc::Sender<RdpClientEvent>,
    image: &mut DecodedImage,
    active_stage: &mut ActiveStage,
    activation_factory: &ConnectionActivationFactory,
    frame_stats: &mut super::super::graphics::FrameStatistics,
) -> Result<bool, RdpClientError>
where
    S: FramedRead + Unpin + Send,
{
    match output {
        ActiveStageOutput::ResponseFrame(data) => {
            if let Err(e) = writer.write_all(&data).await {
                return Err(RdpClientError::ServerDisconnected(format!(
                    "Write error: {e}"
                )));
            }
        }
        ActiveStageOutput::GraphicsUpdate(region) => {
            let rect = RdpRect::new(
                region.left,
                region.top,
                region.right.saturating_sub(region.left),
                region.bottom.saturating_sub(region.top),
            );
            // Measure the region extraction (the per-frame CPU work on this
            // path) and record the painted frame. This is the RemoteFX/legacy
            // path, the one a session actually uses when H.264 is unavailable —
            // so without this `current_fps` read 0.0 even in a perfectly
            // working session (issue #262, the "dead FPS counter" half).
            let extract_start = std::time::Instant::now();
            let data = extract_region_data(image, rect);
            let extract_us = extract_start.elapsed().as_micros() as u64;
            let bytes = data.len();
            let _ = event_tx.send(RdpClientEvent::FrameUpdate { rect, data });
            frame_stats.record_frame(bytes, extract_us);
        }
        ActiveStageOutput::PointerDefault => {
            let _ = event_tx.send(RdpClientEvent::CursorDefault);
        }
        ActiveStageOutput::PointerHidden => {
            let _ = event_tx.send(RdpClientEvent::CursorHidden);
        }
        ActiveStageOutput::PointerPosition { x, y } => {
            let _ = event_tx.send(RdpClientEvent::CursorPosition { x, y });
        }
        ActiveStageOutput::PointerBitmap(pointer) => {
            let expected_size = usize::from(pointer.width) * usize::from(pointer.height) * 4;

            let src = if pointer.bitmap_data.len() > expected_size {
                &pointer.bitmap_data[..expected_size]
            } else {
                &pointer.bitmap_data
            };

            let data = src.to_vec();

            // Pass RGBA data as-is — handle_cursor_update crops transparent
            // padding and does premultiplied alpha + R↔B for HiDPI cursors
            // (pointer bitmaps from IronRDP are RGBA, unlike framebuffer which is BGRA)
            let _ = event_tx.send(RdpClientEvent::CursorUpdate {
                width: pointer.width,
                height: pointer.height,
                hotspot_x: pointer.hotspot_x,
                hotspot_y: pointer.hotspot_y,
                data,
            });
        }
        ActiveStageOutput::Terminate(reason) => {
            tracing::info!("RDP session terminated: {reason:?}");
            return Ok(true);
        }
        ActiveStageOutput::DeactivateAll => {
            handle_reactivation(
                activation_factory,
                reader,
                writer,
                image,
                active_stage,
                event_tx,
            )
            .await?;
        }
        ActiveStageOutput::MultitransportRequest(pdu) => {
            // Server requests sideband UDP (multitransport) transport. We do not
            // implement UDP multitransport — the TCP virtual channels carry
            // everything we need — so log and continue over TCP.
            tracing::debug!(
                request_id = pdu.request_id,
                "Server requested multitransport (UDP) — not supported, ignoring"
            );
        }
        ActiveStageOutput::AutoDetect(request) => {
            // Server sends a network-characteristics result (RTT/bandwidth
            // probe). Extract the RTT measurement and forward it to the GUI.
            if let ironrdp_pdu::rdp::autodetect::AutoDetectRequest::NetworkCharacteristicsResult {
                average_rtt_ms,
                ..
            } = &request
            {
                let _ = event_tx.send(RdpClientEvent::Rtt {
                    rtt_ms: *average_rtt_ms,
                });
            }
            tracing::debug!(
                ?request,
                "Received Auto-Detect network characteristics from server"
            );
        }
        ActiveStageOutput::ServerRedirection(_) => {
            // A connection broker moved the session mid-flight. Ending it as a
            // dropped connection lets automatic reconnection go through the
            // broker again, which routes the user back to their session.
            return Err(RdpClientError::ServerDisconnected(
                "Server redirection to another session host".to_string(),
            ));
        }
        notice @ (ActiveStageOutput::MonitorLayout(_)
        | ActiveStageOutput::WindowingOrders(_)
        | ActiveStageOutput::WindowSurfaceUpdates(_)
        | ActiveStageOutput::AutoReconnectCookie(_)
        | ActiveStageOutput::AutoReconnectFailed
        | ActiveStageOutput::SaveSessionInfo { .. }
        | ActiveStageOutput::SessionId(_)
        | ActiveStageOutput::LogonErrors(_)) => log_session_notice(&notice),
    }
    Ok(false)
}

async fn handle_reactivation<S>(
    activation_factory: &ConnectionActivationFactory,
    reader: &mut Framed<S>,
    writer: &mut impl FramedWrite,
    image: &mut DecodedImage,
    active_stage: &mut ActiveStage,
    event_tx: &std::sync::mpsc::Sender<RdpClientEvent>,
) -> Result<(), RdpClientError>
where
    S: FramedRead + Unpin + Send,
{
    // Execute the Deactivation-Reactivation Sequence:
    // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dfc234ce-481a-4674-9a5d-2a7bafb14432
    tracing::debug!(
        "Received Server Deactivate All PDU, executing Deactivation-Reactivation Sequence"
    );

    let mut connection_activation = activation_factory.create();
    let io_channel_id = activation_factory.io_channel_id();
    let user_channel_id = activation_factory.user_channel_id();

    let mut buf = WriteBuf::new();
    loop {
        let written =
            match single_sequence_step_read(reader, &mut connection_activation, &mut buf).await {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!("Reactivation sequence error: {}", e);
                    break;
                }
            };

        if written.size().is_some()
            && let Err(e) = writer.write_all(buf.filled()).await
        {
            tracing::warn!("Failed to send reactivation response: {}", e);
            break;
        }

        if let ConnectionActivationState::Finalized {
            desktop_size,
            share_id,
            enable_server_pointer,
            pointer_software_rendering,
            static_channel_chunk_size,
            ..
        } = connection_activation.connection_activation_state()
        {
            tracing::debug!(
                ?desktop_size,
                "Deactivation-Reactivation Sequence completed"
            );

            // Update image size with the new desktop size
            *image = DecodedImage::new(
                IronPixelFormat::BgrA32,
                desktop_size.width,
                desktop_size.height,
            );

            // Rebuild the session for the new activation: channel IDs, share
            // ID, pointer settings and the static channel chunk size.
            if !active_stage.reactivate(
                io_channel_id,
                user_channel_id,
                share_id,
                enable_server_pointer,
                pointer_software_rendering,
                static_channel_chunk_size,
            ) {
                tracing::warn!(
                    static_channel_chunk_size,
                    "Reactivation rejected the server's static channel chunk size"
                );
            }

            // Notify GUI about resolution change
            let _ = event_tx.send(RdpClientEvent::ResolutionChanged {
                width: desktop_size.width,
                height: desktop_size.height,
            });

            break;
        }
    }
    Ok(())
}

/// Extracts pixel data for a specific region from the decoded image.
///
/// IronRDP outputs pixels in BgrA32 which matches Cairo's ARGB32 format
/// on little-endian (both are B-G-R-A byte order in memory). No channel swap needed.
///
/// Optimized for 4K rendering: uses row-based `memcpy` which is cache-friendly
/// and auto-vectorizable by LLVM.
fn extract_region_data(image: &DecodedImage, rect: RdpRect) -> Vec<u8> {
    let img_width = image.width();
    let img_height = image.height();
    let data = image.data();

    let region_x = rect.x.min(img_width);
    let region_y = rect.y.min(img_height);
    let region_w = rect.width.min(img_width.saturating_sub(region_x));
    let region_h = rect.height.min(img_height.saturating_sub(region_y));

    if region_w == 0 || region_h == 0 {
        return Vec::new();
    }

    let bpp = 4;

    // Fast path: if the region covers the entire image, avoid row-by-row copy
    if region_x == 0 && region_y == 0 && region_w == img_width && region_h == img_height {
        return data.to_vec();
    }

    let src_stride = img_width as usize * bpp;
    let dst_stride = region_w as usize * bpp;
    let result_size = dst_stride * region_h as usize;
    let mut result = vec![0u8; result_size];

    // Copy rows in bulk (cache-friendly, compiles to memcpy)
    for row in 0..region_h as usize {
        let src_offset = (region_y as usize + row) * src_stride + region_x as usize * bpp;
        let dst_offset = row * dst_stride;

        if src_offset + dst_stride <= data.len() {
            result[dst_offset..dst_offset + dst_stride]
                .copy_from_slice(&data[src_offset..src_offset + dst_stride]);
        }
    }

    result
}

// ============================================================================
// GFX Pipeline Integration (gfx-h264 feature)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a real MCS Send Data Indication frame for `channel_id`.
    ///
    /// Encoded through the same types `decode_send_data_indication` parses, so the
    /// test exercises the actual framing rather than a hand-rolled byte string that
    /// could drift from it.
    fn indication_frame(channel_id: u16, user_data: &[u8]) -> Vec<u8> {
        use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
        use ironrdp_pdu::x224::X224;

        let msg = SendDataIndication {
            initiator_id: 1002,
            channel_id,
            user_data: std::borrow::Cow::Borrowed(user_data),
        };
        let mut buf = ironrdp_pdu::WriteBuf::new();
        ironrdp_core::encode_buf(&X224(McsMessage::SendDataIndication(msg)), &mut buf)
            .expect("encoding a Send Data Indication must succeed");
        buf.into_inner()
    }

    /// The case this guard exists for: Windows 11 sends a Heartbeat on the message
    /// channel every one to two seconds, `ironrdp-session` cannot decode it, and
    /// the session must survive.
    #[test]
    fn a_pdu_on_the_message_channel_is_ignorable() {
        let frame = indication_frame(1005, &[0xff, 0xff, 0xff, 0xff]);
        assert!(is_ignorable_message_channel_pdu(
            ironrdp_pdu::Action::X224,
            &frame,
            Some(1005)
        ));
    }

    /// Everything below is the other half of the contract: the guard must not turn
    /// a genuine protocol failure into a silently ignored one.
    #[test]
    fn a_pdu_on_another_channel_is_still_fatal() {
        // The IO channel carries the framebuffer; a decode failure there is real.
        let frame = indication_frame(1003, &[0xff, 0xff, 0xff, 0xff]);
        assert!(!is_ignorable_message_channel_pdu(
            ironrdp_pdu::Action::X224,
            &frame,
            Some(1005)
        ));
    }

    #[test]
    fn a_fast_path_failure_is_still_fatal() {
        // Fast-path updates are framebuffer data and never reach the message
        // channel, so the channel id must not even be consulted.
        let frame = indication_frame(1005, &[0xff]);
        assert!(!is_ignorable_message_channel_pdu(
            ironrdp_pdu::Action::FastPath,
            &frame,
            Some(1005)
        ));
    }

    #[test]
    fn without_a_message_channel_nothing_is_ignorable() {
        // A server that refused to allocate the channel (id 0) cannot have sent
        // anything on it, so a failure has some other cause.
        let frame = indication_frame(1005, &[0xff]);
        assert!(!is_ignorable_message_channel_pdu(
            ironrdp_pdu::Action::X224,
            &frame,
            None
        ));
    }

    #[test]
    fn unparseable_framing_is_still_fatal() {
        // If the MCS envelope itself does not decode, the stream is out of sync —
        // which is exactly the error that must not be swallowed. This is why the
        // guard re-derives the channel instead of matching on the error kind:
        // `SessionErrorKind::Decode` covers this case too.
        assert!(!is_ignorable_message_channel_pdu(
            ironrdp_pdu::Action::X224,
            &[0x00, 0x01, 0x02, 0x03],
            Some(1005)
        ));
        assert!(!is_ignorable_message_channel_pdu(
            ironrdp_pdu::Action::X224,
            &[],
            Some(1005)
        ));
    }
}
