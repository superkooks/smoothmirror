use std::{ffi::CString, time::Instant};

use ffmpeg_sys_next as ffmpeg;

use crate::{video_capture::VideoCapturer, CAPTURE_HEIGHT, CAPTURE_WIDTH, FRAME_RATE};

pub struct VideoEncoder {
    capturer: VideoCapturer,
    encoder: *mut ffmpeg::AVCodecContext,
    pts: i64,
}

impl VideoEncoder {
    pub fn new() -> Self {
        let codec = unsafe { ffmpeg::avcodec_find_encoder(ffmpeg::AVCodecID::AV_CODEC_ID_H264) };

        let encoder = unsafe { ffmpeg::avcodec_alloc_context3(codec) };

        unsafe {
            (*encoder).sample_aspect_ratio.num = 16;
            (*encoder).sample_aspect_ratio.den = 9;
            (*encoder).time_base.num = 1;
            (*encoder).time_base.den = FRAME_RATE as i32;
            (*encoder).framerate.num = FRAME_RATE as i32;
            (*encoder).framerate.den = 1;
            (*encoder).bit_rate = 8 << 20;
            (*encoder).width = CAPTURE_WIDTH as i32;
            (*encoder).height = CAPTURE_HEIGHT as i32;
            (*encoder).pix_fmt = ffmpeg::AVPixelFormat::AV_PIX_FMT_YUV420P;

            let name = CString::new("preset").unwrap();
            let val = CString::new("ultrafast").unwrap();
            ffmpeg::av_opt_set((*encoder).priv_data, name.as_ptr(), val.as_ptr(), 0);

            let name = CString::new("tune").unwrap();
            let val = CString::new("zerolatency").unwrap();
            ffmpeg::av_opt_set((*encoder).priv_data, name.as_ptr(), val.as_ptr(), 0);
        }

        unsafe {
            ffmpeg::avcodec_open2(encoder, codec, std::ptr::null_mut());
        }

        Self {
            capturer: VideoCapturer::new(),
            encoder,
            pts: 0,
        }
    }

    pub fn capture_and_encode(&mut self) -> Vec<u8> {
        let t = Instant::now();
        let image = self.capturer.capture_frame();
        println!(
            "captured frame after {} us",
            Instant::now().duration_since(t).as_micros()
        );

        // Allocate the RGB frame for the converted image
        let mut yuv_frame = unsafe { ffmpeg::av_frame_alloc() };
        unsafe {
            (*yuv_frame).format = ffmpeg::AVPixelFormat::AV_PIX_FMT_YUV420P as i32;
            (*yuv_frame).width = CAPTURE_WIDTH as i32;
            (*yuv_frame).height = CAPTURE_HEIGHT as i32;
        };

        if unsafe { ffmpeg::av_frame_get_buffer(yuv_frame, 0) } < 0 {
            panic!("could not allocate avframe buffer for yuv_frame");
        }
        if unsafe { ffmpeg::av_frame_make_writable(yuv_frame) } < 0 {
            panic!("could not make yuv_frame writable");
        }

        // Convert the frame into YUV420
        let mut y_plane = unsafe {
            std::slice::from_raw_parts_mut((*yuv_frame).data[0], (*(*yuv_frame).buf[0]).size)
        };
        let mut u_plane = unsafe {
            std::slice::from_raw_parts_mut((*yuv_frame).data[1], (*(*yuv_frame).buf[0]).size)
        };
        let mut v_plane = unsafe {
            std::slice::from_raw_parts_mut((*yuv_frame).data[2], (*(*yuv_frame).buf[0]).size)
        };

        yuvutils_rs::bgra_to_yuv420(
            &mut y_plane,
            unsafe { (*yuv_frame).linesize[0] } as u32,
            &mut u_plane,
            unsafe { (*yuv_frame).linesize[1] } as u32,
            &mut v_plane,
            unsafe { (*yuv_frame).linesize[2] } as u32,
            &image,
            CAPTURE_WIDTH * 4,
            CAPTURE_WIDTH,
            CAPTURE_HEIGHT,
            yuvutils_rs::YuvRange::Full,
            yuvutils_rs::YuvStandardMatrix::Bt709,
        );

        // Set the presentation timestamp
        unsafe {
            (*yuv_frame).pts = self.pts;
        }
        self.pts += 1;

        println!(
            "converted frame after {} us",
            Instant::now().duration_since(t).as_micros()
        );

        // Encode the frame
        if unsafe { ffmpeg::avcodec_send_frame(self.encoder, yuv_frame) } < 0 {
            panic!("failed to submit frame to encoder");
        }
        unsafe { ffmpeg::av_frame_free(std::ptr::addr_of_mut!(yuv_frame)) };

        println!(
            "encoded frame after {} us",
            Instant::now().duration_since(t).as_micros()
        );

        let mut out = vec![];
        let ret = 0;
        while ret >= 0 {
            // Allocate and receive packet
            let mut pkt = unsafe { ffmpeg::av_packet_alloc() };
            let ret = unsafe { ffmpeg::avcodec_receive_packet(self.encoder, pkt) };

            if ret == -ffmpeg::EAGAIN {
                unsafe { ffmpeg::av_packet_free(std::ptr::addr_of_mut!(pkt)) };
                return out;
            } else if ret < 0 {
                panic!("failed to receive encoded packet: {}", ret);
            }

            let data = unsafe { std::slice::from_raw_parts((*pkt).data, (*pkt).size as usize) };
            out.extend_from_slice(data);

            unsafe { ffmpeg::av_packet_free(std::ptr::addr_of_mut!(pkt)) };
        }

        return out;
    }
}
