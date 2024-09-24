use cudarc::driver::CudaDevice;
use nvidia_video_codec_sdk::{
    sys::nvEncodeAPI::{
        NV_ENC_CODEC_H264_GUID, NV_ENC_MULTI_PASS, NV_ENC_PARAMS_RC_MODE, NV_ENC_PRESET_P1_GUID,
    },
    Bitstream, Buffer, EncodePictureParams, Encoder, Session,
};

use crate::{
    ui::FrameLatencyInfo, video_capture::VideoCapturer, CAPTURE_HEIGHT, CAPTURE_WIDTH, FRAME_RATE,
};

pub struct VideoEncoder {
    capturer: VideoCapturer,

    session: &'static Session,
    in_buf: Option<Buffer<'static>>,
    out_bits: Option<Bitstream<'static>>,
}

impl VideoEncoder {
    pub fn new() -> Self {
        // Create gpu encoder
        let cuda_device = CudaDevice::new(0).unwrap();
        let encoder = Encoder::initialize_with_cuda(cuda_device).unwrap();

        // Configure encoder
        let mut enc_conf = encoder.get_preset_config(
            NV_ENC_CODEC_H264_GUID,
            NV_ENC_PRESET_P1_GUID,
            nvidia_video_codec_sdk::sys::nvEncodeAPI::NV_ENC_TUNING_INFO::NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY,
        ).unwrap().presetCfg;
        enc_conf.rcParams.rateControlMode = NV_ENC_PARAMS_RC_MODE::NV_ENC_PARAMS_RC_CBR;
        enc_conf.rcParams.averageBitRate = 8 << 20;
        enc_conf.rcParams.multiPass = NV_ENC_MULTI_PASS::NV_ENC_MULTI_PASS_DISABLED;
        enc_conf.rcParams.lowDelayKeyFrameScale = 0;
        enc_conf.rcParams.enableAQ();
        unsafe {
            enc_conf.encodeCodecConfig.h264Config.repeatSPSPPS();
            enc_conf.encodeCodecConfig.h264Config.idrPeriod = 128;
            enc_conf.encodeCodecConfig.h264Config.enableLTR();
            // enc_conf.encodeCodecConfig.h264Config.sliceMode = 1;
            // enc_conf.encodeCodecConfig.h264Config.sliceModeData = 1300 - 28;
        };

        let mut init_params =
            nvidia_video_codec_sdk::sys::nvEncodeAPI::NV_ENC_INITIALIZE_PARAMS::new(
                NV_ENC_CODEC_H264_GUID,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
            );
        init_params.encode_config(&mut enc_conf);
        init_params.enable_picture_type_decision();
        init_params.display_aspect_ratio(16, 9);
        init_params.framerate(FRAME_RATE, 1);

        let session = encoder.start_session(
            nvidia_video_codec_sdk::sys::nvEncodeAPI::NV_ENC_BUFFER_FORMAT::NV_ENC_BUFFER_FORMAT_ARGB,
            init_params,
        ).unwrap();

        let sess = Box::leak(Box::new(session));

        let mut e = Self {
            capturer: VideoCapturer::new(),
            session: sess,
            in_buf: None,
            out_bits: None,
        };

        // Create input and output buffers
        e.in_buf = Some(sess.create_input_buffer().unwrap());
        e.out_bits = Some(sess.create_output_bitstream().unwrap());

        e
    }

    pub fn capture_and_encode(&mut self) -> (Vec<u8>, FrameLatencyInfo) {
        // Capture the image
        let (image, mut f) = self.capturer.capture_frame();

        // Encode the image, writing potentially multiple nalus
        unsafe { self.in_buf.as_mut().unwrap().lock().unwrap().write(&image) };
        f.measure("in_buf write");

        self.session
            .encode_picture(
                self.in_buf.as_mut().unwrap(),
                self.out_bits.as_mut().unwrap(),
                EncodePictureParams::default(),
            )
            .unwrap();
        f.measure("encode");

        let nalus = self.out_bits.as_mut().unwrap().lock().unwrap();
        f.measure("out_bits_read");

        let b = nalus.data().to_vec();
        f.measure("nalues to_vec");

        (b, f)
    }
}
