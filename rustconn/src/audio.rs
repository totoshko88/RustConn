//! Audio playback module for RDP embedded mode
//!
//! This module provides audio playback functionality using `cpal` for
//! RDP sessions with audio redirection enabled.
//!
//! # Architecture
//!
//! Audio data flows from the RDP server through `IronRDP` to this module:
//! 1. `RdpClientEvent::AudioFormatChanged` - configures the audio stream
//! 2. `RdpClientEvent::AudioData` - queues PCM data for playback
//! 3. `RdpClientEvent::AudioVolume` - adjusts playback volume
//! 4. `RdpClientEvent::AudioClose` - stops playback
//!
//! # Safety Notes
//!
//! Mutex locks in audio callbacks are safe - they protect a simple buffer and
//! are held only briefly. Poisoning would indicate a panic in the audio thread
//! which is unrecoverable anyway. The `unwrap()` calls in audio callbacks are
//! intentional and documented with function-level `#[allow]` attributes.

// Audio processing requires numeric casts that are intentional and safe for audio data
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::unused_self)]

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Stream, StreamConfig};
use rustconn_core::AudioFormatInfo;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use thiserror::Error;

/// Error type for audio operations
#[derive(Debug, Error)]
pub enum AudioError {
    /// No audio output device available
    #[error("No audio output device available")]
    NoDevice,

    /// Unsupported audio format
    #[error("Unsupported audio format: {0}")]
    UnsupportedFormat(String),

    /// Stream creation failed
    #[error("Failed to create audio stream: {0}")]
    StreamCreation(String),

    /// Playback error
    #[error("Audio playback error: {0}")]
    Playback(String),
}

/// Audio buffer for queuing PCM data
#[derive(Debug, Default)]
struct AudioBuffer {
    /// Queued audio samples (interleaved i16)
    samples: VecDeque<i16>,
    /// Maximum buffer size in samples
    max_size: usize,
}

impl AudioBuffer {
    /// Creates a new audio buffer with default capacity
    fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(48_000 * 2 * 2), // ~2 seconds at 48kHz stereo
            max_size: 48_000 * 2 * 4,                         // ~4 seconds max
        }
    }

    /// Pushes PCM data (16-bit signed, little-endian) to the buffer
    fn push_pcm_data(&mut self, data: &[u8]) {
        // Convert bytes to i16 samples
        for chunk in data.chunks_exact(2) {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            self.samples.push_back(sample);
        }

        // Trim if buffer is too large (drop oldest samples)
        while self.samples.len() > self.max_size {
            self.samples.pop_front();
        }
    }

    /// Pops samples from the buffer for playback
    fn pop_samples(&mut self, count: usize) -> Vec<i16> {
        let actual_count = count.min(self.samples.len());
        self.samples.drain(..actual_count).collect()
    }

    /// Returns the number of buffered samples
    fn len(&self) -> usize {
        self.samples.len()
    }

    /// Clears the buffer
    fn clear(&mut self) {
        self.samples.clear();
    }
}

/// Audio player for RDP sessions
pub struct RdpAudioPlayer {
    /// Shared audio buffer
    buffer: Arc<Mutex<AudioBuffer>>,
    /// Current audio stream (if playing)
    stream: Option<Stream>,
    /// Current audio format
    format: Option<AudioFormatInfo>,
    /// Volume as fixed-point (0-65535 maps to 0.0-1.0) - lock-free for audio callback
    volume: Arc<AtomicU32>,
}

impl RdpAudioPlayer {
    /// Creates a new audio player
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(AudioBuffer::new())),
            stream: None,
            format: None,
            volume: Arc::new(AtomicU32::new(65535)), // Default to full volume
        }
    }

    /// Configures the audio stream for the given format
    ///
    /// # Errors
    ///
    /// Returns error if audio device is not available or format is unsupported.
    pub fn configure(&mut self, format: AudioFormatInfo) -> Result<(), AudioError> {
        // Stop existing stream
        self.stop();

        // Only support PCM format
        if !format.is_pcm() {
            return Err(AudioError::UnsupportedFormat(format!(
                "Only PCM format supported, got tag {}",
                format.format_tag
            )));
        }

        tracing::debug!(
            "[Audio] Configuring: {} Hz, {} ch, {} bit",
            format.samples_per_sec,
            format.channels,
            format.bits_per_sample
        );

        // Get default audio output device
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(AudioError::NoDevice)?;

        // Configure stream
        let config = StreamConfig {
            channels: format.channels,
            sample_rate: format.samples_per_sec,
            buffer_size: cpal::BufferSize::Default,
        };

        // Create stream based on sample format
        let buffer = Arc::clone(&self.buffer);
        let volume = Arc::clone(&self.volume);

        let stream = match format.bits_per_sample {
            16 => self.create_i16_stream(&device, &config, buffer, volume)?,
            8 => self.create_u8_stream(&device, &config, buffer, volume)?,
            _ => {
                return Err(AudioError::UnsupportedFormat(format!(
                    "Unsupported bits per sample: {}",
                    format.bits_per_sample
                )));
            }
        };

        // Start playback
        stream
            .play()
            .map_err(|e| AudioError::Playback(e.to_string()))?;

        self.stream = Some(stream);
        self.format = Some(format);

        tracing::debug!("[Audio] Stream started");
        Ok(())
    }

    /// Creates an i16 audio stream
    ///
    /// # Panics
    ///
    /// The audio callback panics if the buffer mutex is poisoned, which indicates
    /// an unrecoverable panic occurred in another thread while holding the lock.
    #[allow(clippy::unwrap_used)]
    fn create_i16_stream(
        &self,
        device: &cpal::Device,
        config: &StreamConfig,
        buffer: Arc<Mutex<AudioBuffer>>,
        volume: Arc<AtomicU32>,
    ) -> Result<Stream, AudioError> {
        let stream = device
            .build_output_stream(
                config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    // Lock-free volume read (0-65535 range)
                    let vol_raw = volume.load(Ordering::Relaxed);
                    let vol = vol_raw as f32 / 65535.0;

                    // Graceful degradation on poisoned mutex — fill silence
                    let Ok(mut buf) = buffer.lock() else {
                        for sample in data.iter_mut() {
                            *sample = 0;
                        }
                        return;
                    };
                    let samples = buf.pop_samples(data.len());

                    for (i, sample) in data.iter_mut().enumerate() {
                        if i < samples.len() {
                            // Apply volume
                            let scaled = (f32::from(samples[i]) * vol) as i16;
                            *sample = scaled;
                        } else {
                            // Silence if buffer underrun
                            *sample = 0;
                        }
                    }
                },
                |err| {
                    tracing::error!("[Audio] Stream error: {}", err);
                },
                None,
            )
            .map_err(|e| AudioError::StreamCreation(e.to_string()))?;

        Ok(stream)
    }

    /// Creates a u8 audio stream (8-bit unsigned)
    ///
    /// # Panics
    ///
    /// The audio callback panics if the buffer mutex is poisoned, which indicates
    /// an unrecoverable panic occurred in another thread while holding the lock.
    #[allow(clippy::unwrap_used)]
    fn create_u8_stream(
        &self,
        device: &cpal::Device,
        config: &StreamConfig,
        buffer: Arc<Mutex<AudioBuffer>>,
        volume: Arc<AtomicU32>,
    ) -> Result<Stream, AudioError> {
        let stream = device
            .build_output_stream(
                config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    // Lock-free volume read
                    let vol_raw = volume.load(Ordering::Relaxed);
                    let vol = vol_raw as f32 / 65535.0;

                    // Graceful degradation on poisoned mutex — fill silence
                    let Ok(mut buf) = buffer.lock() else {
                        for sample in data.iter_mut() {
                            *sample = 0;
                        }
                        return;
                    };
                    let samples = buf.pop_samples(data.len());

                    for (i, sample) in data.iter_mut().enumerate() {
                        if i < samples.len() {
                            // Convert i16 to output and apply volume
                            let scaled = (f32::from(samples[i]) * vol) as i16;
                            *sample = scaled;
                        } else {
                            *sample = 0;
                        }
                    }
                },
                |err| {
                    tracing::error!("[Audio] Stream error: {}", err);
                },
                None,
            )
            .map_err(|e| AudioError::StreamCreation(e.to_string()))?;

        Ok(stream)
    }

    /// Queues audio data for playback
    pub fn queue_data(&self, data: &[u8]) {
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.push_pcm_data(data);
        }
    }

    /// Sets the playback volume
    ///
    /// # Arguments
    ///
    /// * `left` - Left channel volume (0-65535)
    /// * `right` - Right channel volume (0-65535)
    pub fn set_volume(&self, left: u16, right: u16) {
        // Average left and right channels
        let avg = u32::midpoint(u32::from(left), u32::from(right));
        // Store as atomic (lock-free)
        self.volume.store(avg, Ordering::Relaxed);
        tracing::debug!("[Audio] Volume set to {:.1}%", avg as f32 / 65535.0 * 100.0);
    }

    /// Stops audio playback
    pub fn stop(&mut self) {
        if let Some(stream) = self.stream.take() {
            drop(stream);
            tracing::debug!("[Audio] Stream stopped");
        }

        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.clear();
        }

        self.format = None;
    }

    /// Returns true if audio is currently playing
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.stream.is_some()
    }

    /// Returns the current audio format
    #[must_use]
    pub fn format(&self) -> Option<&AudioFormatInfo> {
        self.format.as_ref()
    }

    /// Returns the buffer fill level (0.0 - 1.0)
    #[must_use]
    pub fn buffer_level(&self) -> f32 {
        if let Ok(buffer) = self.buffer.lock() {
            let max = buffer.max_size;
            if max > 0 {
                return buffer.len() as f32 / max as f32;
            }
        }
        0.0
    }
}

impl Default for RdpAudioPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RdpAudioPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_buffer_push_pop() {
        let mut buffer = AudioBuffer::new();

        // Push some PCM data (4 samples = 8 bytes)
        let data = [0x00, 0x10, 0x00, 0x20, 0x00, 0x30, 0x00, 0x40];
        buffer.push_pcm_data(&data);

        assert_eq!(buffer.len(), 4);

        let samples = buffer.pop_samples(2);
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0], 0x1000); // Little-endian
        assert_eq!(samples[1], 0x2000);

        assert_eq!(buffer.len(), 2);
    }

    #[test]
    fn test_audio_buffer_overflow() {
        let mut buffer = AudioBuffer::new();
        buffer.max_size = 10;

        // Push more than max
        for i in 0..20i16 {
            let bytes = i.to_le_bytes();
            buffer.push_pcm_data(&bytes);
        }

        // Should be trimmed to max_size
        assert!(buffer.len() <= buffer.max_size);
    }

    #[test]
    fn test_audio_player_creation() {
        let player = RdpAudioPlayer::new();
        assert!(!player.is_playing());
        assert!(player.format().is_none());
    }
}
