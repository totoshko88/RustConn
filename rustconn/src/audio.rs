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

// cast_precision_loss, cast_possible_truncation, unused_self allowed at workspace level

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Stream, StreamConfig};
use rustconn_core::rdp_client::AudioFormatInfo;
use thiserror::Error;

/// Error type for audio operations
#[derive(Debug, Error)]
pub enum AudioError {
    /// No audio output device available
    #[error("No audio output device available")]
    NoDevice,

    /// No audio input (microphone) device available
    #[error("No audio input device available")]
    NoInputDevice,

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

/// Cuts captured audio into the fixed-size 16-bit PCM packets that MS-RDPEAI
/// Data PDUs carry, and hands each one to `send`.
struct PacketBuilder {
    pending: Vec<u8>,
    packet_size: usize,
    send: Box<dyn FnMut(Vec<u8>) + Send>,
}

impl PacketBuilder {
    fn new(packet_size: usize, send: Box<dyn FnMut(Vec<u8>) + Send>) -> Self {
        // A zero size would never complete a packet; one sample is the floor.
        let packet_size = packet_size.max(2);
        Self {
            pending: Vec::with_capacity(packet_size * 2),
            packet_size,
            send,
        }
    }

    fn push_i16(&mut self, samples: impl IntoIterator<Item = i16>) {
        for sample in samples {
            self.pending.extend_from_slice(&sample.to_le_bytes());
        }
        while self.pending.len() >= self.packet_size {
            let rest = self.pending.split_off(self.packet_size);
            let packet = std::mem::replace(&mut self.pending, rest);
            (self.send)(packet);
        }
    }
}

/// Converts an `f32` sample in `[-1.0, 1.0]` to 16-bit PCM.
#[expect(
    clippy::cast_possible_truncation,
    reason = "clamped to the i16 range before the cast"
)]
fn f32_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
}

/// Captures the default microphone for the RDP `AUDIO_INPUT` channel.
///
/// The device is asked for the server's channel count and sample rate
/// directly — PipeWire and PulseAudio convert — as 16-bit samples, or as
/// `f32` converted here when the device offers only that. Capture stops when
/// this is dropped.
pub struct RdpMicrophone {
    _stream: Stream,
}

impl RdpMicrophone {
    /// Starts capturing, handing `send` packets of exactly `packet_size` bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if there is no input device or it cannot capture in
    /// the requested format.
    pub fn start(
        channels: u16,
        sample_rate: u32,
        packet_size: usize,
        send: impl FnMut(Vec<u8>) + Send + 'static,
    ) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or(AudioError::NoInputDevice)?;
        let config = StreamConfig {
            channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };
        let packets = Arc::new(Mutex::new(PacketBuilder::new(packet_size, Box::new(send))));

        let i16_packets = Arc::clone(&packets);
        let stream = device
            .build_input_stream(
                config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    if let Ok(mut packets) = i16_packets.lock() {
                        packets.push_i16(data.iter().copied());
                    }
                },
                |err| tracing::error!("[Audio] Microphone stream error: {}", err),
                None,
            )
            .or_else(|_| {
                device.build_input_stream(
                    config,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if let Ok(mut packets) = packets.lock() {
                            packets.push_i16(data.iter().copied().map(f32_to_i16));
                        }
                    },
                    |err| tracing::error!("[Audio] Microphone stream error: {}", err),
                    None,
                )
            })
            .map_err(|e| AudioError::StreamCreation(e.to_string()))?;
        stream
            .play()
            .map_err(|e| AudioError::Playback(e.to_string()))?;

        tracing::debug!(
            channels,
            sample_rate,
            packet_size,
            "[Audio] Microphone capture started"
        );
        Ok(Self { _stream: stream })
    }
}

/// Audio buffer for queuing PCM data
#[derive(Debug, Default)]
struct AudioBuffer {
    /// Queued audio samples (interleaved i16)
    samples: VecDeque<i16>,
    /// Maximum buffer size in samples
    max_size: usize,
    /// Whether incoming PCM data is 8-bit unsigned (converted to i16 on push).
    /// Defaults to false (16-bit signed little-endian).
    eight_bit: bool,
}

impl AudioBuffer {
    /// Creates a new audio buffer with default capacity
    fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(48_000 * 2 * 2), // ~2 seconds at 48kHz stereo
            max_size: 48_000 * 2 * 4,                         // ~4 seconds max
            eight_bit: false,
        }
    }

    /// Pushes PCM data to the buffer, converting to interleaved i16.
    ///
    /// 16-bit data is signed little-endian (two bytes per sample); 8-bit data
    /// is unsigned (one byte per sample, centred on 128) and is scaled up to the
    /// full i16 range so it plays at the correct level.
    fn push_pcm_data(&mut self, data: &[u8]) {
        if self.eight_bit {
            for &byte in data {
                let sample = (i16::from(byte) - 128) << 8;
                self.samples.push_back(sample);
            }
        } else {
            for chunk in data.as_chunks::<2>().0 {
                let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                self.samples.push_back(sample);
            }
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

        // Tell the buffer how to interpret incoming PCM (8-bit unsigned vs
        // 16-bit signed). The output stream is always i16 — 8-bit samples are
        // converted on push — so both formats share one stream builder.
        match format.bits_per_sample {
            8 | 16 => {
                if let Ok(mut buf) = self.buffer.lock() {
                    buf.eight_bit = format.bits_per_sample == 8;
                }
            }
            _ => {
                return Err(AudioError::UnsupportedFormat(format!(
                    "Unsupported bits per sample: {}",
                    format.bits_per_sample
                )));
            }
        }

        // Create stream (always i16 output)
        let buffer = Arc::clone(&self.buffer);
        let volume = Arc::clone(&self.volume);
        let stream = self.create_i16_stream(&device, config, buffer, volume)?;

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
    fn create_i16_stream(
        &self,
        device: &cpal::Device,
        config: StreamConfig,
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
    #[test]
    fn microphone_packets_are_cut_to_the_requested_size() {
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Vec<u8>>::new()));
        let sink = std::sync::Arc::clone(&sent);
        let mut packets =
            super::PacketBuilder::new(6, Box::new(move |packet| sink.lock().unwrap().push(packet)));
        // Five samples are ten bytes: one six-byte packet, four bytes pending.
        packets.push_i16([1, -1, 2, -2, 3]);
        assert_eq!(*sent.lock().unwrap(), vec![vec![1, 0, 255, 255, 2, 0]]);
        packets.push_i16([-3]);
        assert_eq!(sent.lock().unwrap().len(), 2);
        assert_eq!(sent.lock().unwrap()[1], vec![254, 255, 3, 0, 253, 255]);
    }

    #[test]
    fn f32_samples_are_clamped_to_pcm() {
        assert_eq!(super::f32_to_i16(0.0), 0);
        assert_eq!(super::f32_to_i16(1.0), i16::MAX);
        assert_eq!(super::f32_to_i16(2.0), i16::MAX);
        assert_eq!(super::f32_to_i16(-2.0), -i16::MAX);
    }

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
