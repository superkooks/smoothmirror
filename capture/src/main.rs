use std::fs::File;
use std::io::{Read, Seek};
use std::os::fd::{FromRawFd, IntoRawFd};
use std::time::{Duration, Instant};

use cudarc::driver::CudaDevice;
use ffmpeg_next::format::Pixel;
use ffmpeg_next::frame::Video;
use ffmpeg_next::software::scaling::{self, Flags};
use nvidia_video_codec_sdk::{
    sys::nvEncodeAPI::{
        NV_ENC_CODEC_H264_GUID, NV_ENC_MULTI_PASS, NV_ENC_PARAMS_RC_MODE, NV_ENC_PRESET_P1_GUID,
    },
    EncodePictureParams, Encoder,
};
use nvidia_video_codec_sdk::{Bitstream, Buffer, Session};

use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tokio::task;
use tokio::time::sleep;
use x11rb::protocol::shm::ConnectionExt;
use x11rb::protocol::xproto::Screen;
use x11rb::rust_connection::RustConnection;
use x11rb::{connection::Connection, protocol::xproto::ImageFormat};

const ENCODED_WIDTH: u32 = 1920;
const ENCODED_HEIGHT: u32 = 1080;
const FRAME_DURATION: Duration = Duration::from_micros(16_666);
const FRAME_RATE: u32 = 60;

#[derive(Serialize, Deserialize)]
struct Msg {
    seq: u64,

    #[serde(with = "serde_bytes")]
    data: Vec<u8>,
}

pub struct Capturer {
    conn: RustConnection,
    session: &'static Session,
    screen: Screen,
    scaler: scaling::Context,

    shm_buf: File,
    shm_seg: u32,

    in_buf: Option<Buffer<'static>>,
    out_bits: Option<Bitstream<'static>>,
}

pub async fn new_encoder() -> Capturer {
    println!("capture starting");

    let (conn, screen_num) = x11rb::connect(None).unwrap();
    let screen = conn.setup().roots[screen_num].clone();

    let shm_seg = conn.generate_id().unwrap();
    let shm_reply = conn
        .shm_create_segment(shm_seg, 2560 * 1440 * 4, false)
        .unwrap()
        .reply()
        .unwrap();

    let shm_buf = unsafe { File::from_raw_fd(shm_reply.shm_fd.into_raw_fd()) };

    let cuda_device = CudaDevice::new(0).unwrap();
    let encoder = Encoder::initialize_with_cuda(cuda_device).unwrap();

    let mut enc_conf = encoder.get_preset_config(
        NV_ENC_CODEC_H264_GUID,
        NV_ENC_PRESET_P1_GUID,
        nvidia_video_codec_sdk::sys::nvEncodeAPI::NV_ENC_TUNING_INFO::NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY,
    ).unwrap().presetCfg;
    enc_conf.rcParams.rateControlMode = NV_ENC_PARAMS_RC_MODE::NV_ENC_PARAMS_RC_CBR;
    enc_conf.rcParams.averageBitRate = 8 << 20;
    enc_conf.rcParams.multiPass = NV_ENC_MULTI_PASS::NV_ENC_MULTI_PASS_DISABLED;
    enc_conf.rcParams.lowDelayKeyFrameScale = 0;
    unsafe {
        enc_conf.encodeCodecConfig.h264Config.repeatSPSPPS();
        enc_conf.encodeCodecConfig.h264Config.idrPeriod = 128;
        // enc_conf.encodeCodecConfig.h264Config.sliceMode = 1;
        // enc_conf.encodeCodecConfig.h264Config.sliceModeData = 1300 - 28;
    };

    let mut init_params = nvidia_video_codec_sdk::sys::nvEncodeAPI::NV_ENC_INITIALIZE_PARAMS::new(
        NV_ENC_CODEC_H264_GUID,
        ENCODED_WIDTH,
        ENCODED_HEIGHT,
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

    let mut e = Capturer {
        conn,
        session: sess,
        screen,
        shm_buf,
        shm_seg,
        in_buf: None,
        out_bits: None,
        scaler: scaling::Context::get(
            Pixel::BGRA,
            2560,
            1440,
            Pixel::BGRA,
            ENCODED_WIDTH,
            ENCODED_HEIGHT,
            Flags::FAST_BILINEAR,
        )
        .unwrap(),
    };

    e.in_buf = Some(sess.create_input_buffer().unwrap());
    e.out_bits = Some(sess.create_output_bitstream().unwrap());

    // println!("waiting on ready");
    // ready.notified().await;
    // println!("running now");

    e
}

impl Capturer {
    pub async fn capture(&mut self) -> Vec<u8> {
        // Capture screen from x11, using shared memory
        self.conn
            .shm_get_image(
                self.screen.root,
                3840,
                720,
                2560,
                1440,
                0x00ffffff,
                ImageFormat::Z_PIXMAP.into(),
                self.shm_seg,
                0,
            )
            .unwrap()
            .reply()
            .unwrap();

        let mut image = vec![];
        self.shm_buf.seek(std::io::SeekFrom::Start(0)).unwrap();
        self.shm_buf.read_to_end(&mut image).unwrap();

        // Resize the image
        let mut input = Video::new(Pixel::BGRA, 2560, 1440);
        let mut output = Video::empty();
        input.data_mut(0).copy_from_slice(&image);
        self.scaler.run(&input, &mut output).unwrap();

        // Encode the image, writing potentially multiple nalus
        unsafe {
            self.in_buf
                .as_mut()
                .unwrap()
                .lock()
                .unwrap()
                .write(&output.data(0))
        };
        self.session
            .encode_picture(
                self.in_buf.as_mut().unwrap(),
                self.out_bits.as_mut().unwrap(),
                EncodePictureParams::default(),
            )
            .unwrap();

        let nalus = self.out_bits.as_mut().unwrap().lock().unwrap();

        return nalus.data().to_vec();
    }
}

#[tokio::main]
async fn main() {
    let local = task::LocalSet::new();
    local
        .run_until(async move {
            let mut enc = new_encoder().await;
            let sock = UdpSocket::bind("0.0.0.0:0").await.unwrap();
            sock.connect("10.8.0.1:42069").await.unwrap();

            sock.send(&vec![0]).await.unwrap();

            println!("waiting for a display client");
            let mut buf = vec![];
            sock.recv_buf(&mut buf).await.unwrap();
            sock.connect(std::str::from_utf8(&buf).unwrap())
                .await
                .unwrap();

            sleep(Duration::from_millis(100)).await;
            println!("got display client");

            // Begin capturing
            let mut cur_seq = 0u64;
            let mut ticker = tokio::time::interval(FRAME_DURATION);

            loop {
                println!("capturing...");
                let mut t = Instant::now();
                let nalus = enc.capture().await;
                println!(
                    "captured image after {} us",
                    Instant::now().duration_since(t).as_micros()
                );
                t = Instant::now();

                // Packetize the nalus into mtu sized blocks
                let chunks: Vec<&[u8]> = nalus.chunks(1400).collect();
                for chunk in chunks {
                    let m = Msg {
                        seq: cur_seq,
                        data: chunk.into(),
                    };
                    cur_seq += 1;

                    let buf = rmp_serde::to_vec(&m).unwrap();
                    sock.send(&buf).await.unwrap();
                    println!("sent packet");
                }

                ticker.tick().await;
            }
        })
        .await;
}
