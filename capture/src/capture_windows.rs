use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use windows_capture::{
    capture::{CaptureControl, GraphicsCaptureApiHandler},
    monitor::Monitor,
    settings::Settings,
};

use crate::{CAPTURE_HEIGHT, CAPTURE_OFFSET_X, CAPTURE_OFFSET_Y, CAPTURE_WIDTH};

pub struct VideoCapturer {
    cur_image: Arc<Mutex<Vec<u8>>>,
    control: CaptureControl<CaptureInternal, Box<dyn std::error::Error + Send + Sync>>,
}

impl VideoCapturer {
    pub fn new() -> Self {
        let cur_image = Arc::new(Mutex::new(vec![]));

        let control = CaptureInternal::start_free_threaded(Settings::new(
            Monitor::primary().unwrap(),
            windows_capture::settings::CursorCaptureSettings::WithCursor,
            windows_capture::settings::DrawBorderSettings::Default,
            windows_capture::settings::ColorFormat::Bgra8,
            cur_image.clone(),
        ))
        .unwrap();

        Self { cur_image, control }
    }

    pub fn capture_frame(&mut self) -> Vec<u8> {
        self.cur_image.lock().unwrap().clone()
    }
}

pub struct CaptureInternal {
    cur_image: Arc<Mutex<Vec<u8>>>,
    t: Instant,
}

impl GraphicsCaptureApiHandler for CaptureInternal {
    type Flags = Arc<Mutex<Vec<u8>>>;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(cur_image: Self::Flags) -> Result<Self, Self::Error> {
        Ok(Self {
            cur_image,
            t: Instant::now(),
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut windows_capture::frame::Frame,
        _capture_control: windows_capture::graphics_capture_api::InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        self.cur_image.lock().unwrap().clear();
        self.cur_image
            .lock()
            .unwrap()
            .extend_from_slice(frame.buffer().unwrap().as_raw_buffer());

        println!(
            "{} us since last frame arrived",
            Instant::now().duration_since(self.t).as_micros()
        );
        self.t = Instant::now();

        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
