pub struct AudioCapturer {}

impl AudioCapturer {
    pub fn new() -> Self {
        Self {}
    }

    pub fn capture_audio(&mut self) -> &[f32] {
        &[]
    }

    pub fn uncork(&self) {}
}
