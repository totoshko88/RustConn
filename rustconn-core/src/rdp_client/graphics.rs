//! Graphics mode detection and configuration for RDP
//!
//! This module provides utilities for detecting and configuring graphics
//! capabilities for RDP sessions. It handles codec negotiation and
//! provides fallback strategies when advanced codecs aren't available.
//!
//! # Graphics Pipeline Overview
//!
//! RDP supports multiple graphics pipelines:
//! - Legacy: Basic bitmap updates (always available)
//! - RemoteFX: High-quality codec for LAN connections
//! - GFX (RDPGFX): Modern pipeline with H.264/AVC support
//!
//! # IronRDP Support Status
//!
//! - Legacy: Fully supported
//! - RemoteFX: Supported
//! - GFX/H.264: Not yet supported (requires upstream changes)

#![allow(clippy::struct_excessive_bools)]

use serde::{Deserialize, Serialize};

/// Graphics mode for RDP sessions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GraphicsMode {
    /// Automatic mode selection based on server capabilities
    #[default]
    Auto,
    /// Legacy bitmap updates (most compatible)
    Legacy,
    /// RemoteFX codec (good quality, moderate bandwidth)
    RemoteFx,
    /// GFX pipeline without H.264 (better than RemoteFX)
    Gfx,
    /// GFX pipeline with H.264/AVC (best quality, requires decoder)
    GfxH264,
    /// GFX pipeline with H.264 in AVC444 mode (highest quality)
    GfxAvc444,
}

impl GraphicsMode {
    /// Returns a human-readable name for the graphics mode
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::Legacy => "Legacy (Compatible)",
            Self::RemoteFx => "RemoteFX",
            Self::Gfx => "GFX Pipeline",
            Self::GfxH264 => "GFX + H.264",
            Self::GfxAvc444 => "GFX + AVC444",
        }
    }

    /// Returns whether this mode requires H.264 decoding
    #[must_use]
    pub const fn requires_h264(&self) -> bool {
        matches!(self, Self::GfxH264 | Self::GfxAvc444)
    }

    /// Returns whether this mode uses the GFX pipeline
    #[must_use]
    pub const fn uses_gfx(&self) -> bool {
        matches!(self, Self::Gfx | Self::GfxH264 | Self::GfxAvc444)
    }

    /// Returns the recommended color depth for this mode
    #[must_use]
    pub const fn recommended_color_depth(&self) -> u8 {
        match self {
            Self::Legacy => 16,
            Self::RemoteFx | Self::Gfx => 24,
            Self::GfxH264 | Self::GfxAvc444 | Self::Auto => 32,
        }
    }

    /// Returns whether this mode is currently supported by IronRDP
    #[must_use]
    pub const fn is_supported(&self) -> bool {
        matches!(self, Self::Auto | Self::Legacy | Self::RemoteFx)
    }
}

/// Server graphics capabilities detected during connection
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct ServerGraphicsCapabilities {
    /// Server supports RemoteFX codec
    pub supports_remotefx: bool,
    /// Server supports GFX pipeline
    pub supports_gfx: bool,
    /// Server supports H.264/AVC in GFX
    pub supports_h264: bool,
    /// Server supports AVC444 mode
    pub supports_avc444: bool,
    /// Server supports dynamic resolution changes
    pub supports_dynamic_resolution: bool,
    /// Maximum supported color depth
    pub max_color_depth: u8,
    /// Server-preferred graphics mode
    pub preferred_mode: Option<GraphicsMode>,
}

impl ServerGraphicsCapabilities {
    /// Creates capabilities for a basic server (legacy only)
    #[must_use]
    pub const fn legacy_only() -> Self {
        Self {
            supports_remotefx: false,
            supports_gfx: false,
            supports_h264: false,
            supports_avc444: false,
            supports_dynamic_resolution: false,
            max_color_depth: 24,
            preferred_mode: Some(GraphicsMode::Legacy),
        }
    }

    /// Creates capabilities for a modern Windows server
    #[must_use]
    pub const fn modern_windows() -> Self {
        Self {
            supports_remotefx: true,
            supports_gfx: true,
            supports_h264: true,
            supports_avc444: true,
            supports_dynamic_resolution: true,
            max_color_depth: 32,
            preferred_mode: Some(GraphicsMode::GfxH264),
        }
    }

    /// Selects the best available graphics mode
    #[must_use]
    pub fn select_best_mode(&self, requested: GraphicsMode) -> GraphicsMode {
        match requested {
            GraphicsMode::Auto => self.auto_select(),
            mode if self.supports_mode(mode) => mode,
            _ => self.auto_select(),
        }
    }

    /// Automatically selects the best supported mode
    fn auto_select(&self) -> GraphicsMode {
        // Prefer modes in order of quality (but only if supported by IronRDP)
        // Currently, H.264 modes aren't supported by IronRDP
        if self.supports_remotefx {
            GraphicsMode::RemoteFx
        } else {
            GraphicsMode::Legacy
        }
    }

    /// Checks if a specific mode is supported
    #[must_use]
    pub const fn supports_mode(&self, mode: GraphicsMode) -> bool {
        match mode {
            GraphicsMode::Auto | GraphicsMode::Legacy => true,
            GraphicsMode::RemoteFx => self.supports_remotefx,
            GraphicsMode::Gfx => self.supports_gfx,
            GraphicsMode::GfxH264 => self.supports_gfx && self.supports_h264,
            GraphicsMode::GfxAvc444 => self.supports_gfx && self.supports_avc444,
        }
    }
}

/// Graphics quality settings
///
/// Controls visual quality features for RDP sessions. Higher quality settings
/// provide better visuals but require more bandwidth and processing power.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphicsQuality {
    /// Color depth (8, 15, 16, 24, or 32)
    pub color_depth: u8,
    /// Visual experience flags controlling various quality features
    pub features: GraphicsFeatures,
}

/// Visual experience feature flags for RDP sessions
///
/// These flags control various visual quality features. Each flag can be
/// independently enabled or disabled to balance quality vs. performance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GraphicsFeatures {
    /// Enable font smoothing (ClearType)
    pub font_smoothing: bool,
    /// Enable desktop composition (Aero glass effects)
    pub desktop_composition: bool,
    /// Enable full window drag (show window contents while dragging)
    pub full_window_drag: bool,
    /// Enable menu animations
    pub menu_animations: bool,
    /// Enable visual themes
    pub themes: bool,
    /// Show desktop wallpaper
    pub wallpaper: bool,
}

impl GraphicsFeatures {
    /// Creates features with all options enabled (highest quality)
    #[must_use]
    pub const fn all_enabled() -> Self {
        Self {
            font_smoothing: true,
            desktop_composition: true,
            full_window_drag: true,
            menu_animations: true,
            themes: true,
            wallpaper: true,
        }
    }

    /// Creates features with all options disabled (best performance)
    #[must_use]
    pub const fn all_disabled() -> Self {
        Self {
            font_smoothing: false,
            desktop_composition: false,
            full_window_drag: false,
            menu_animations: false,
            themes: false,
            wallpaper: false,
        }
    }

    /// Creates balanced features (good quality with reasonable performance)
    #[must_use]
    pub const fn balanced() -> Self {
        Self {
            font_smoothing: true,
            desktop_composition: false,
            full_window_drag: false,
            menu_animations: false,
            themes: true,
            wallpaper: false,
        }
    }
}

impl Default for GraphicsQuality {
    fn default() -> Self {
        Self::balanced()
    }
}

impl GraphicsQuality {
    /// Creates quality settings optimized for best visual quality
    #[must_use]
    pub const fn high_quality() -> Self {
        Self {
            color_depth: 32,
            features: GraphicsFeatures::all_enabled(),
        }
    }

    /// Creates balanced quality settings
    #[must_use]
    pub const fn balanced() -> Self {
        Self {
            color_depth: 24,
            features: GraphicsFeatures::balanced(),
        }
    }

    /// Creates quality settings optimized for performance
    #[must_use]
    pub const fn performance() -> Self {
        Self {
            color_depth: 16,
            features: GraphicsFeatures::all_disabled(),
        }
    }

    /// Creates quality settings for low bandwidth connections
    #[must_use]
    pub const fn low_bandwidth() -> Self {
        Self {
            color_depth: 15,
            features: GraphicsFeatures::all_disabled(),
        }
    }

    /// Validates the quality settings
    ///
    /// # Errors
    ///
    /// Returns an error if the color depth is not a valid RDP color depth
    /// (must be 8, 15, 16, 24, or 32).
    pub fn validate(&self) -> Result<(), GraphicsError> {
        if !matches!(self.color_depth, 8 | 15 | 16 | 24 | 32) {
            return Err(GraphicsError::InvalidColorDepth(self.color_depth));
        }
        Ok(())
    }
}

/// Graphics-related errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum GraphicsError {
    /// Invalid color depth
    #[error("Invalid color depth: {0}. Must be 8, 15, 16, 24, or 32")]
    InvalidColorDepth(u8),

    /// Codec not supported
    #[error("Graphics codec not supported: {0}")]
    CodecNotSupported(String),

    /// Decoder error
    #[error("Graphics decoder error: {0}")]
    DecoderError(String),

    /// Frame buffer error
    #[error("Frame buffer error: {0}")]
    FrameBufferError(String),
}

/// Frame statistics for performance monitoring
#[derive(Debug, Clone, Default)]
pub struct FrameStatistics {
    /// Total frames received
    pub frames_received: u64,
    /// Total frames decoded
    pub frames_decoded: u64,
    /// Total frames dropped
    pub frames_dropped: u64,
    /// Total bytes received
    pub bytes_received: u64,
    /// Average decode time in microseconds
    pub avg_decode_time_us: u64,
    /// Current frames per second
    pub current_fps: f32,
    /// Peak frames per second
    pub peak_fps: f32,
    /// Last update timestamp
    last_update: Option<std::time::Instant>,
    /// Frames since last FPS calculation
    frames_since_update: u32,
}

impl FrameStatistics {
    /// Creates new frame statistics
    #[must_use]
    pub const fn new() -> Self {
        Self {
            frames_received: 0,
            frames_decoded: 0,
            frames_dropped: 0,
            bytes_received: 0,
            avg_decode_time_us: 0,
            current_fps: 0.0,
            peak_fps: 0.0,
            last_update: None,
            frames_since_update: 0,
        }
    }

    /// Records a received frame
    pub fn record_frame(&mut self, bytes: usize, decode_time_us: u64) {
        self.frames_received += 1;
        self.frames_decoded += 1;
        self.bytes_received += bytes as u64;
        self.frames_since_update += 1;

        // Update average decode time (exponential moving average)
        if self.avg_decode_time_us == 0 {
            self.avg_decode_time_us = decode_time_us;
        } else {
            self.avg_decode_time_us = (self.avg_decode_time_us * 7 + decode_time_us) / 8;
        }

        // Update FPS every second
        self.update_fps();
    }

    /// Records a dropped frame
    pub fn record_dropped(&mut self) {
        self.frames_received += 1;
        self.frames_dropped += 1;
    }

    /// Updates FPS calculation
    fn update_fps(&mut self) {
        let now = std::time::Instant::now();

        match self.last_update {
            Some(last) => {
                let elapsed = now.duration_since(last);
                if elapsed.as_secs_f32() >= 1.0 {
                    self.current_fps = self.frames_since_update as f32 / elapsed.as_secs_f32();
                    if self.current_fps > self.peak_fps {
                        self.peak_fps = self.current_fps;
                    }
                    self.frames_since_update = 0;
                    self.last_update = Some(now);
                }
            }
            None => {
                self.last_update = Some(now);
            }
        }
    }

    /// Returns the frame drop rate as a percentage
    #[must_use]
    pub fn drop_rate(&self) -> f32 {
        if self.frames_received == 0 {
            0.0
        } else {
            (self.frames_dropped as f32 / self.frames_received as f32) * 100.0
        }
    }

    /// Returns the average bandwidth in Kbps
    #[must_use]
    pub fn bandwidth_kbps(&self) -> f32 {
        if self.current_fps == 0.0 {
            0.0
        } else {
            // bytes_per_frame * fps * 8 / 1000
            let bytes_per_frame = self.bytes_received as f32 / self.frames_decoded.max(1) as f32;
            bytes_per_frame * self.current_fps * 8.0 / 1000.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graphics_mode_display() {
        assert_eq!(GraphicsMode::Auto.display_name(), "Automatic");
        assert_eq!(GraphicsMode::RemoteFx.display_name(), "RemoteFX");
    }

    #[test]
    fn test_graphics_mode_h264() {
        assert!(!GraphicsMode::RemoteFx.requires_h264());
        assert!(GraphicsMode::GfxH264.requires_h264());
        assert!(GraphicsMode::GfxAvc444.requires_h264());
    }

    #[test]
    fn test_graphics_mode_supported() {
        assert!(GraphicsMode::Legacy.is_supported());
        assert!(GraphicsMode::RemoteFx.is_supported());
        assert!(!GraphicsMode::GfxH264.is_supported());
    }

    #[test]
    fn test_server_capabilities_legacy() {
        let caps = ServerGraphicsCapabilities::legacy_only();
        assert!(!caps.supports_remotefx);
        assert!(!caps.supports_gfx);
        assert_eq!(
            caps.select_best_mode(GraphicsMode::Auto),
            GraphicsMode::Legacy
        );
    }

    #[test]
    fn test_server_capabilities_modern() {
        let caps = ServerGraphicsCapabilities::modern_windows();
        assert!(caps.supports_remotefx);
        assert!(caps.supports_gfx);
        assert!(caps.supports_h264);
        // Auto should select RemoteFX since H.264 isn't supported by IronRDP yet
        assert_eq!(
            caps.select_best_mode(GraphicsMode::Auto),
            GraphicsMode::RemoteFx
        );
    }

    #[test]
    fn test_graphics_quality_presets() {
        let high = GraphicsQuality::high_quality();
        assert_eq!(high.color_depth, 32);
        assert!(high.features.font_smoothing);
        assert!(high.features.desktop_composition);

        let perf = GraphicsQuality::performance();
        assert_eq!(perf.color_depth, 16);
        assert!(!perf.features.font_smoothing);
    }

    #[test]
    fn test_graphics_quality_validate() {
        let valid = GraphicsQuality::balanced();
        assert!(valid.validate().is_ok());

        let invalid = GraphicsQuality {
            color_depth: 12,
            features: GraphicsFeatures::default(),
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn test_graphics_features() {
        let all = GraphicsFeatures::all_enabled();
        assert!(all.font_smoothing);
        assert!(all.desktop_composition);
        assert!(all.themes);

        let none = GraphicsFeatures::all_disabled();
        assert!(!none.font_smoothing);
        assert!(!none.desktop_composition);
        assert!(!none.themes);

        let balanced = GraphicsFeatures::balanced();
        assert!(balanced.font_smoothing);
        assert!(!balanced.desktop_composition);
        assert!(balanced.themes);
    }

    #[test]
    fn test_frame_statistics() {
        let mut stats = FrameStatistics::new();

        stats.record_frame(1000, 500);
        assert_eq!(stats.frames_received, 1);
        assert_eq!(stats.frames_decoded, 1);
        assert_eq!(stats.bytes_received, 1000);

        stats.record_dropped();
        assert_eq!(stats.frames_received, 2);
        assert_eq!(stats.frames_dropped, 1);
        assert!((stats.drop_rate() - 50.0).abs() < f32::EPSILON);
    }
}
