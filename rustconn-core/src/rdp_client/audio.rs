//! RDP Audio backend implementation (RDPSND)
//!
//! This module implements the `RdpsndClientHandler` trait from `IronRDP`
//! to handle audio playback from the RDP server.
//!
//! # Architecture
//!
//! Audio data flows from server to client:
//! 1. Server sends audio format negotiation
//! 2. Client responds with supported formats
//! 3. Server sends audio data via `wave()` callback
//! 4. Backend sends data to GUI via event channel
//! 5. GUI plays audio using platform audio API (cpal/rodio)

use super::RdpClientEvent;
use ironrdp::core::impl_as_any;
use ironrdp::rdpsnd::client::RdpsndClientHandler;
use ironrdp::rdpsnd::pdu::{AudioFormat, AudioFormatFlags, PitchPdu, VolumePdu, WaveFormat};
use std::borrow::Cow;
use std::sync::mpsc::Sender;
use tracing::{debug, trace};

/// Audio format information for GUI playback
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFormatInfo {
    /// Format tag (1 = PCM, 6 = A-Law, 7 = μ-Law, etc.)
    pub format_tag: u16,
    /// Number of channels (1 = mono, 2 = stereo)
    pub channels: u16,
    /// Samples per second (e.g., 44100, 48000)
    pub samples_per_sec: u32,
    /// Average bytes per second
    pub avg_bytes_per_sec: u32,
    /// Block alignment
    pub block_align: u16,
    /// Bits per sample (8, 16, 24, 32)
    pub bits_per_sample: u16,
}

impl AudioFormatInfo {
    /// PCM format tag
    pub const FORMAT_PCM: u16 = 1;
    /// A-Law format tag
    pub const FORMAT_ALAW: u16 = 6;
    /// μ-Law format tag
    pub const FORMAT_MULAW: u16 = 7;

    /// Creates a new audio format info
    #[must_use]
    pub fn new(format_tag: u16, channels: u16, samples_per_sec: u32, bits_per_sample: u16) -> Self {
        let block_align = channels * (bits_per_sample / 8);
        let avg_bytes_per_sec = samples_per_sec * u32::from(block_align);
        Self {
            format_tag,
            channels,
            samples_per_sec,
            avg_bytes_per_sec,
            block_align,
            bits_per_sample,
        }
    }

    /// Creates a standard CD-quality PCM format (44100 Hz, 16-bit, stereo)
    #[must_use]
    pub fn cd_quality() -> Self {
        Self::new(Self::FORMAT_PCM, 2, 44100, 16)
    }

    /// Creates a DVD-quality PCM format (48000 Hz, 16-bit, stereo)
    #[must_use]
    pub fn dvd_quality() -> Self {
        Self::new(Self::FORMAT_PCM, 2, 48000, 16)
    }

    /// Returns true if this is a PCM format
    #[must_use]
    pub const fn is_pcm(&self) -> bool {
        self.format_tag == Self::FORMAT_PCM
    }

    /// Returns the number of bytes per sample (all channels)
    #[must_use]
    pub const fn bytes_per_sample(&self) -> u16 {
        self.block_align
    }
}

impl From<&AudioFormat> for AudioFormatInfo {
    fn from(format: &AudioFormat) -> Self {
        // Convert WaveFormat enum to u16 using its raw value
        let format_tag = match format.format {
            WaveFormat::PCM => 1,
            WaveFormat::ADPCM => 2,
            WaveFormat::ALAW => 6,
            WaveFormat::MULAW => 7,
            WaveFormat::DVI_ADPCM => 17,
            WaveFormat::GSM610 => 49,
            WaveFormat::MSG723 => 66,
            WaveFormat::MPEGLAYER3 => 85,
            WaveFormat::AAC_MS => 352,
            _ => 0, // Unknown format
        };
        Self {
            format_tag,
            channels: format.n_channels,
            samples_per_sec: format.n_samples_per_sec,
            avg_bytes_per_sec: format.n_avg_bytes_per_sec,
            block_align: format.n_block_align,
            bits_per_sample: format.bits_per_sample,
        }
    }
}

/// `RustConn` audio backend for `IronRDP`
///
/// This backend receives audio data from the RDP server and forwards it
/// to the GUI for playback via the event channel.
#[derive(Debug)]
pub struct RustConnAudioBackend {
    /// Supported audio formats
    supported_formats: Vec<AudioFormat>,
    /// Event sender to GUI
    event_tx: Sender<RdpClientEvent>,
    /// Currently selected format index
    current_format: Option<usize>,
    /// Whether audio is enabled
    enabled: bool,
}

impl_as_any!(RustConnAudioBackend);

impl RustConnAudioBackend {
    /// Creates a new audio backend
    #[must_use]
    pub fn new(event_tx: Sender<RdpClientEvent>) -> Self {
        Self {
            supported_formats: Self::default_formats(),
            event_tx,
            current_format: None,
            enabled: true,
        }
    }

    /// Creates a disabled (no-op) audio backend
    #[must_use]
    pub fn disabled(event_tx: Sender<RdpClientEvent>) -> Self {
        Self {
            supported_formats: Vec::new(),
            event_tx,
            current_format: None,
            enabled: false,
        }
    }

    /// Returns the default supported audio formats
    ///
    /// We support common PCM formats that are easy to play back:
    /// - 48000 Hz stereo 16-bit (DVD quality)
    /// - 44100 Hz stereo 16-bit (CD quality)
    /// - 22050 Hz stereo 16-bit (low quality fallback)
    /// - 48000 Hz mono 16-bit
    /// - 44100 Hz mono 16-bit
    fn default_formats() -> Vec<AudioFormat> {
        vec![
            // 48000 Hz stereo 16-bit (preferred)
            AudioFormat {
                format: WaveFormat::PCM,
                n_channels: 2,
                n_samples_per_sec: 48_000,
                n_avg_bytes_per_sec: 192_000,
                n_block_align: 4,
                bits_per_sample: 16,
                data: None,
            },
            // 44100 Hz stereo 16-bit (CD quality)
            AudioFormat {
                format: WaveFormat::PCM,
                n_channels: 2,
                n_samples_per_sec: 44_100,
                n_avg_bytes_per_sec: 176_400,
                n_block_align: 4,
                bits_per_sample: 16,
                data: None,
            },
            // 22050 Hz stereo 16-bit (low quality)
            AudioFormat {
                format: WaveFormat::PCM,
                n_channels: 2,
                n_samples_per_sec: 22050,
                n_avg_bytes_per_sec: 88200,
                n_block_align: 4,
                bits_per_sample: 16,
                data: None,
            },
            // 48000 Hz mono 16-bit
            AudioFormat {
                format: WaveFormat::PCM,
                n_channels: 1,
                n_samples_per_sec: 48000,
                n_avg_bytes_per_sec: 96000,
                n_block_align: 2,
                bits_per_sample: 16,
                data: None,
            },
            // 44100 Hz mono 16-bit
            AudioFormat {
                format: WaveFormat::PCM,
                n_channels: 1,
                n_samples_per_sec: 44100,
                n_avg_bytes_per_sec: 88200,
                n_block_align: 2,
                bits_per_sample: 16,
                data: None,
            },
        ]
    }

    /// Returns the current audio format info if available
    #[must_use]
    pub fn current_format_info(&self) -> Option<AudioFormatInfo> {
        self.current_format
            .and_then(|idx| self.supported_formats.get(idx))
            .map(AudioFormatInfo::from)
    }

    /// Returns whether audio is enabled
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }
}

impl RdpsndClientHandler for RustConnAudioBackend {
    fn get_formats(&self) -> &[AudioFormat] {
        if self.enabled {
            &self.supported_formats
        } else {
            &[]
        }
    }

    fn get_flags(&self) -> AudioFormatFlags {
        // Request volume control capability
        AudioFormatFlags::VOLUME
    }

    fn wave(&mut self, format_no: usize, ts: u32, data: Cow<'_, [u8]>) {
        if !self.enabled {
            return;
        }

        // Update current format if changed
        if self.current_format != Some(format_no) {
            self.current_format = Some(format_no);
            if let Some(format) = self.supported_formats.get(format_no) {
                debug!(
                    "Audio format selected: {} Hz, {} ch, {} bit",
                    format.n_samples_per_sec, format.n_channels, format.bits_per_sample
                );
                let format_info = AudioFormatInfo::from(format);
                let _ = self
                    .event_tx
                    .send(RdpClientEvent::AudioFormatChanged(format_info));
            }
        }

        trace!(
            "Audio wave: format={}, ts={}, {} bytes",
            format_no,
            ts,
            data.len()
        );

        // Send audio data to GUI for playback
        let _ = self.event_tx.send(RdpClientEvent::AudioData {
            format_index: format_no,
            timestamp: ts,
            data: data.into_owned(),
        });
    }

    fn set_volume(&mut self, volume: VolumePdu) {
        debug!(
            "Audio volume changed: left={}, right={}",
            volume.volume_left, volume.volume_right
        );
        let _ = self.event_tx.send(RdpClientEvent::AudioVolume {
            left: volume.volume_left,
            right: volume.volume_right,
        });
    }

    fn set_pitch(&mut self, pitch: PitchPdu) {
        trace!("Audio pitch changed: {}", pitch.pitch);
        // Pitch changes are rarely used, just log them
    }

    fn close(&mut self) {
        debug!("Audio channel closed");
        self.current_format = None;
        let _ = self.event_tx.send(RdpClientEvent::AudioClose);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn test_audio_format_info_cd_quality() {
        let format = AudioFormatInfo::cd_quality();
        assert_eq!(format.format_tag, 1);
        assert_eq!(format.channels, 2);
        assert_eq!(format.samples_per_sec, 44100);
        assert_eq!(format.bits_per_sample, 16);
        assert_eq!(format.block_align, 4);
        assert!(format.is_pcm());
    }

    #[test]
    fn test_audio_format_info_dvd_quality() {
        let format = AudioFormatInfo::dvd_quality();
        assert_eq!(format.samples_per_sec, 48000);
    }

    #[test]
    fn test_audio_backend_default_formats() {
        let (tx, _rx) = mpsc::channel();
        let backend = RustConnAudioBackend::new(tx);
        let formats = backend.get_formats();
        assert!(!formats.is_empty());
        // First format should be 48000 Hz stereo
        assert_eq!(formats[0].n_samples_per_sec, 48_000);
        assert_eq!(formats[0].n_channels, 2);
    }

    #[test]
    fn test_audio_backend_disabled() {
        let (tx, _rx) = mpsc::channel();
        let backend = RustConnAudioBackend::disabled(tx);
        assert!(!backend.is_enabled());
        assert!(backend.get_formats().is_empty());
    }

    #[test]
    fn test_audio_backend_wave_sends_event() {
        let (tx, rx) = mpsc::channel();
        let mut backend = RustConnAudioBackend::new(tx);

        let data = vec![0u8; 1024];
        backend.wave(0, 12345, Cow::Borrowed(&data));

        // Should receive format change event first
        let event = rx.try_recv().unwrap();
        assert!(matches!(event, RdpClientEvent::AudioFormatChanged(_)));

        // Then audio data event
        let event = rx.try_recv().unwrap();
        if let RdpClientEvent::AudioData {
            format_index,
            timestamp,
            data: received_data,
        } = event
        {
            assert_eq!(format_index, 0);
            assert_eq!(timestamp, 12345);
            assert_eq!(received_data.len(), 1024);
        } else {
            panic!("Expected AudioData event");
        }
    }

    #[test]
    fn test_audio_backend_volume() {
        let (tx, rx) = mpsc::channel();
        let mut backend = RustConnAudioBackend::new(tx);

        backend.set_volume(VolumePdu {
            volume_left: 32768,
            volume_right: 65535,
        });

        let event = rx.try_recv().unwrap();
        if let RdpClientEvent::AudioVolume { left, right } = event {
            assert_eq!(left, 32768);
            assert_eq!(right, 65535);
        } else {
            panic!("Expected AudioVolume event");
        }
    }
}
