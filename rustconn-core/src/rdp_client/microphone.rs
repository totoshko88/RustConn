//! Microphone redirection for the embedded RDP client (MS-RDPEAI).
//!
//! The server opens the `AUDIO_INPUT` dynamic channel and picks a format; the
//! [`RdpeaiClient`](ironrdp_rdpeai::client::RdpeaiClient) negotiates it and asks
//! [`RustConnMicrophone`] to start capturing. Capture itself runs in the GUI,
//! which owns the audio device as it does for playback: this handler only
//! reports the format the GUI must capture in
//! ([`RdpClientEvent::MicrophoneOpened`]) and keeps the packet sink that the
//! GUI's packets ([`RdpClientCommand::MicrophoneData`]) are handed to.
//!
//! [`RdpClientCommand::MicrophoneData`]: super::RdpClientCommand::MicrophoneData

use std::sync::{Arc, Mutex};

use ironrdp_rdpeai::AudioFormat;
use ironrdp_rdpeai::client::{AudioPacketSink, RdpeaiCaptureHandler};
use ironrdp_rdpeai::pdu::pcm_format;

use super::RdpClientEvent;

/// Where captured packets go while the server has the microphone open.
///
/// Shared between the capture handler, which fills it on open and empties it
/// on close, and the session loop, which hands it each packet from the GUI.
pub type MicrophoneSink = Arc<Mutex<Option<AudioPacketSink>>>;

/// The formats offered to the server, in order of preference: 16-bit PCM,
/// which every Windows host accepts and which needs no encoder.
fn offered_formats() -> Vec<AudioFormat> {
    vec![
        pcm_format(2, 44_100, 16),
        pcm_format(1, 44_100, 16),
        pcm_format(2, 22_050, 16),
        pcm_format(1, 22_050, 16),
    ]
}

/// `AUDIO_INPUT` capture handler that delegates the device to the GUI.
pub struct RustConnMicrophone {
    formats: Vec<AudioFormat>,
    sink: MicrophoneSink,
    event_tx: std::sync::mpsc::Sender<RdpClientEvent>,
}

impl RustConnMicrophone {
    /// Creates the handler; `sink` is shared with the session loop.
    #[must_use]
    pub fn new(sink: MicrophoneSink, event_tx: std::sync::mpsc::Sender<RdpClientEvent>) -> Self {
        Self {
            formats: offered_formats(),
            sink,
            event_tx,
        }
    }

    /// Asks the GUI to capture in `format`, delivering `packet_size` bytes at a time.
    fn request_capture(&self, format: &AudioFormat, packet_size: usize) {
        tracing::info!(
            channels = format.n_channels,
            samples_per_sec = format.n_samples_per_sec,
            bits_per_sample = format.bits_per_sample,
            packet_size,
            "Microphone opened by the server"
        );
        let _ = self.event_tx.send(RdpClientEvent::MicrophoneOpened {
            channels: format.n_channels,
            samples_per_sec: format.n_samples_per_sec,
            bits_per_sample: format.bits_per_sample,
            packet_size,
        });
    }
}

impl RdpeaiCaptureHandler for RustConnMicrophone {
    fn supported_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn open(
        &mut self,
        capture_format: &AudioFormat,
        _encode_format: &AudioFormat,
        packet_size: usize,
        sink: AudioPacketSink,
    ) -> i32 {
        // Only PCM is offered, so the capture and encoding formats are the same.
        if let Ok(mut slot) = self.sink.lock() {
            *slot = Some(sink);
        }
        self.request_capture(capture_format, packet_size);
        0
    }

    fn set_format(&mut self, encode_format: &AudioFormat, packet_size: usize) -> bool {
        self.request_capture(encode_format, packet_size);
        true
    }

    fn close(&mut self) {
        if let Ok(mut slot) = self.sink.lock() {
            *slot = None;
        }
        tracing::info!("Microphone closed by the server");
        let _ = self.event_tx.send(RdpClientEvent::MicrophoneClosed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_hands_the_sink_over_and_tells_the_gui_the_format() {
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let sink: MicrophoneSink = Arc::new(Mutex::new(None));
        let mut mic = RustConnMicrophone::new(Arc::clone(&sink), event_tx);

        let format = mic.supported_formats()[0].clone();
        let received = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let received_by_sink = Arc::clone(&received);
        assert_eq!(
            mic.open(
                &format,
                &format,
                1764,
                Box::new(move |packet| received_by_sink.lock().unwrap().push(packet)),
            ),
            0
        );
        assert!(matches!(
            event_rx.try_recv(),
            Ok(RdpClientEvent::MicrophoneOpened {
                channels: 2,
                samples_per_sec: 44_100,
                bits_per_sample: 16,
                packet_size: 1764,
            })
        ));

        // The session loop feeds the GUI's packets through the shared slot.
        if let Some(sink) = sink.lock().unwrap().as_mut() {
            sink(vec![1, 2, 3]);
        }
        assert_eq!(*received.lock().unwrap(), vec![vec![1, 2, 3]]);

        mic.close();
        assert!(sink.lock().unwrap().is_none());
        assert!(matches!(
            event_rx.try_recv(),
            Ok(RdpClientEvent::MicrophoneClosed)
        ));
    }

    #[test]
    fn only_16_bit_pcm_is_offered() {
        for format in offered_formats() {
            assert_eq!(format.format, ironrdp_rdpeai::WaveFormat::PCM);
            assert_eq!(format.bits_per_sample, 16);
        }
    }
}
