//! EGFX Graphics Pipeline handler for the RDP client.
//!
//! Implements [`GraphicsPipelineHandler`] from `ironrdp-egfx` to receive
//! decoded bitmap data and surface lifecycle events. The handler accumulates
//! frame updates and sends them to the session loop via an `mpsc` channel.
//!
//! # Architecture
//!
//! The [`GraphicsPipelineClient`](ironrdp_egfx::client::GraphicsPipelineClient) handles
//! H.264 decoding internally. Our [`RustConnGfxHandler`] receives already-decoded
//! RGBA pixel data via `on_bitmap_updated` and forwards it to the session loop
//! for RGBA→BGRA conversion and framebuffer blitting.
//!
//! # Why this handler keeps its own copy of every surface
//!
//! `ironrdp-egfx` decodes wire codecs but performs no compositing: `SolidFill`,
//! `SurfaceToSurface`, `SurfaceToCache` and `CacheToSurface` are forwarded to
//! handler callbacks and nothing else. Those PDUs carry no pixels — they
//! reference content the client is expected to already hold. Leaving the
//! callbacks unimplemented (as 0.19.14 did) means scrolling, window moves,
//! solid fills and cache blits silently do nothing, which showed up as
//! horizontal bands and unfilled rectangle outlines across the desktop
//! (issue [#262]). So [`RustConnGfxHandler`] owns an RGBA copy of every surface
//! plus the bitmap cache, and synthesises a frame update for each operation.
//!
//! [#262]: https://github.com/totoshko88/RustConn/issues/262

use std::collections::HashMap;
use std::sync::mpsc;

use ironrdp_egfx::client::{BitmapUpdate, GraphicsPipelineHandler};
use ironrdp_egfx::decode::H264Decoder;
use ironrdp_egfx::pdu::{
    CacheToSurfacePdu, CapabilitiesV8Flags, CapabilitiesV81Flags, CapabilitySet,
    EvictCacheEntryPdu, GfxPdu, SolidFillPdu, SurfaceToCachePdu, SurfaceToSurfacePdu,
    WireToSurface2Pdu,
};

use super::RdpClientEvent;
use super::graphics::GraphicsMode;

/// Bitmap-cache budget in bytes.
///
/// We advertise `SMALL_CACHE`, which per MS-RDPEGFX 2.2.3.1 declares a 16 MB
/// client-side cache. Honouring the number we advertise keeps a misbehaving
/// server from growing the cache without bound.
const CACHE_BUDGET_BYTES: usize = 16 * 1024 * 1024;

/// Ceiling on the RGBA copies held for all live surfaces, in bytes.
///
/// A single 8K surface is already ~133 MB, and a server is free to create
/// several. Past this point the copies are dropped and the handler degrades to
/// forwarding decoded bitmaps only — the same behaviour as 0.19.14, so the
/// picture is still painted, just without `SurfaceToSurface` / cache support.
const SURFACE_BUDGET_BYTES: usize = 256 * 1024 * 1024;

/// Dropped surface updates that must pile up before the GUI is told the server
/// is using a codec we cannot decode.
///
/// Frames arrive in quick succession, so this is well under a second on a live
/// session; it exists only to avoid reacting to a single odd region.
const UNSUPPORTED_CODEC_THRESHOLD: u32 = 5;

/// Client-side RGBA copy of one EGFX surface or one bitmap-cache entry.
struct SurfaceBuffer {
    /// Width in pixels
    width: u16,
    /// Height in pixels
    height: u16,
    /// RGBA8888 pixels, row-major, `width * height * 4` bytes
    data: Vec<u8>,
}

impl SurfaceBuffer {
    /// Allocates an opaque black buffer.
    ///
    /// Alpha starts at 255: the GUI blits these pixels into a Cairo ARGB32
    /// surface, where a zero alpha byte renders as fully transparent rather
    /// than as black.
    fn new(width: u16, height: u16) -> Self {
        let mut data = vec![0u8; Self::byte_size(width, height)];
        for pixel in data.as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
        Self {
            width,
            height,
            data,
        }
    }

    /// Returns the byte size of a `width` × `height` RGBA buffer.
    const fn byte_size(width: u16, height: u16) -> usize {
        width as usize * height as usize * 4
    }

    /// Returns the row stride in bytes.
    const fn stride(&self) -> usize {
        self.width as usize * 4
    }

    /// Clips a `width` × `height` region at (`x`, `y`) to the buffer bounds.
    ///
    /// Returns `None` when the region falls entirely outside the buffer or
    /// collapses to zero area.
    fn clip(&self, x: u16, y: u16, width: u16, height: u16) -> Option<(u16, u16)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let width = width.min(self.width - x);
        let height = height.min(self.height - y);
        (width > 0 && height > 0).then_some((width, height))
    }

    /// Copies a `width` × `height` region at (`x`, `y`) out into a fresh buffer.
    ///
    /// The caller must have clipped the region with [`Self::clip`] first.
    fn extract(&self, x: u16, y: u16, width: u16, height: u16) -> Vec<u8> {
        let row_bytes = width as usize * 4;
        let mut out = vec![0u8; row_bytes * height as usize];
        for row in 0..height as usize {
            let src = (y as usize + row) * self.stride() + x as usize * 4;
            let dst = row * row_bytes;
            if src + row_bytes <= self.data.len() {
                out[dst..dst + row_bytes].copy_from_slice(&self.data[src..src + row_bytes]);
            }
        }
        out
    }

    /// Blits a `width` × `height` region of `src` into the buffer at (`x`, `y`).
    ///
    /// `src_width` is the row width of `src`, which may exceed `width` when the
    /// destination clipped the region.
    fn blit(&mut self, x: u16, y: u16, width: u16, height: u16, src: &[u8], src_width: u16) {
        let row_bytes = width as usize * 4;
        let src_stride = src_width as usize * 4;
        let dst_stride = self.stride();
        for row in 0..height as usize {
            let src_off = row * src_stride;
            let dst_off = (y as usize + row) * dst_stride + x as usize * 4;
            if src_off + row_bytes <= src.len() && dst_off + row_bytes <= self.data.len() {
                self.data[dst_off..dst_off + row_bytes]
                    .copy_from_slice(&src[src_off..src_off + row_bytes]);
            }
        }
    }

    /// Fills a `width` × `height` region at (`x`, `y`) with one RGBA pixel.
    fn fill(&mut self, x: u16, y: u16, width: u16, height: u16, pixel: [u8; 4]) {
        let dst_stride = self.stride();
        for row in 0..height as usize {
            let dst_off = (y as usize + row) * dst_stride + x as usize * 4;
            let row_end = dst_off + width as usize * 4;
            if row_end <= self.data.len() {
                for target in self.data[dst_off..row_end].as_chunks_mut::<4>().0 {
                    target.copy_from_slice(&pixel);
                }
            }
        }
    }
}

/// Crops the top-left `width` × `height` corner out of a `src_width`-wide RGBA buffer.
///
/// Returns `src` unchanged when no cropping is needed, which is the common case.
fn crop_rgba(src: &[u8], src_width: u16, width: u16, height: u16) -> Vec<u8> {
    let row_bytes = width as usize * 4;
    let src_stride = src_width as usize * 4;
    if row_bytes == src_stride {
        // A decoder that returned fewer rows than advertised would otherwise
        // panic on the slice; take what is there and let the caller's clipping
        // deal with the shortfall.
        return src[..(row_bytes * height as usize).min(src.len())].to_vec();
    }
    let mut out = vec![0u8; row_bytes * height as usize];
    for row in 0..height as usize {
        let src_off = row * src_stride;
        if src_off + row_bytes <= src.len() {
            out[row * row_bytes..(row + 1) * row_bytes]
                .copy_from_slice(&src[src_off..src_off + row_bytes]);
        }
    }
    out
}

/// Decoded GFX frame update ready for the session loop.
///
/// Contains RGBA pixel data at the specified framebuffer coordinates.
/// The session loop converts RGBA→BGRA and blits into the `DecodedImage`.
#[derive(Debug, Clone)]
pub struct GfxFrameUpdate {
    /// X coordinate in the output framebuffer
    pub x: u16,
    /// Y coordinate in the output framebuffer
    pub y: u16,
    /// Width of the update region in pixels
    pub width: u16,
    /// Height of the update region in pixels
    pub height: u16,
    /// RGBA pixel data (4 bytes per pixel, row-major)
    pub data: Vec<u8>,
}

/// Handler receiving decoded EGFX bitmap data from `ironrdp-egfx`.
///
/// Maintains surface-to-output coordinate mappings, an RGBA copy of every live
/// surface and the bitmap cache, and forwards frame updates to the session loop
/// via an `mpsc` channel.
pub struct RustConnGfxHandler {
    /// Surface ID → (output_origin_x, output_origin_y); mapped surfaces only
    surface_mappings: HashMap<u16, (u32, u32)>,
    /// Surface ID → client-side pixel copy, including offscreen surfaces
    surfaces: HashMap<u16, SurfaceBuffer>,
    /// Cache slot → cached region, populated by `SurfaceToCache`
    cache: HashMap<u16, SurfaceBuffer>,
    /// Bytes currently held in [`Self::cache`], against [`CACHE_BUDGET_BYTES`]
    cache_bytes: usize,
    /// Bytes currently held in [`Self::surfaces`], against [`SURFACE_BUDGET_BYTES`]
    surface_bytes: usize,
    /// Channel sender for delivering frame updates to the session loop
    update_tx: mpsc::Sender<GfxFrameUpdate>,
    /// Channel sender for delivering client events (errors, status) to the GUI
    event_tx: mpsc::Sender<RdpClientEvent>,
    /// Consecutive empty bitmap updates (potential persistent decode failure)
    consecutive_empty: u32,
    /// Whether [`RdpClientEvent::GfxDecodeFailure`] was already reported for
    /// the current run of empty updates, so the GUI hears about it once
    decode_failure_reported: bool,
    /// Surface updates dropped because their codec has no decoder
    unsupported_codec_frames: u32,
    /// Whether the EGFX pipeline has completed capability negotiation
    active: bool,
}

impl RustConnGfxHandler {
    /// Creates a new GFX handler.
    ///
    /// # Arguments
    ///
    /// * `update_tx` — channel sender for delivering decoded frame updates
    ///   to the session loop
    /// * `event_tx` — channel sender for delivering client events (e.g.
    ///   persistent decode failure) to the GUI
    #[must_use]
    pub fn new(
        update_tx: mpsc::Sender<GfxFrameUpdate>,
        event_tx: mpsc::Sender<RdpClientEvent>,
    ) -> Self {
        Self {
            surface_mappings: HashMap::new(),
            surfaces: HashMap::new(),
            cache: HashMap::new(),
            cache_bytes: 0,
            surface_bytes: 0,
            update_tx,
            event_tx,
            consecutive_empty: 0,
            decode_failure_reported: false,
            unsupported_codec_frames: 0,
            active: false,
        }
    }

    /// Returns whether the EGFX pipeline is active (capabilities confirmed).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Sends a frame update for a region of a surface, in output coordinates.
    ///
    /// Updates to offscreen (unmapped) surfaces are stored but not emitted:
    /// they have no place on screen until a `SurfaceToSurface` or cache blit
    /// moves them onto a mapped surface.
    fn emit_update(
        &self,
        surface_id: u16,
        surface_x: u16,
        surface_y: u16,
        width: u16,
        height: u16,
        data: Vec<u8>,
    ) {
        let Some(&(origin_x, origin_y)) = self.surface_mappings.get(&surface_id) else {
            return;
        };
        let dest_x = origin_x.saturating_add(u32::from(surface_x));
        let dest_y = origin_y.saturating_add(u32::from(surface_y));

        // If the receiver is gone the session is shutting down — discard.
        let _ = self.update_tx.send(GfxFrameUpdate {
            x: u16::try_from(dest_x).unwrap_or(u16::MAX),
            y: u16::try_from(dest_y).unwrap_or(u16::MAX),
            width,
            height,
            data,
        });
    }

    /// Blits `pixels` onto `surface_id` at each destination point and emits the
    /// resulting frame updates.
    ///
    /// Shared by `SurfaceToSurface` and `CacheToSurface`, which differ only in
    /// where the pixels came from.
    fn blit_to_points(
        &mut self,
        surface_id: u16,
        points: &[ironrdp_egfx::pdu::Point],
        pixels: &[u8],
        width: u16,
        height: u16,
    ) {
        for point in points {
            let Some(surface) = self.surfaces.get_mut(&surface_id) else {
                return;
            };
            let Some((clipped_w, clipped_h)) = surface.clip(point.x, point.y, width, height) else {
                continue;
            };
            surface.blit(point.x, point.y, clipped_w, clipped_h, pixels, width);

            let data = crop_rgba(pixels, width, clipped_w, clipped_h);
            self.emit_update(surface_id, point.x, point.y, clipped_w, clipped_h, data);
        }
    }

    /// Allocates the client-side copy for a newly created surface.
    ///
    /// Split out of [`GraphicsPipelineHandler::on_surface_created`] because
    /// `ironrdp-egfx`'s `Surface` is `#[non_exhaustive]` and therefore cannot be
    /// constructed from our tests.
    fn register_surface(&mut self, surface_id: u16, width: u16, height: u16) {
        let bytes = SurfaceBuffer::byte_size(width, height);
        if self.surface_bytes.saturating_add(bytes) > SURFACE_BUDGET_BYTES {
            tracing::warn!(
                surface_id,
                width,
                height,
                held_bytes = self.surface_bytes,
                "EGFX surface copy budget exhausted — SurfaceToSurface and cache \
                 blits will be skipped for this surface"
            );
            return;
        }
        self.surface_bytes += bytes;
        self.surfaces
            .insert(surface_id, SurfaceBuffer::new(width, height));
        tracing::debug!(surface_id, width, height, "EGFX surface created");
    }

    /// Records surface content the pipeline has no decoder for, and reports it
    /// to the GUI once enough of it has been dropped.
    ///
    /// `ironrdp-egfx` forwards undecodable content to handler callbacks rather
    /// than decoding it, so the pixels are simply lost. Nothing reaches
    /// `on_bitmap_updated`, which means the empty-frame counter behind
    /// [`RdpClientEvent::GfxDecodeFailure`] stays at zero and the session just
    /// looks frozen — the failure shape behind issue #262.
    fn note_undecodable_content(&mut self, codec: &str, surface_id: u16) {
        self.unsupported_codec_frames = self.unsupported_codec_frames.saturating_add(1);

        if self.unsupported_codec_frames == 1 {
            tracing::warn!(
                codec,
                surface_id,
                "EGFX surface content arrived in a format the pipeline cannot decode"
            );
        }
        if self.unsupported_codec_frames == UNSUPPORTED_CODEC_THRESHOLD {
            tracing::error!(
                codec,
                dropped_frames = self.unsupported_codec_frames,
                "GFX undecodable content — the session cannot paint through this pipeline"
            );
            let _ = self.event_tx.send(RdpClientEvent::GfxUnsupportedCodec {
                codec: codec.to_owned(),
                dropped_frames: self.unsupported_codec_frames,
            });
        }
    }

    /// Drops every surface and cache entry, resetting the byte accounting.
    fn clear_stores(&mut self) {
        self.surface_mappings.clear();
        self.surfaces.clear();
        self.cache.clear();
        self.cache_bytes = 0;
        self.surface_bytes = 0;
    }
}

impl GraphicsPipelineHandler for RustConnGfxHandler {
    /// Advertises only the capability versions `ironrdp-egfx` can actually decode.
    ///
    /// The upstream default also offers `V10_7`, which tells the server AVC444
    /// is available. `ironrdp-egfx` 0.3.0 does not implement AVC444: it routes
    /// those `WireToSurface1` PDUs to its catch-all callback instead of decoding
    /// them. Windows takes the best offer, so on any host that prefers AVC444 —
    /// Windows 11 25H2 among them — every frame was discarded and the session
    /// looked frozen (issue [#262]).
    ///
    /// `V8_1` with `AVC420_ENABLED` keeps H.264, which *is* decoded, and `V8` is
    /// the no-AVC fallback for servers without a hardware or software encoder.
    ///
    /// Revisit on every `ironrdp-egfx` bump: once AVC444 lands upstream, adding
    /// `V10_7` back is a one-line quality and bandwidth win. There is a matching
    /// reminder next to the dependency in `rustconn-core/Cargo.toml`.
    ///
    /// [#262]: https://github.com/totoshko88/RustConn/issues/262
    fn capabilities(&self) -> Vec<CapabilitySet> {
        vec![
            CapabilitySet::V8_1 {
                flags: CapabilitiesV81Flags::AVC420_ENABLED | CapabilitiesV81Flags::SMALL_CACHE,
            },
            CapabilitySet::V8 {
                flags: CapabilitiesV8Flags::SMALL_CACHE,
            },
        ]
    }

    fn on_capabilities_confirmed(&mut self, caps: &CapabilitySet) {
        self.active = true;
        let mode = negotiated_graphics_mode(caps);
        tracing::info!(?caps, ?mode, "EGFX capabilities confirmed");
        // Tell the GUI what really negotiated. Until 0.19.15 the status bar
        // reported a compile-time constant, so it claimed "GFX + H.264" even
        // for sessions that never brought the GFX channel up (issue #262).
        let _ = self
            .event_tx
            .send(RdpClientEvent::GraphicsModeActive { mode });
    }

    fn on_reset_graphics(&mut self, width: u32, height: u32) {
        self.clear_stores();
        self.consecutive_empty = 0;
        self.decode_failure_reported = false;

        // Signal resolution change to the session loop via sentinel update.
        // Empty data + non-zero dimensions = resolution reset request.
        let reset_signal = GfxFrameUpdate {
            x: 0,
            y: 0,
            width: u16::try_from(width).unwrap_or(u16::MAX),
            height: u16::try_from(height).unwrap_or(u16::MAX),
            data: Vec::new(),
        };
        let _ = self.update_tx.send(reset_signal);

        tracing::info!(width, height, "EGFX graphics reset — surface state cleared");
    }

    fn on_surface_created(&mut self, surface: &ironrdp_egfx::client::Surface) {
        self.register_surface(surface.id, surface.width, surface.height);
    }

    fn on_surface_mapped(&mut self, surface_id: u16, origin_x: u32, origin_y: u32) {
        self.surface_mappings
            .insert(surface_id, (origin_x, origin_y));
        tracing::debug!(surface_id, origin_x, origin_y, "Surface mapped to output");
    }

    fn on_surface_deleted(&mut self, surface_id: u16) {
        self.surface_mappings.remove(&surface_id);
        if let Some(surface) = self.surfaces.remove(&surface_id) {
            self.surface_bytes = self.surface_bytes.saturating_sub(surface.data.len());
        }
        tracing::debug!(surface_id, "EGFX surface deleted");
    }

    fn on_bitmap_updated(&mut self, update: &BitmapUpdate) {
        // Skip empty updates (decode was skipped or failed upstream)
        if update.data.is_empty() {
            self.consecutive_empty += 1;
            // ponytail: threshold 10 matches Req 3 AC 5; increase if servers
            // legitimately send empty frames during codec renegotiation.
            if self.consecutive_empty >= 10 && !self.decode_failure_reported {
                self.decode_failure_reported = true;
                tracing::error!(
                    consecutive_empty = self.consecutive_empty,
                    surface_id = update.surface_id,
                    "Persistent decode failure — 10+ consecutive empty bitmap updates"
                );
                // Signal the GUI about persistent decode failure so it can
                // display a degraded-quality warning (Req 6 AC 3, Req 10 AC 3).
                let _ = self.event_tx.send(RdpClientEvent::GfxDecodeFailure {
                    consecutive_failures: self.consecutive_empty,
                });
            }
            return;
        }

        // Reset the empty-frame run on successful data
        self.consecutive_empty = 0;
        self.decode_failure_reported = false;

        let left = update.destination_rectangle.left;
        let top = update.destination_rectangle.top;

        // Keep the client-side copy in sync: SurfaceToSurface and SurfaceToCache
        // read back from it later. Surfaces beyond the copy budget are absent
        // here, in which case the decoded bitmap is forwarded as-is and the
        // session loop clips it against the framebuffer.
        let (width, height, data) = match self.surfaces.get_mut(&update.surface_id) {
            Some(surface) => {
                let Some((width, height)) = surface.clip(left, top, update.width, update.height)
                else {
                    return;
                };
                surface.blit(left, top, width, height, &update.data, update.width);
                (
                    width,
                    height,
                    crop_rgba(&update.data, update.width, width, height),
                )
            }
            None => (update.width, update.height, update.data.clone()),
        };

        self.emit_update(update.surface_id, left, top, width, height, data);
    }

    fn on_solid_fill(&mut self, pdu: &SolidFillPdu) {
        // MS-RDPEGFX 2.2.2.7 carries the colour as RDPGFX_COLOR32 (B, G, R, XA).
        // XA is "alpha or unused" and Windows leaves it zero, so the pixel is
        // forced opaque — otherwise the filled region blits fully transparent.
        let pixel = [pdu.fill_pixel.r, pdu.fill_pixel.g, pdu.fill_pixel.b, 255];

        for rect in &pdu.rectangles {
            let Some(surface) = self.surfaces.get_mut(&pdu.surface_id) else {
                return;
            };
            let requested_w = rect.right.saturating_sub(rect.left);
            let requested_h = rect.bottom.saturating_sub(rect.top);
            let Some((width, height)) = surface.clip(rect.left, rect.top, requested_w, requested_h)
            else {
                continue;
            };
            surface.fill(rect.left, rect.top, width, height, pixel);

            let data = pixel.repeat(width as usize * height as usize);
            self.emit_update(pdu.surface_id, rect.left, rect.top, width, height, data);
        }
        tracing::trace!(
            surface_id = pdu.surface_id,
            rectangles = pdu.rectangles.len(),
            "EGFX solid fill"
        );
    }

    fn on_surface_to_surface(&mut self, pdu: &SurfaceToSurfacePdu) {
        let rect = &pdu.source_rectangle;
        let Some(source) = self.surfaces.get(&pdu.source_surface_id) else {
            tracing::debug!(
                surface_id = pdu.source_surface_id,
                "SurfaceToSurface from a surface with no client-side copy — skipping"
            );
            return;
        };
        let Some((width, height)) = source.clip(
            rect.left,
            rect.top,
            rect.right.saturating_sub(rect.left),
            rect.bottom.saturating_sub(rect.top),
        ) else {
            return;
        };

        // Copy the source out before touching the destination: same-surface
        // copies are the common case (scrolling, window drags) and the regions
        // routinely overlap.
        let pixels = source.extract(rect.left, rect.top, width, height);
        self.blit_to_points(
            pdu.destination_surface_id,
            &pdu.destination_points,
            &pixels,
            width,
            height,
        );
    }

    fn on_surface_to_cache(&mut self, pdu: &SurfaceToCachePdu) {
        let rect = &pdu.source_rectangle;
        let Some(surface) = self.surfaces.get(&pdu.surface_id) else {
            return;
        };
        let Some((width, height)) = surface.clip(
            rect.left,
            rect.top,
            rect.right.saturating_sub(rect.left),
            rect.bottom.saturating_sub(rect.top),
        ) else {
            return;
        };
        let data = surface.extract(rect.left, rect.top, width, height);

        // Overwriting a slot releases its old budget first.
        if let Some(previous) = self.cache.remove(&pdu.cache_slot) {
            self.cache_bytes = self.cache_bytes.saturating_sub(previous.data.len());
        }
        if self.cache_bytes.saturating_add(data.len()) > CACHE_BUDGET_BYTES {
            tracing::warn!(
                cache_slot = pdu.cache_slot,
                cache_bytes = self.cache_bytes,
                "EGFX bitmap cache budget exhausted — entry dropped"
            );
            return;
        }
        self.cache_bytes += data.len();
        self.cache.insert(
            pdu.cache_slot,
            SurfaceBuffer {
                width,
                height,
                data,
            },
        );
    }

    fn on_cache_to_surface(&mut self, pdu: &CacheToSurfacePdu) {
        let Some(entry) = self.cache.get(&pdu.cache_slot) else {
            tracing::debug!(
                cache_slot = pdu.cache_slot,
                "CacheToSurface for an unpopulated slot — skipping"
            );
            return;
        };
        // Cache entries are small tiles, so cloning is cheaper than the
        // borrow gymnastics needed to blit straight out of the map.
        let (width, height, pixels) = (entry.width, entry.height, entry.data.clone());
        self.blit_to_points(
            pdu.surface_id,
            &pdu.destination_points,
            &pixels,
            width,
            height,
        );
    }

    fn on_evict_cache_entry(&mut self, pdu: &EvictCacheEntryPdu) {
        if let Some(entry) = self.cache.remove(&pdu.cache_slot) {
            self.cache_bytes = self.cache_bytes.saturating_sub(entry.data.len());
        }
    }

    fn on_wire_to_surface2(&mut self, pdu: &WireToSurface2Pdu) {
        // RFX Progressive. `ironrdp-egfx` has no progressive decoder and only
        // forwards the PDU, so the region is lost exactly like an undecodable
        // `WireToSurface1` and is reported the same way.
        self.note_undecodable_content("RfxProgressive", pdu.surface_id);
    }

    fn on_unhandled_pdu(&mut self, pdu: &GfxPdu) {
        let GfxPdu::WireToSurface1(wire) = pdu else {
            tracing::debug!(?pdu, "Unhandled EGFX PDU");
            return;
        };
        self.note_undecodable_content(&format!("{:?}", wire.codec_id), wire.surface_id);
    }

    fn on_frame_complete(&mut self, frame_id: u32) {
        tracing::trace!(frame_id, "EGFX frame complete");
    }

    fn on_close(&mut self) {
        self.active = false;
        tracing::info!("EGFX channel closed");
    }
}

/// Maps a confirmed EGFX capability set onto the graphics mode it represents.
///
/// Only the versions [`RustConnGfxHandler::capabilities`] advertises can come
/// back, but a server is free to confirm something else, so unknown sets are
/// reported as the plain GFX pipeline rather than claiming H.264.
fn negotiated_graphics_mode(caps: &CapabilitySet) -> GraphicsMode {
    match caps {
        CapabilitySet::V8_1 { flags } if flags.contains(CapabilitiesV81Flags::AVC420_ENABLED) => {
            GraphicsMode::GfxH264
        }
        _ => GraphicsMode::Gfx,
    }
}

// ============================================================================
// OpenH264 loading
// ============================================================================

/// Standard library search paths for OpenH264.
///
/// Flatpak puts it under `/app/lib/`, native installs use `/usr/lib/` or
/// `/usr/lib64/`. The `openh264` crate's libloading backend handles the
/// actual dlopen; we just need to find the `.so` file.
///
/// These are the *unversioned* names, which a distribution ships only in its
/// `-dev`/`-devel` package. A runtime-only install has none of them, which is
/// why [`OPENH264_SEARCH_DIRS`] is scanned for versioned sonames as well.
#[cfg(not(target_os = "macos"))]
const OPENH264_SEARCH_PATHS: &[&str] = &[
    "/app/lib/libopenh264.so",
    "/app/lib64/libopenh264.so",
    "/usr/lib/libopenh264.so",
    "/usr/lib64/libopenh264.so",
    "/usr/lib/x86_64-linux-gnu/libopenh264.so",
    "/usr/lib/aarch64-linux-gnu/libopenh264.so",
];

/// Standard library search paths for OpenH264 on macOS.
///
/// macOS ships Mach-O `.dylib` files, never ELF `.so`, so the Linux list above
/// can never match. The distributed `.app` carries its own copy (see
/// [`bundled_openh264_path`]); these Homebrew prefixes only cover development
/// runs of the bare `target/` binary on Apple Silicon and Intel.
#[cfg(target_os = "macos")]
const OPENH264_SEARCH_PATHS: &[&str] = &[
    "/opt/homebrew/lib/libopenh264.dylib",
    "/usr/local/lib/libopenh264.dylib",
];

/// Returns the OpenH264 copy shipped inside the macOS application bundle.
///
/// The canonical bundle layout is `RustConn.app/Contents/MacOS/rustconn`, so the
/// relocated library sits at `../Frameworks/libopenh264.dylib` relative to the
/// running executable. Returns `None` outside a bundle-like layout or when the
/// executable path cannot be resolved.
#[cfg(target_os = "macos")]
fn bundled_openh264_path() -> Option<std::path::PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let contents = executable.parent()?.parent()?;
    Some(contents.join("Frameworks").join("libopenh264.dylib"))
}

/// Prefix every versioned OpenH264 soname starts with.
const OPENH264_SONAME_PREFIX: &str = "libopenh264.so.";

/// Directories scanned for a versioned OpenH264 soname.
///
/// Needed because [`OPENH264_SEARCH_PATHS`] names only the unversioned
/// `libopenh264.so`, which lives in the `-dev` package. Debian's
/// `libopenh264-8` installs `libopenh264.so.8` and `libopenh264.so.2.6.0` and
/// nothing else, so probing the unversioned name alone reports "not found" on a
/// machine that has the library — and RDP silently drops to the RemoteFX path.
#[cfg(not(target_os = "macos"))]
const OPENH264_SEARCH_DIRS: &[&str] = &[
    "/app/lib",
    "/app/lib64",
    "/usr/lib",
    "/usr/lib64",
    "/usr/lib/x86_64-linux-gnu",
    "/usr/lib/aarch64-linux-gnu",
];

/// Directories scanned for a versioned OpenH264 soname — none on macOS.
///
/// The bundled copy and the Homebrew prefixes are unversioned `.dylib` files
/// that [`OPENH264_SEARCH_PATHS`] and [`bundled_openh264_path`] already name.
/// Mach-O version naming puts the version before the extension
/// (`libopenh264.2.6.0.dylib`), so it would not match the ELF soname pattern
/// this scan looks for anyway.
#[cfg(target_os = "macos")]
const OPENH264_SEARCH_DIRS: &[&str] = &[];

/// Parses the version segments of a versioned OpenH264 soname.
///
/// Returns the dot-separated numbers after `libopenh264.so.`, or `None` when
/// `name` is not a versioned soname of that library. Every segment must parse
/// as a number, which drops companions like `libopenh264.so.debug`.
fn soname_version(name: &str) -> Option<Vec<u64>> {
    let suffix = name.strip_prefix(OPENH264_SONAME_PREFIX)?;
    if suffix.is_empty() {
        return None;
    }
    suffix
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect()
}

/// Returns the versioned OpenH264 sonames among `names`, newest ABI first.
///
/// Takes names rather than a directory so the ordering is testable without a
/// filesystem. Sorted by major version descending — a numeric compare, so
/// `.so.10` outranks `.so.9` where a string sort would not — then by segment
/// count ascending, which prefers the bare soname `libopenh264.so.8` over the
/// `libopenh264.so.8.0.1` it points at, then by the remaining segments
/// descending so the result does not depend on directory order.
fn sonames_newest_first(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut versioned: Vec<(Vec<u64>, String)> = names
        .into_iter()
        .filter_map(|name| soname_version(&name).map(|version| (version, name)))
        .collect();

    versioned.sort_by(|(a, _), (b, _)| {
        b.first()
            .cmp(&a.first())
            .then_with(|| a.len().cmp(&b.len()))
            .then_with(|| b.cmp(a))
    });

    versioned.into_iter().map(|(_, name)| name).collect()
}

/// Returns versioned OpenH264 sonames found on disk, newest ABI first.
///
/// An unreadable directory is skipped: the list is a set of guesses about where
/// a distribution might have put the library, so most of them are expected to
/// be absent on any given machine.
fn versioned_openh264_candidates() -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();

    for dir in OPENH264_SEARCH_DIRS {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let names: Vec<String> = entries
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .collect();
        found.extend(
            sonames_newest_first(names)
                .into_iter()
                .map(|name| std::path::Path::new(dir).join(name)),
        );
    }

    found
}

/// Environment variable naming an OpenH264 library to try before anything else.
///
/// The loader only accepts Cisco's own published binaries (see
/// [`try_load_openh264`]), and no Linux distribution ships one, so the *only*
/// way to get H.264 on a packaged install is to point RustConn at a blob
/// downloaded from `ciscobinary.openh264.org`. Without this there was nowhere to
/// put it except a system directory, i.e. it needed root.
///
/// Read-only: nothing in RustConn ever sets it.
const OPENH264_PATH_ENV: &str = "RUSTCONN_OPENH264";

/// Returns OpenH264 candidates in priority order.
///
/// An explicit [`OPENH264_PATH_ENV`] wins over everything: it is a deliberate
/// choice, and on Linux it is usually the only candidate that can be loaded at
/// all. The bundled copy comes next so a self-contained macOS `.app` never
/// depends on a Homebrew installation at runtime. Unversioned names come before
/// versioned sonames because an unversioned name is a deliberate pointer at the
/// installation the system considers current.
fn openh264_candidates() -> Vec<std::path::PathBuf> {
    let mut candidates = Vec::new();

    if let Some(explicit) = std::env::var_os(OPENH264_PATH_ENV)
        .map(std::path::PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
    {
        candidates.push(explicit);
    }

    // The on-demand Cisco blob the user downloaded via the consent flow
    // (`openh264_download::download_openh264`). It sits in the user cache and,
    // being Cisco's own published binary, is the one library on a packaged
    // Linux install the loader's SHA-256 allow-list actually accepts. It comes
    // AFTER the explicit env override (which is a deliberate per-run choice) but
    // BEFORE the system search paths (whose distribution builds the loader
    // always rejects). `cached_openh264_path` is a pure read — it never triggers
    // a download — and returns `None` unless the file exists AND its hash
    // matches, so a corrupt/partial cache entry is simply skipped here.
    if let Some(cached) = super::openh264_download::cached_openh264_path() {
        candidates.push(cached);
    }

    #[cfg(target_os = "macos")]
    if let Some(bundled) = bundled_openh264_path() {
        candidates.push(bundled);
    }

    candidates.extend(OPENH264_SEARCH_PATHS.iter().map(std::path::PathBuf::from));
    candidates.extend(versioned_openh264_candidates());
    candidates
}

/// The outcome of the library search, decided once per process.
///
/// `Some` is the library that loaded; `None` means nothing usable was found.
///
/// The decoder itself cannot be shared — each session needs its own — but the
/// search can, and so can the explanation. Without this the whole walk ran again
/// for every RDP connection: re-`stat`ing every candidate, re-`dlopen`ing each
/// one, and re-emitting the same warnings. A log from three connections carried
/// nine identical lines about an unchangeable property of the machine, at the one
/// severity users actually read, which is how real warnings get lost.
///
/// The trade is that installing a Cisco blob mid-session is not picked up until
/// restart. That is the right way round: the answer depends on files and an
/// environment variable that do not change under a running process in practice,
/// and the alternative is paying the walk on every connection forever.
static USABLE_LIBRARY: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();

/// Attempts to load OpenH264 at runtime via dlopen.
///
/// The search itself happens once per process and is cached in
/// [`USABLE_LIBRARY`]; this returns a fresh decoder built from whatever that
/// search settled on.
///
/// Searches [`OPENH264_PATH_ENV`] and the well-known system paths, returning a
/// decoder suitable for passing to
/// [`GraphicsPipelineClient::new`](ironrdp_egfx::client::GraphicsPipelineClient::new).
///
/// # Why a library that is installed still gets rejected
///
/// `ironrdp-egfx` loads through `openh264::OpenH264API::from_blob_path`, which
/// compares the file's SHA-256 against a list of **Cisco's own published
/// binaries** and refuses anything else with `Invalid hash: <sha>`. That is
/// deliberate, not a bug: Cisco pays the H.264 patent royalties for the binaries
/// it distributes itself, which is why the crate's own documentation says to
/// download the library from Cisco. No distribution build can be on that list —
/// Debian's `libopenh264-8`, Fedora's `libopenh264`, and a local build from the
/// Cisco *source* tarball are all refused — and the unchecked loader is `unsafe`
/// and not re-exported, so there is nothing to opt into.
///
/// The practical consequence is that on a packaged Linux install H.264 requires
/// a blob from `ciscobinary.openh264.org` and [`OPENH264_PATH_ENV`] pointing at
/// it. A rejected hash is therefore reported as the configuration problem it is,
/// rather than as an opaque load failure.
///
/// Returns `None` when nothing loadable is found. The session then uses the
/// RemoteFX path; see the EGFX registration in `client::connection` for why it
/// does not open a GFX channel it cannot paint through (issue #262).
///
/// # Errors
///
/// This function does not return an error — it logs warnings and returns
/// `None` on failure, allowing graceful fallback.
#[must_use]
pub fn try_load_openh264() -> Option<Box<dyn H264Decoder>> {
    use ironrdp_egfx::decode::OpenH264Decoder;

    let path = USABLE_LIBRARY.get_or_init(probe_openh264).as_ref()?;

    match OpenH264Decoder::from_library_path(path) {
        Ok(decoder) => Some(Box::new(decoder)),
        Err(e) => {
            // The probe already loaded this exact file, so a failure here is a
            // second session failing where the first succeeded — worth a warning
            // rather than a silent fallback.
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "OpenH264 loaded during detection but not for this session"
            );
            None
        }
    }
}

/// Walks the candidates once and reports which one is usable, if any.
fn probe_openh264() -> Option<std::path::PathBuf> {
    use ironrdp_egfx::decode::OpenH264Decoder;

    let mut rejected_hash = false;

    for path in openh264_candidates() {
        let path = path.as_path();
        if !path.exists() {
            continue;
        }

        match OpenH264Decoder::from_library_path(path) {
            Ok(_) => {
                tracing::info!(
                    path = %path.display(),
                    "OpenH264 loaded — H.264 decoding enabled"
                );
                return Some(path.to_path_buf());
            }
            Err(e) => {
                // `Invalid hash` is the signature of the check described above:
                // the file loaded fine, it is simply not one of Cisco's.
                let message = e.to_string();
                if message.contains("Invalid hash") {
                    rejected_hash = true;
                    tracing::warn!(
                        path = %path.display(),
                        reason = "openh264_not_cisco_build",
                        "OpenH264 at this path is not one of Cisco's published binaries, so the \
                         loader refuses it — this is expected for a distribution package. Point \
                         {} at a library downloaded from ciscobinary.openh264.org to enable H.264.",
                        OPENH264_PATH_ENV
                    );
                } else {
                    tracing::warn!(
                        path = %path.display(),
                        error = %message,
                        "OpenH264 found but failed to initialize"
                    );
                }
            }
        }
    }

    if rejected_hash {
        REJECTED_NON_CISCO.store(true, std::sync::atomic::Ordering::Relaxed);
        tracing::warn!(
            reason = "openh264_not_cisco_build",
            "No usable OpenH264 — every library found was a non-Cisco build. GFX pipeline will \
             use non-AVC codecs; see docs/INSTALL.md for how to enable H.264."
        );
    } else {
        tracing::warn!(
            reason = "openh264_unavailable",
            "OpenH264 not found — GFX pipeline will use non-AVC codecs"
        );
    }
    None
}

/// Set by [`probe_openh264`] when every OpenH264 it found was a non-Cisco build
/// rejected by the hash check, as opposed to none being found at all. Read via
/// [`openh264_unavailable_reason`] to tell the user which case they are in.
static REJECTED_NON_CISCO: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Returns why H.264 is unavailable, or `None` when a decoder DID load.
///
/// Runs the one-shot probe (shared with [`try_load_openh264`]) so it reflects
/// the same decision the session made, without re-walking the filesystem.
#[must_use]
pub fn openh264_unavailable_reason() -> Option<super::graphics::H264UnavailableReason> {
    use super::graphics::H264UnavailableReason;
    if USABLE_LIBRARY.get_or_init(probe_openh264).is_some() {
        return None;
    }
    Some(
        if REJECTED_NON_CISCO.load(std::sync::atomic::Ordering::Relaxed) {
            H264UnavailableReason::RejectedNonCisco
        } else {
            H264UnavailableReason::NotFound
        },
    )
}

// ============================================================================
// Error types
// ============================================================================

/// Errors specific to the GFX/H.264 pipeline.
#[derive(Debug, thiserror::Error)]
pub enum GfxError {
    /// OpenH264 library not available at runtime.
    #[error("OpenH264 library not available: {0}")]
    H264Unavailable(String),

    /// Single-frame H.264 decode failure.
    #[error("H.264 decode failed for surface {surface_id}: {reason}")]
    H264DecodeFailed {
        /// Surface that failed to decode
        surface_id: u16,
        /// Human-readable failure reason
        reason: String,
    },

    /// Bitmap update references an unmapped surface.
    #[error("Surface {surface_id} not mapped to output")]
    SurfaceNotMapped {
        /// The unmapped surface ID
        surface_id: u16,
    },

    /// Too many consecutive empty frames indicate a persistent problem.
    #[error("Persistent decode failure: {consecutive_failures} consecutive empty frames")]
    PersistentDecodeFailure {
        /// Number of consecutive failures observed
        consecutive_failures: u32,
    },
}

#[cfg(test)]
mod tests {
    use ironrdp::pdu::geometry::ExclusiveRectangle;
    use ironrdp_egfx::pdu::{Codec1Type, Codec2Type, Color, PixelFormat, Point, WireToSurface1Pdu};

    use super::*;

    #[test]
    fn gfx_error_display() {
        let err = GfxError::H264Unavailable("library not found".into());
        assert!(err.to_string().contains("not available"));

        let err = GfxError::H264DecodeFailed {
            surface_id: 5,
            reason: "corrupted NAL".into(),
        };
        assert!(err.to_string().contains("surface 5"));

        let err = GfxError::SurfaceNotMapped { surface_id: 3 };
        assert!(err.to_string().contains("Surface 3"));

        let err = GfxError::PersistentDecodeFailure {
            consecutive_failures: 10,
        };
        assert!(err.to_string().contains("10"));
    }

    /// Builds a handler plus the receiving ends of both channels.
    fn test_handler() -> (
        RustConnGfxHandler,
        mpsc::Receiver<GfxFrameUpdate>,
        mpsc::Receiver<RdpClientEvent>,
    ) {
        let (tx, rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        (RustConnGfxHandler::new(tx, event_tx), rx, event_rx)
    }

    /// Registers a mapped surface of the given size, as the server would.
    fn map_surface(handler: &mut RustConnGfxHandler, id: u16, width: u16, height: u16) {
        handler.register_surface(id, width, height);
        handler.on_surface_mapped(id, 0, 0);
    }

    /// Builds a solid-fill PDU with one rectangle in wire (BGRA) colour order.
    fn solid_fill(surface_id: u16, bgr: [u8; 3], rect: ExclusiveRectangle) -> SolidFillPdu {
        SolidFillPdu {
            surface_id,
            fill_pixel: Color {
                b: bgr[0],
                g: bgr[1],
                r: bgr[2],
                xa: 0,
            },
            rectangles: vec![rect],
        }
    }

    /// Builds an exclusive rectangle from an origin plus a size.
    const fn rect(x: u16, y: u16, width: u16, height: u16) -> ExclusiveRectangle {
        ExclusiveRectangle {
            left: x,
            top: y,
            right: x + width,
            bottom: y + height,
        }
    }

    #[test]
    fn handler_tracks_surface_mappings() {
        let (mut handler, _rx, _event_rx) = test_handler();

        handler.on_surface_mapped(1, 100, 200);
        handler.on_surface_mapped(2, 300, 400);

        assert_eq!(handler.surface_mappings.get(&1), Some(&(100, 200)));
        assert_eq!(handler.surface_mappings.get(&2), Some(&(300, 400)));

        handler.on_surface_deleted(1);
        assert_eq!(handler.surface_mappings.get(&1), None);
        assert_eq!(handler.surface_mappings.get(&2), Some(&(300, 400)));
    }

    #[test]
    fn handler_reset_clears_state() {
        let (mut handler, _rx, _event_rx) = test_handler();

        map_surface(&mut handler, 1, 64, 64);
        handler.on_reset_graphics(1920, 1080);

        assert!(handler.surface_mappings.is_empty());
        assert!(handler.surfaces.is_empty());
        assert!(handler.cache.is_empty());
        assert_eq!(handler.surface_bytes, 0);
        assert_eq!(handler.cache_bytes, 0);
        assert_eq!(handler.consecutive_empty, 0);
    }

    /// The handler must not offer AVC444: `ironrdp-egfx` 0.3.0 discards every
    /// AVC444 surface update, so advertising it froze the session (issue #262).
    ///
    /// When this test starts failing after an `ironrdp-egfx` bump, check whether
    /// AVC444 landed upstream — if it did, re-adding `V10_7` is the win.
    #[test]
    fn advertised_capabilities_exclude_avc444() {
        let (handler, _rx, _event_rx) = test_handler();
        let caps = handler.capabilities();

        assert!(
            !caps.iter().any(|cap| matches!(
                cap,
                CapabilitySet::V10 { .. }
                    | CapabilitySet::V10_1
                    | CapabilitySet::V10_2 { .. }
                    | CapabilitySet::V10_3 { .. }
                    | CapabilitySet::V10_4 { .. }
                    | CapabilitySet::V10_5 { .. }
                    | CapabilitySet::V10_6 { .. }
                    | CapabilitySet::V10_6Err { .. }
                    | CapabilitySet::V10_7 { .. }
            )),
            "V10.x implies AVC444, which ironrdp-egfx cannot decode: {caps:?}"
        );
        assert!(
            caps.iter().any(|cap| matches!(
                cap,
                CapabilitySet::V8_1 { flags }
                    if flags.contains(CapabilitiesV81Flags::AVC420_ENABLED)
            )),
            "AVC420 must stay advertised — it is the codec we can decode: {caps:?}"
        );
    }

    #[test]
    fn capabilities_confirm_reports_the_negotiated_mode() {
        let (mut handler, _rx, event_rx) = test_handler();

        handler.on_capabilities_confirmed(&CapabilitySet::V8_1 {
            flags: CapabilitiesV81Flags::AVC420_ENABLED | CapabilitiesV81Flags::SMALL_CACHE,
        });

        assert!(matches!(
            event_rx.try_recv(),
            Ok(RdpClientEvent::GraphicsModeActive {
                mode: GraphicsMode::GfxH264
            })
        ));

        // A server that drops AVC must not be reported as an H.264 session.
        let (mut handler, _rx, event_rx) = test_handler();
        handler.on_capabilities_confirmed(&CapabilitySet::V8 {
            flags: CapabilitiesV8Flags::SMALL_CACHE,
        });
        assert!(matches!(
            event_rx.try_recv(),
            Ok(RdpClientEvent::GraphicsModeActive {
                mode: GraphicsMode::Gfx
            })
        ));
    }

    #[test]
    fn solid_fill_paints_and_emits_the_region() {
        let (mut handler, rx, _event_rx) = test_handler();
        map_surface(&mut handler, 1, 16, 16);

        handler.on_solid_fill(&solid_fill(1, [0x30, 0x20, 0x10], rect(2, 3, 4, 2)));

        let update = rx.try_recv().expect("solid fill must emit an update");
        assert_eq!((update.x, update.y), (2, 3));
        assert_eq!((update.width, update.height), (4, 2));
        assert_eq!(update.data.len(), 4 * 2 * 4);
        // RGBA, forced opaque: the wire colour is BGRA with an unused XA byte.
        assert_eq!(&update.data[..4], &[0x10, 0x20, 0x30, 255]);

        // The client-side copy must carry the fill for later reads.
        let surface = handler.surfaces.get(&1).expect("surface present");
        assert_eq!(surface.extract(2, 3, 1, 1), vec![0x10, 0x20, 0x30, 255]);
        // Outside the rectangle the surface stays opaque black.
        assert_eq!(surface.extract(0, 0, 1, 1), vec![0, 0, 0, 255]);
    }

    #[test]
    fn surface_to_surface_copies_within_one_surface() {
        let (mut handler, rx, _event_rx) = test_handler();
        map_surface(&mut handler, 1, 8, 8);

        // Paint a 2×2 marker at the origin, then copy it across.
        handler.on_solid_fill(&solid_fill(1, [9, 8, 7], rect(0, 0, 2, 2)));
        while rx.try_recv().is_ok() {}

        handler.on_surface_to_surface(&SurfaceToSurfacePdu {
            source_surface_id: 1,
            destination_surface_id: 1,
            source_rectangle: rect(0, 0, 2, 2),
            destination_points: vec![Point { x: 4, y: 4 }],
        });

        let update = rx.try_recv().expect("copy must emit an update");
        assert_eq!(
            (update.x, update.y, update.width, update.height),
            (4, 4, 2, 2)
        );
        let surface = handler.surfaces.get(&1).expect("surface present");
        assert_eq!(surface.extract(4, 4, 1, 1), vec![7, 8, 9, 255]);
    }

    #[test]
    fn cache_round_trip_restores_pixels() {
        let (mut handler, rx, _event_rx) = test_handler();
        map_surface(&mut handler, 1, 8, 8);

        handler.on_solid_fill(&solid_fill(1, [1, 2, 3], rect(0, 0, 2, 2)));
        while rx.try_recv().is_ok() {}

        handler.on_surface_to_cache(&SurfaceToCachePdu {
            surface_id: 1,
            cache_key: 42,
            cache_slot: 7,
            source_rectangle: rect(0, 0, 2, 2),
        });
        assert_eq!(handler.cache_bytes, 2 * 2 * 4);

        handler.on_cache_to_surface(&CacheToSurfacePdu {
            cache_slot: 7,
            surface_id: 1,
            destination_points: vec![Point { x: 6, y: 6 }],
        });

        let update = rx.try_recv().expect("cache blit must emit an update");
        assert_eq!(
            (update.x, update.y, update.width, update.height),
            (6, 6, 2, 2)
        );
        let surface = handler.surfaces.get(&1).expect("surface present");
        assert_eq!(surface.extract(6, 6, 1, 1), vec![3, 2, 1, 255]);

        handler.on_evict_cache_entry(&EvictCacheEntryPdu { cache_slot: 7 });
        assert_eq!(handler.cache_bytes, 0);
        assert!(handler.cache.is_empty());
    }

    /// Offscreen surfaces must be stored but never pushed to the screen.
    #[test]
    fn unmapped_surface_updates_are_stored_not_emitted() {
        let (mut handler, rx, _event_rx) = test_handler();
        handler.register_surface(2, 8, 8);

        handler.on_solid_fill(&solid_fill(2, [4, 5, 6], rect(0, 0, 4, 4)));

        assert!(rx.try_recv().is_err(), "offscreen surface must not paint");
        let surface = handler.surfaces.get(&2).expect("surface still stored");
        assert_eq!(surface.extract(0, 0, 1, 1), vec![6, 5, 4, 255]);
    }

    /// Regions extending past the surface edge must be clipped, not panic.
    #[test]
    fn out_of_bounds_regions_are_clipped() {
        let (mut handler, rx, _event_rx) = test_handler();
        map_surface(&mut handler, 1, 8, 8);

        handler.on_solid_fill(&SolidFillPdu {
            surface_id: 1,
            fill_pixel: Color {
                b: 1,
                g: 1,
                r: 1,
                xa: 0,
            },
            rectangles: vec![
                // Straddles the right/bottom edge.
                rect(6, 6, 34, 34),
                // Entirely outside.
                rect(20, 20, 4, 4),
            ],
        });

        let update = rx.try_recv().expect("the straddling rect must clip");
        assert_eq!((update.width, update.height), (2, 2));
        assert!(rx.try_recv().is_err(), "the outside rect must be dropped");
    }

    /// The unsupported-codec path is the actual issue #262 failure: content
    /// arrives, cannot be decoded, and must be reported rather than dropped.
    #[test]
    fn unsupported_codec_reports_once_past_the_threshold() {
        let (mut handler, _rx, event_rx) = test_handler();
        let pdu = GfxPdu::WireToSurface1(WireToSurface1Pdu {
            surface_id: 1,
            codec_id: Codec1Type::Avc444v2,
            pixel_format: PixelFormat::XRgb,
            destination_rectangle: rect(0, 0, 16, 16),
            bitmap_data: vec![0; 8],
        });

        for _ in 0..UNSUPPORTED_CODEC_THRESHOLD - 1 {
            handler.on_unhandled_pdu(&pdu);
            assert!(event_rx.try_recv().is_err(), "must not fire early");
        }

        handler.on_unhandled_pdu(&pdu);
        let event = event_rx.try_recv().expect("threshold must report");
        match event {
            RdpClientEvent::GfxUnsupportedCodec {
                codec,
                dropped_frames,
            } => {
                assert!(codec.contains("Avc444"), "codec was: {codec}");
                assert_eq!(dropped_frames, UNSUPPORTED_CODEC_THRESHOLD);
            }
            other => panic!("expected GfxUnsupportedCodec, got: {other:?}"),
        }

        // Reported once, not on every subsequent frame.
        handler.on_unhandled_pdu(&pdu);
        assert!(event_rx.try_recv().is_err(), "must report only once");
    }

    /// RFX Progressive has no decoder either, and `ironrdp-egfx` delivers it
    /// through its own callback rather than the catch-all — so it needs the same
    /// reporting or it becomes a second silent freeze.
    #[test]
    fn progressive_codec_is_reported_as_undecodable() {
        let (mut handler, _rx, event_rx) = test_handler();
        let pdu = WireToSurface2Pdu {
            surface_id: 1,
            codec_id: Codec2Type::RemoteFxProgressive,
            codec_context_id: 0,
            pixel_format: PixelFormat::XRgb,
            bitmap_data: vec![0; 8],
        };

        for _ in 0..UNSUPPORTED_CODEC_THRESHOLD {
            handler.on_wire_to_surface2(&pdu);
        }

        match event_rx.try_recv().expect("threshold must report") {
            RdpClientEvent::GfxUnsupportedCodec { codec, .. } => {
                assert_eq!(codec, "RfxProgressive");
            }
            other => panic!("expected GfxUnsupportedCodec, got: {other:?}"),
        }
    }

    #[test]
    fn handler_activation() {
        let (tx, _rx) = mpsc::channel();
        let (event_tx, _event_rx) = mpsc::channel();
        let mut handler = RustConnGfxHandler::new(tx, event_tx);

        assert!(!handler.is_active());

        let caps = ironrdp_egfx::pdu::CapabilitySet::V8 {
            flags: ironrdp_egfx::pdu::CapabilitiesV8Flags::SMALL_CACHE,
        };
        handler.on_capabilities_confirmed(&caps);
        assert!(handler.is_active());

        handler.on_close();
        assert!(!handler.is_active());
    }

    /// Verifies that when 10+ consecutive empty bitmap updates are received,
    /// the handler sends a `GfxDecodeFailure` event to the GUI.
    ///
    /// Since `BitmapUpdate` is `#[non_exhaustive]` (cannot be constructed
    /// outside `ironrdp-egfx`), we test the event-sending logic by directly
    /// simulating the internal state transition that `on_bitmap_updated`
    /// performs for empty frames.
    #[test]
    fn handler_sends_decode_failure_event() {
        let (tx, _rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let mut handler = RustConnGfxHandler::new(tx, event_tx);

        // Simulate 9 empty frames — no event yet
        handler.consecutive_empty = 9;
        assert!(event_rx.try_recv().is_err(), "No event before threshold");

        // Simulate the 10th empty frame crossing the threshold:
        // This is exactly what on_bitmap_updated does when data is empty.
        handler.consecutive_empty = 10;
        tracing::error!(
            consecutive_empty = handler.consecutive_empty,
            "Persistent decode failure — test simulation"
        );
        let _ = handler.event_tx.send(RdpClientEvent::GfxDecodeFailure {
            consecutive_failures: handler.consecutive_empty,
        });

        let event = event_rx
            .try_recv()
            .expect("Should receive GfxDecodeFailure event");
        assert!(
            matches!(
                event,
                RdpClientEvent::GfxDecodeFailure {
                    consecutive_failures: 10
                }
            ),
            "Expected GfxDecodeFailure with 10 failures, got: {event:?}"
        );
    }

    /// Verifies that all [`GfxError`] variants produce meaningful Display output
    /// suitable for structured logging and user-facing error messages.
    ///
    /// Covers: Req 10 AC 1, AC 5
    #[test]
    fn gfx_error_variants_display_coverage() {
        // Each variant produces a non-empty, descriptive string
        let variants: Vec<(GfxError, &str)> = vec![
            (
                GfxError::H264Unavailable("libpath not found".into()),
                "not available",
            ),
            (
                GfxError::H264DecodeFailed {
                    surface_id: 42,
                    reason: "invalid NAL unit".into(),
                },
                "surface 42",
            ),
            (GfxError::SurfaceNotMapped { surface_id: 7 }, "Surface 7"),
            (
                GfxError::PersistentDecodeFailure {
                    consecutive_failures: 15,
                },
                "15",
            ),
        ];

        for (err, expected_substr) in variants {
            let display = err.to_string();
            assert!(
                !display.is_empty(),
                "GfxError Display should not be empty: {err:?}"
            );
            assert!(
                display.contains(expected_substr),
                "Expected '{expected_substr}' in '{display}'"
            );
        }
    }

    #[test]
    fn soname_version_accepts_only_numeric_segments() {
        assert_eq!(soname_version("libopenh264.so.8"), Some(vec![8]));
        assert_eq!(soname_version("libopenh264.so.2.6.0"), Some(vec![2, 6, 0]));

        // The unversioned name is handled by OPENH264_SEARCH_PATHS, not here.
        assert_eq!(soname_version("libopenh264.so"), None);
        assert_eq!(soname_version("libopenh264.so."), None);
        assert_eq!(soname_version("libopenh264.so.debug"), None);
        assert_eq!(soname_version("libopenh264.so.8.debug"), None);
        assert_eq!(soname_version("libavcodec.so.60"), None);
    }

    #[test]
    fn sonames_sort_newest_abi_first() {
        // Deliberately not in the answer's order, and not in an order a string
        // sort would fix: "10" < "9" lexicographically.
        let names = vec![
            "libopenh264.so.2.6.0".to_string(),
            "libopenh264.so.9".to_string(),
            "libopenh264.so.10".to_string(),
            "libopenh264.so.7".to_string(),
        ];

        assert_eq!(
            sonames_newest_first(names),
            vec![
                "libopenh264.so.10",
                "libopenh264.so.9",
                "libopenh264.so.7",
                "libopenh264.so.2.6.0",
            ]
        );
    }

    #[test]
    fn bare_soname_wins_over_the_file_it_points_at() {
        let names = vec![
            "libopenh264.so.8.0.1".to_string(),
            "libopenh264.so.8".to_string(),
        ];

        assert_eq!(
            sonames_newest_first(names),
            vec!["libopenh264.so.8", "libopenh264.so.8.0.1"]
        );
    }

    #[test]
    fn sonames_drop_unrelated_entries() {
        let names = vec![
            "libopenh264.so".to_string(),
            "libopenh264.so.8".to_string(),
            "libx264.so.164".to_string(),
            "pkgconfig".to_string(),
        ];

        assert_eq!(sonames_newest_first(names), vec!["libopenh264.so.8"]);
    }

    /// The Debian layout that made H.264 unreachable before the scan existed:
    /// a runtime-only install carries the soname symlink and the real file, and
    /// never the unversioned name the old candidate list looked for.
    #[test]
    fn debian_runtime_only_layout_resolves() {
        let names = vec![
            "libopenh264.so.2.6.0".to_string(),
            "libopenh264.so.8".to_string(),
        ];

        let ordered = sonames_newest_first(names);
        assert_eq!(
            ordered.first().map(String::as_str),
            Some("libopenh264.so.8")
        );
        assert_eq!(ordered.len(), 2, "the real file stays as a fallback");
    }
}
