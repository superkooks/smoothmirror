#![feature(thread_sleep_until)]

mod audio_encode;
mod ui;

#[cfg_attr(target_os = "linux", path = "audio_linux.rs")]
#[cfg_attr(target_os = "windows", path = "audio_windows.rs")]
mod audio_capture;

#[cfg_attr(target_os = "linux", path = "capture_linux.rs")]
#[cfg_attr(target_os = "windows", path = "capture_windows.rs")]
mod video_capture;

#[cfg(target_os = "linux")]
mod gamepad_linux;
#[cfg_attr(feature = "nvenc", path = "encode_nvidia.rs")]
#[cfg_attr(not(feature = "nvenc"), path = "encode_ffmpeg.rs")]
mod video_encode;

use std::net::{TcpStream, UdpSocket};
use std::thread::{self, sleep, sleep_until};
use std::time::{Duration, Instant};

use audio_encode::AudioEncoder;
use enigo::{Enigo, Keyboard, Mouse, Settings};

use gamepad_linux::GamepadEmulator;
use log::info;
use serde::{Deserialize, Serialize};
use ui::FrameLatencyInfo;
use video_encode::VideoEncoder;

const FRAME_DURATION: Duration = Duration::from_micros(16_666);
const FRAME_RATE: u32 = 60;

const CAPTURE_WIDTH: u32 = 2560;
const CAPTURE_HEIGHT: u32 = 1440;
const CAPTURE_OFFSET_X: u32 = 3840;
const CAPTURE_OFFSET_Y: u32 = 240;

#[derive(Serialize, Deserialize)]
struct Msg {
    seq: i64,
    is_audio: bool,

    #[serde(with = "serde_bytes")]
    data: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
enum KeyEvent {
    Key { letter: char, state: bool },
    Mouse { x: f64, y: f64 },
    Click { button: i32, state: bool },
    GamepadButton { button: gilrs::Button, state: u8 },
    GamepadAxis { axis: gilrs::Axis, state: f32 },
}

pub struct Capturer {
    audio: AudioEncoder,
    video: VideoEncoder,
}

pub fn new_encoder() -> Capturer {
    // info!("capture starting");

    let audio = AudioEncoder::new();
    let video = VideoEncoder::new();

    Capturer { audio, video }
}

fn main() {
    let (ui, ui_thread) = ui::start_ui();
    log::set_boxed_logger(Box::new(ui::Logger(ui.clone()))).unwrap();
    log::set_max_level(log::LevelFilter::Info);

    let mut gpe = GamepadEmulator::new();

    let mut enc = new_encoder();
    info!("waiting for a display client");

    let mut sock = UdpSocket::bind("0.0.0.0:0").unwrap();
    sock.connect("dw.superkooks.com:42069").unwrap();
    sock.send(&vec![0]).unwrap();
    let net_hand = thread::spawn(move || {
        sock.recv(&mut vec![0]).unwrap();
        sock
    });
    loop {
        if net_hand.is_finished() {
            sock = net_hand.join().unwrap();
            break;
        } else if ui_thread.is_finished() {
            return;
        }
    }

    let mut tcp_sock = TcpStream::connect("dw.superkooks.com:42069").unwrap();
    tcp_sock.set_nodelay(true).unwrap();

    // Forward keyboard events to application
    thread::spawn(move || {
        let mut enigo = Enigo::new(&Settings::default()).unwrap();
        let mut t = Instant::now();

        loop {
            let ev = rmp_serde::from_read::<&mut TcpStream, KeyEvent>(&mut tcp_sock).unwrap();
            match ev {
                KeyEvent::Key { letter, state } => {
                    enigo
                        .key(
                            enigo::Key::Unicode(letter),
                            match state {
                                true => enigo::Direction::Press,
                                false => enigo::Direction::Release,
                            },
                        )
                        .unwrap();
                }
                KeyEvent::Click { button, state } => {
                    enigo
                        .button(
                            match button {
                                0 => enigo::Button::Left,
                                1 => enigo::Button::Middle,
                                2 => enigo::Button::Right,
                                _ => panic!("invalid button"),
                            },
                            match state {
                                true => enigo::Direction::Press,
                                false => enigo::Direction::Release,
                            },
                        )
                        .unwrap();
                }
                KeyEvent::Mouse { x, y } => {
                    // println!("{} {}", x, y);
                    // println!(
                    //     "last mouse {} us ago",
                    //     Instant::now().duration_since(t).as_micros()
                    // );
                    t = Instant::now();
                    enigo
                        .move_mouse(x as i32, y as i32, enigo::Coordinate::Rel)
                        .unwrap();
                }
                _ => {
                    gpe.send_gamepad_event(ev);
                }
            }
        }
    });

    // let mut socket = TcpListener::bind("localhost:9999").unwrap();
    // let mut conn = socket.accept().unwrap().0;

    sleep(Duration::from_millis(100));
    info!("got display client");

    // Begin capturing
    let mut cur_seq_video = 0i64;
    let mut cur_seq_audio = 0i64;
    enc.audio.source.uncork();

    let mut f = FrameLatencyInfo::new();
    loop {
        let loop_start = Instant::now();
        let mut main_fli = FrameLatencyInfo::new();

        if ui_thread.is_finished() {
            return;
        }

        // Video
        // println!("capturing...");
        // let mut t = Instant::now();
        let (nalus, fli) = enc.video.capture_and_encode();
        main_fli.measure("capture");
        ui.lock().unwrap().add_frame_latency_info("frame", fli);
        main_fli.measure("ui frame fli");
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

            f.measure("last_packet");
            if f.total() > 2500 {
                ui.lock().unwrap().add_frame_latency_info("packet", f);
                f = FrameLatencyInfo::new();
            }
            // conn.write_all(&buf).unwrap();
            // println!("sent video packet");
            // println!(
            //     "last video {} us ago",
            //     Instant::now().duration_since(last_video).as_micros()
            // );
        }
        main_fli.measure("packetize video");

        // Audio
        let packet = enc.audio.capture_and_encode();
        main_fli.measure("capture audio");
        if packet.is_some() {
            let m = Msg {
                seq: cur_seq_audio,
                is_audio: true,
                data: packet.unwrap(),
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
        main_fli.measure("packetize audio");

        sleep_until(loop_start + FRAME_DURATION);
        main_fli.measure("sleep");
        ui.lock()
            .unwrap()
            .add_frame_latency_info("main_loop", main_fli);
    }
}
