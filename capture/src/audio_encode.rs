use crate::audio_capture::AudioCapturer;

pub struct AudioEncoder {
    pub source: AudioCapturer,
    encoder: audiopus::coder::Encoder,
}

impl AudioEncoder {
    pub fn new() -> Self {
        Self {
            source: AudioCapturer::new(),
            encoder: audiopus::coder::Encoder::new(
                audiopus::SampleRate::Hz48000,
                audiopus::Channels::Stereo,
                audiopus::Application::Audio,
            )
            .unwrap(),
        }
    }

    pub fn capture_and_encode(&mut self) -> Option<Vec<u8>> {
        let floats = self.source.capture_audio();
        if floats.len() == 0 {
            return None;
        }

        let mut encoded = vec![0; 1400];
        let count = self.encoder.encode_float(&floats, &mut encoded).unwrap();

        encoded.truncate(count);

        Some(encoded)
    }
}
