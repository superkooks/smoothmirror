use std::ffi::CString;

use ffmpeg_sys_next::{self as ffmpeg, av_frame_alloc};

use crate::{video_capture::VideoCapturer, CAPTURE_HEIGHT, CAPTURE_WIDTH, FRAME_RATE};

pub struct VideoEncoder {
    capturer: VideoCapturer,
    encoder: *mut ffmpeg::AVCodecContext,
    scaler: *mut ffmpeg::SwsContext,
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

        let scaler = unsafe {
            ffmpeg::sws_getContext(
                CAPTURE_WIDTH as i32,
                CAPTURE_HEIGHT as i32,
                ffmpeg::AVPixelFormat::AV_PIX_FMT_BGRA,
                CAPTURE_WIDTH as i32,
                CAPTURE_HEIGHT as i32,
                ffmpeg::AVPixelFormat::AV_PIX_FMT_YUV420P,
                ffmpeg::SWS_BILINEAR,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };

        Self {
            capturer: VideoCapturer::new(),
            encoder,
            scaler,
        }
    }

    pub fn capture_and_encode(&mut self) -> Vec<u8> {
        let image = self.capturer.capture_frame();

        // Allocate the RGB frame for the captured image
        let mut rgb_frame = unsafe { ffmpeg::av_frame_alloc() };
        unsafe {
            (*rgb_frame).format = ffmpeg::AVPixelFormat::AV_PIX_FMT_BGRA as i32;
            (*rgb_frame).width = CAPTURE_WIDTH as i32;
            (*rgb_frame).height = CAPTURE_HEIGHT as i32;
        };

        if unsafe { ffmpeg::av_frame_get_buffer(rgb_frame, 0) } < 0 {
            panic!("could not allocate avframe buffer for rgb_frame");
        }
        if unsafe { ffmpeg::av_frame_make_writable(rgb_frame) } < 0 {
            panic!("could not make rgb_frame writable");
        }

        // Copy the image into the frame
        unsafe { std::ptr::copy(image.as_ptr(), (*rgb_frame).data[0], image.len()) };

        // Convert the frame to YUV420
        let mut yuv_frame = unsafe { av_frame_alloc() };
        if unsafe { ffmpeg::sws_scale_frame(self.scaler, yuv_frame, rgb_frame) } < 0 {
            panic!("failed to scale frame");
        }
        unsafe { ffmpeg::av_frame_free(std::ptr::addr_of_mut!(rgb_frame)) };

        // Encode the frame
        if unsafe { ffmpeg::avcodec_send_frame(self.encoder, yuv_frame) } < 0 {
            panic!("failed to submit frame to encoder");
        }
        unsafe { ffmpeg::av_frame_free(std::ptr::addr_of_mut!(yuv_frame)) };

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
