use std::{
    fs::File,
    io::{Read, Seek},
    os::fd::{FromRawFd, IntoRawFd},
};

use x11rb::{
    connection::Connection,
    protocol::{
        shm::ConnectionExt,
        xfixes::ConnectionExt as _,
        xproto::{ImageFormat, Screen},
    },
    rust_connection::RustConnection,
};

use crate::{
    ui::FrameLatencyInfo, CAPTURE_HEIGHT, CAPTURE_OFFSET_X, CAPTURE_OFFSET_Y, CAPTURE_WIDTH,
};

pub struct VideoCapturer {
    xconn: RustConnection,
    screen: Screen,

    shm_buf: File,
    shm_seg: u32,
}

impl VideoCapturer {
    pub fn new() -> Self {
        // Connect to X11
        let (xconn, screen_num) = x11rb::connect(None).unwrap();
        let screen = xconn.setup().roots[screen_num].clone();

        // Negotiate XFixes version
        xconn.xfixes_query_version(6, 0).unwrap().reply().unwrap();

        // Create shared memory segment for capturing frames
        let shm_seg = xconn.generate_id().unwrap();
        let shm_reply = xconn
            .shm_create_segment(shm_seg, CAPTURE_WIDTH * CAPTURE_HEIGHT * 4, false)
            .unwrap()
            .reply()
            .unwrap();

        let shm_buf = unsafe { File::from_raw_fd(shm_reply.shm_fd.into_raw_fd()) };

        Self {
            xconn,
            screen,
            shm_buf,
            shm_seg,
        }
    }

    pub fn capture_frame(&mut self) -> (Vec<u8>, FrameLatencyInfo) {
        let mut f = FrameLatencyInfo::new();
        // Capture screen from x11, using shared memory
        self.xconn
            .shm_get_image(
                self.screen.root,
                CAPTURE_OFFSET_X as i16,
                CAPTURE_OFFSET_Y as i16,
                CAPTURE_WIDTH as u16,
                CAPTURE_HEIGHT as u16,
                0x00ffffff,
                ImageFormat::Z_PIXMAP.into(),
                self.shm_seg,
                0,
            )
            .unwrap()
            .reply()
            .unwrap();

        f.measure("shm_get_image");

        let mut image = vec![];
        self.shm_buf.seek(std::io::SeekFrom::Start(0)).unwrap();
        self.shm_buf.read_to_end(&mut image).unwrap();
        f.measure("shm_buf read");

        // Capture cursor
        let cursor = self
            .xconn
            .xfixes_get_cursor_image()
            .unwrap()
            .reply()
            .unwrap();
        f.measure("get_cursor_image");

        let ox = cursor.x as i64 - CAPTURE_OFFSET_X as i64;
        let oy = cursor.y as i64 - CAPTURE_OFFSET_Y as i64;

        // Copy cursor onto image if it is within bounds
        if ox >= 0 && ox <= CAPTURE_WIDTH as i64 && oy >= 0 && oy <= CAPTURE_HEIGHT as i64 {
            for x in (ox as i64 - cursor.xhot as i64)
                ..(ox as i64 + cursor.width as i64 - cursor.xhot as i64)
            {
                for y in (oy as i64 - cursor.yhot as i64)
                    ..(oy as i64 + cursor.height as i64 - cursor.yhot as i64)
                {
                    let cx = x - ox as i64 + cursor.xhot as i64;
                    let cy = y - oy as i64 + cursor.yhot as i64;
                    let idx = (cx + cy * cursor.width as i64) as usize;
                    let cb = cursor.cursor_image[idx];

                    let img_offset = (y * CAPTURE_WIDTH as i64 + x) * 4;
                    if img_offset < 0 || img_offset >= image.len() as i64 {
                        continue;
                    }

                    if cb >> 24 as u8 == 0 {
                        continue;
                    }

                    image[img_offset as usize] = cb as u8;
                    image[img_offset as usize + 1] = (cb >> 8) as u8;
                    image[img_offset as usize + 2] = (cb >> 16) as u8;
                }
            }
        }
        f.measure("composite cursor");

        (image, f)
    }
}
