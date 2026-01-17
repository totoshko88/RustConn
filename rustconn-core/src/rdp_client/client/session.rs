use super::super::{RdpClientCommand, RdpClientError, RdpClientEvent, RdpRect};
use super::commands::process_command;
use super::connection::UpgradedFramed;
use ironrdp::connector::connection_activation::ConnectionActivationState;
use ironrdp::connector::ConnectionResult;
use ironrdp::graphics::image_processing::PixelFormat as IronPixelFormat;
use ironrdp::pdu::WriteBuf;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{fast_path, ActiveStage, ActiveStageOutput};
use ironrdp_tokio::{
    single_sequence_step_read, split_tokio_framed, Framed, FramedRead, FramedWrite,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Runs the active RDP session, processing framebuffer updates and input
// The future is not Send because IronRDP's AsyncNetworkClient is not Send.
// This is fine because we run on a single-threaded Tokio runtime.
#[allow(clippy::future_not_send)]
pub async fn run_active_session(
    framed: UpgradedFramed,
    connection_result: ConnectionResult,
    event_tx: std::sync::mpsc::Sender<RdpClientEvent>,
    command_rx: std::sync::mpsc::Receiver<RdpClientCommand>,
    shutdown_signal: Arc<AtomicBool>,
) -> Result<(), RdpClientError> {
    let (mut reader, mut writer) = split_tokio_framed(framed);

    // Create decoded image buffer
    let mut image = DecodedImage::new(
        IronPixelFormat::BgrA32,
        connection_result.desktop_size.width,
        connection_result.desktop_size.height,
    );

    let mut active_stage = ActiveStage::new(connection_result);

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

        // Process commands from GUI (non-blocking)
        while let Ok(cmd) = command_rx.try_recv() {
            if process_command(cmd, &mut active_stage, &mut image, &mut writer).await? {
                return Ok(());
            }
        }

        // Read and process RDP frames with timeout
        let read_result = tokio::time::timeout(
            std::time::Duration::from_millis(16), // ~60 FPS
            reader.read_pdu(),
        )
        .await;

        match read_result {
            Ok(Ok((action, payload))) => match active_stage.process(&mut image, action, &payload) {
                Ok(outputs) => {
                    for output in outputs {
                        if handle_active_stage_output(
                            output,
                            &mut writer,
                            &mut reader,
                            &event_tx,
                            &mut image,
                            &mut active_stage,
                        )
                        .await?
                        {
                            return Ok(());
                        }
                    }
                }
                Err(e) => {
                    return Err(RdpClientError::ProtocolError(format!("Session error: {e}")));
                }
            },
            Ok(Err(e)) => {
                return Err(RdpClientError::ConnectionFailed(format!("Read error: {e}")));
            }
            Err(_) => {
                // Timeout - no data available, continue loop
            }
        }
    }

    Ok(())
}

async fn handle_active_stage_output<S>(
    output: ActiveStageOutput,
    writer: &mut impl FramedWrite,
    reader: &mut Framed<S>,
    event_tx: &std::sync::mpsc::Sender<RdpClientEvent>,
    image: &mut DecodedImage,
    active_stage: &mut ActiveStage,
) -> Result<bool, RdpClientError>
where
    S: FramedRead + Unpin + Send,
{
    match output {
        ActiveStageOutput::ResponseFrame(data) => {
            if let Err(e) = writer.write_all(&data).await {
                return Err(RdpClientError::ConnectionFailed(format!(
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
            let data = extract_region_data(image, rect);
            let _ = event_tx.send(RdpClientEvent::FrameUpdate { rect, data });
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
            // Convert cursor bitmap from IronRDP's BGRA to Cairo's ARGB32 format
            // Same conversion as for framebuffer data (swap R and B channels)
            let mut converted_data = pointer.bitmap_data.clone();
            for chunk in converted_data.chunks_exact_mut(4) {
                chunk.swap(0, 2); // Swap B and R
            }
            let _ = event_tx.send(RdpClientEvent::CursorUpdate {
                width: pointer.width,
                height: pointer.height,
                hotspot_x: pointer.hotspot_x,
                hotspot_y: pointer.hotspot_y,
                data: converted_data,
            });
        }
        ActiveStageOutput::Terminate(reason) => {
            tracing::info!("RDP session terminated: {reason:?}");
            return Ok(true);
        }
        ActiveStageOutput::DeactivateAll(connection_activation) => {
            handle_reactivation(
                connection_activation,
                reader,
                writer,
                image,
                active_stage,
                event_tx,
            )
            .await?;
        }
    }
    Ok(false)
}

async fn handle_reactivation<S>(
    mut connection_activation: Box<
        ironrdp::connector::connection_activation::ConnectionActivationSequence,
    >,
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

    let mut buf = WriteBuf::new();
    loop {
        let written =
            match single_sequence_step_read(reader, &mut *connection_activation, &mut buf).await {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!("Reactivation sequence error: {}", e);
                    break;
                }
            };

        if written.size().is_some() {
            if let Err(e) = writer.write_all(buf.filled()).await {
                tracing::warn!("Failed to send reactivation response: {}", e);
                break;
            }
        }

        if let ConnectionActivationState::Finalized {
            io_channel_id,
            user_channel_id,
            desktop_size,
            enable_server_pointer,
            pointer_software_rendering,
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

            // Update the active stage with new channel IDs
            // and pointer settings
            active_stage.set_fastpath_processor(
                fast_path::ProcessorBuilder {
                    io_channel_id,
                    user_channel_id,
                    enable_server_pointer,
                    pointer_software_rendering,
                }
                .build(),
            );
            active_stage.set_enable_server_pointer(enable_server_pointer);

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

/// Extracts pixel data for a specific region from the decoded image
/// Converts from `IronRDP`'s BGRA format to Cairo's ARGB32 format by swapping R and B channels
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

    let bytes_per_pixel = 4;
    let stride = img_width as usize * bytes_per_pixel;
    let result_size = region_w as usize * region_h as usize * bytes_per_pixel;
    let mut result = vec![0u8; result_size];

    for row in 0..region_h as usize {
        let src_row = (region_y as usize + row) * stride + region_x as usize * bytes_per_pixel;
        let dst_row = row * region_w as usize * bytes_per_pixel;

        for col in 0..region_w as usize {
            let src_idx = src_row + col * bytes_per_pixel;
            let dst_idx = dst_row + col * bytes_per_pixel;

            if src_idx + 4 <= data.len() {
                // IronRDP outputs BGRA, Cairo expects ARGB32 (which is BGRA on little-endian)
                // However, we observe swapped colors (Blue <-> Red), so we need to swap them manually.
                // This suggests either IronRDP or the server is producing RGBA order.
                result[dst_idx] = data[src_idx + 2]; // B
                result[dst_idx + 1] = data[src_idx + 1]; // G
                result[dst_idx + 2] = data[src_idx]; // R
                result[dst_idx + 3] = data[src_idx + 3]; // A
            }
        }
    }

    result
}
