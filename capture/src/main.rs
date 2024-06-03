#![feature(thread_sleep_until)]

use std::cell::RefCell;
use std::fs::File;
use std::io::{Read, Seek};
use std::net::{TcpStream, UdpSocket};
use std::ops::Deref;
use std::os::fd::{FromRawFd, IntoRawFd};
use std::rc::Rc;
use std::thread::{self, sleep, sleep_until};
use std::time::{Duration, Instant};

use cudarc::driver::CudaDevice;
use enigo::{Enigo, Keyboard, Settings};
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

use pulse::def::BufferAttr;
use pulse::mainloop::standard::IterateResult;
use pulse::stream::PeekResult;
use serde::{Deserialize, Serialize};
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
    is_audio: bool,

    #[serde(with = "serde_bytes")]
    data: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct KeyEvent {
    letter: char,
    state: bool,
}

pub struct Capturer {
    xconn: RustConnection,
    session: &'static Session,
    screen: Screen,
    scaler: scaling::Context,

    shm_buf: File,
    shm_seg: u32,

    in_buf: Option<Buffer<'static>>,
    out_bits: Option<Bitstream<'static>>,

    audio_stream: Rc<RefCell<pulse::stream::Stream>>,
    audio_loop: Rc<RefCell<pulse::mainloop::standard::Mainloop>>,
    _audio_ctx: Rc<RefCell<pulse::context::Context>>,
    audio_encoder: audiopus::coder::Encoder,
}

pub fn new_encoder() -> Capturer {
    println!("capture starting");

    let (xconn, screen_num) = x11rb::connect(None).unwrap();
    let screen = xconn.setup().roots[screen_num].clone();

    let shm_seg = xconn.generate_id().unwrap();
    let shm_reply = xconn
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

    let spec = pulse::sample::Spec {
        format: pulse::sample::Format::F32le,
        channels: 2,
        rate: 48000,
    };
    assert!(spec.is_valid());

    let ml = Rc::new(RefCell::new(
        pulse::mainloop::standard::Mainloop::new().unwrap(),
    ));

    let ctx = Rc::new(RefCell::new(
        pulse::context::Context::new(ml.borrow().deref(), "prospectivegopher").unwrap(),
    ));
    ctx.borrow_mut()
        .connect(None, pulse::context::FlagSet::empty(), None)
        .unwrap();
    loop {
        match ml.borrow_mut().iterate(false) {
            IterateResult::Quit(_) | IterateResult::Err(_) => panic!("ahhhhh"),
            IterateResult::Success(_) => {}
        }
        match ctx.borrow().get_state() {
            pulse::context::State::Ready => break,
            pulse::context::State::Failed | pulse::context::State::Terminated => {
                panic!("ahhhh (2)")
            }
            _ => {}
        }
    }

    let stream = Rc::new(RefCell::new(
        pulse::stream::Stream::new(&mut ctx.borrow_mut(), "desktop audio", &spec, None).unwrap(),
    ));
    stream
        .borrow_mut()
        .connect_record(
            Some("input.sink1.monitor"),
            Some(&BufferAttr {
                maxlength: 7680 * 8,
                tlength: u32::MAX,
                prebuf: u32::MAX,
                minreq: u32::MAX,
                fragsize: 7680 * 4,
            }),
            pulse::stream::FlagSet::START_CORKED | pulse::stream::FlagSet::ADJUST_LATENCY,
        )
        .unwrap();
    loop {
        match ml.borrow_mut().iterate(false) {
            IterateResult::Quit(_) | IterateResult::Err(_) => panic!("ahhhhh"),
            IterateResult::Success(_) => {}
        }
        match stream.borrow().get_state() {
            pulse::stream::State::Ready => break,
            pulse::stream::State::Failed | pulse::stream::State::Terminated => panic!("ahhhh"),
            _ => {}
        }
    }

    let mut e = Capturer {
        xconn,
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
        audio_stream: stream,
        audio_loop: ml,
        _audio_ctx: ctx,
        audio_encoder: audiopus::coder::Encoder::new(
            audiopus::SampleRate::Hz48000,
            audiopus::Channels::Stereo,
            audiopus::Application::Audio,
        )
        .unwrap(),
    };

    e.in_buf = Some(sess.create_input_buffer().unwrap());
    e.out_bits = Some(sess.create_output_bitstream().unwrap());

    e
}

impl Capturer {
    pub fn capture_frame(&mut self) -> Vec<u8> {
        // Capture screen from x11, using shared memory
        self.xconn
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

    pub fn capture_audio(&mut self) -> Vec<u8> {
        match self.audio_loop.borrow_mut().iterate(false) {
            IterateResult::Quit(_) | IterateResult::Err(_) => {
                eprintln!("Iterate state was not success, quitting...");
                return vec![];
            }
            IterateResult::Success(_) => {}
        }

        let peek_res = self.audio_stream.borrow_mut().peek().unwrap();
        match peek_res {
            PeekResult::Data(data) => {
                // println!("got buffer data len {}", data.len());
                self.audio_stream.borrow_mut().discard().unwrap();

                // Encode in batches of 20ms
                let (prefix, floats, suffix) = unsafe { data.align_to::<f32>() };
                assert!(prefix.len() == 0 && suffix.len() == 0);

                let mut encoded = vec![0; 1400];

                let count = self
                    .audio_encoder
                    .encode_float(&floats, &mut encoded)
                    .unwrap();

                encoded.truncate(count);

                return encoded;
            }
            PeekResult::Empty => {}
            PeekResult::Hole(_) => self.audio_stream.borrow_mut().discard().unwrap(),
        };

        vec![]
    }
}

fn main() {
    let mut enc = new_encoder();
    let sock = UdpSocket::bind("0.0.0.0:0").unwrap();
    sock.connect("dw.superkooks.com:42069").unwrap();
    sock.send(&vec![0]).unwrap();
    sock.recv(&mut vec![]).unwrap();

    println!("waiting for a display client");
    let mut tcp_sock = TcpStream::connect("dw.superkooks.com:42069").unwrap();

    // Forward keyboard events to application
    thread::spawn(move || {
        let mut enigo = Enigo::new(&Settings::default()).unwrap();

        loop {
            println!("will read key");
            let ev = rmp_serde::from_read::<&mut TcpStream, KeyEvent>(&mut tcp_sock).unwrap();
            println!("got key {} {}", ev.letter, ev.state);
            enigo
                .key(
                    enigo::Key::Unicode(ev.letter),
                    match ev.state {
                        true => enigo::Direction::Press,
                        false => enigo::Direction::Release,
                    },
                )
                .unwrap();
        }
    });

    sleep(Duration::from_millis(100));
    println!("got display client");

    // Begin capturing
    let mut cur_seq_video = 0u64;
    let mut cur_seq_audio = 0u64;
    enc.audio_stream.borrow_mut().uncork(None);
    // let mut last_audio = Instant::now();

    loop {
        let loop_start = Instant::now();

        // Video
        // println!("capturing...");
        // let mut t = Instant::now();
        let nalus = enc.capture_frame();
        // println!(
        //     "captured image after {} us",
        //     Instant::now().duration_since(t).as_micros()
        // );
        // t = Instant::now();

        // Packetize the nalus into mtu sized blocks
        let chunks: Vec<&[u8]> = nalus.chunks(1400).collect();
        for chunk in chunks {
            let m = Msg {
                seq: cur_seq_video,
                is_audio: false,
                data: chunk.into(),
            };
            cur_seq_video += 1;

            let buf = rmp_serde::to_vec(&m).unwrap();
            sock.send(&buf).unwrap();
            // println!("sent video packet");
        }

        // Audio
        let packet = enc.capture_audio();
        if !packet.is_empty() {
            let m = Msg {
                seq: cur_seq_audio,
                is_audio: true,
                data: packet,
            };
            cur_seq_audio += 1;

            let buf = rmp_serde::to_vec(&m).unwrap();
            sock.send(&buf).unwrap();
            // println!("sent audio packet");
            // println!(
            //     "last audio {} us ago",
            //     Instant::now().duration_since(last_audio).as_micros()
            // );
            // last_audio = Instant::now();
        }

        sleep_until(loop_start + FRAME_DURATION);
    }
}
