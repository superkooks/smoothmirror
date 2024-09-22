use std::{
    io::Write,
    net::{SocketAddr, TcpStream, UdpSocket},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_std::task::sleep;
use audiopus::{packet::Packet, MutSignals};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Sample, SampleRate, StreamConfig,
};
use ffmpeg_sys_next::{self as ffmpeg};
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use winit::{
    dpi::{PhysicalPosition, PhysicalSize, Size},
    event::{Event, WindowEvent},
    event_loop::EventLoop,
    window::{Window, WindowBuilder},
};

const ENCODED_WIDTH: u32 = 2560;
const ENCODED_HEIGHT: u32 = 1440;
const FRAME_DURATION: Duration = Duration::from_micros(16_666);

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
}

struct Client {
    queue: wgpu::Queue,
    decoder: *mut ffmpeg::AVCodecContext,
    parser: *mut ffmpeg::AVCodecParserContext,
    surface: wgpu::Surface,
    scaler: *mut ffmpeg::SwsContext,

    audio_decoder: audiopus::coder::Decoder,
    decoded_audio: Arc<Mutex<Vec<f32>>>,
}

async fn init(window: &Window, decoded_audio: Arc<Mutex<Vec<f32>>>) -> Client {
    // Init graphics
    let size = window.inner_size();

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());

    let surface = unsafe { instance.create_surface(window) }.unwrap();

    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        })
        .await
        .unwrap();

    let (device, queue) = adapter
        .request_device(
            &wgpu::DeviceDescriptor {
                features: wgpu::Features::empty(),
                limits: wgpu::Limits::default(),
                label: None,
            },
            None,
        )
        .await
        .unwrap();

    let surface_caps = surface.get_capabilities(&adapter);
    let surface_format = surface_caps
        .formats
        .iter()
        .find(|f| f.is_srgb())
        .copied()
        .unwrap_or(surface_caps.formats[0]);

    println!("using surface format {:?}", surface_format);

    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::COPY_DST,
        format: surface_format,
        width: size.width,
        height: size.height,
        present_mode: surface_caps.present_modes[0],
        alpha_mode: surface_caps.alpha_modes[0],
        view_formats: vec![],
    };

    surface.configure(&device, &config);

    println!(
        "texture has width {:?} for screen width {:?}",
        surface.get_current_texture().unwrap().texture.width(),
        size.width
    );

    // Create video decoder
    let codec = unsafe { ffmpeg::avcodec_find_decoder(ffmpeg::AVCodecID::AV_CODEC_ID_H264) };

    let parser = unsafe { ffmpeg::av_parser_init((*codec).id as i32) };

    let decoder = unsafe { ffmpeg::avcodec_alloc_context3(codec) };

    unsafe {
        ffmpeg::avcodec_open2(decoder, codec, std::ptr::null_mut());
    }

    let scaler = unsafe {
        ffmpeg::sws_getContext(
            ENCODED_WIDTH as i32,
            ENCODED_HEIGHT as i32,
            ffmpeg::AVPixelFormat::AV_PIX_FMT_YUV420P,
            size.width as i32,
            size.height as i32,
            ffmpeg::AVPixelFormat::AV_PIX_FMT_BGRA,
            ffmpeg::SWS_BILINEAR,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };

    Client {
        queue,
        decoder,
        parser,
        surface,
        scaler,
        audio_decoder: audiopus::coder::Decoder::new(
            audiopus::SampleRate::Hz48000,
            audiopus::Channels::Stereo,
        )
        .unwrap(),
        decoded_audio,
    }
}

impl Client {
    fn consume_nalu(&mut self, mut nals: *mut ffmpeg::AVPacket) {
        let mut t = Instant::now();
        let res = unsafe { ffmpeg::avcodec_send_packet(self.decoder, nals) };
        unsafe { ffmpeg::av_packet_free(std::ptr::addr_of_mut!(nals)) };
        println!("send_packet={}", res);
        // println!(
        //     "tooked {} us to send packet",
        //     Instant::now().duration_since(t).as_micros()
        // );
        println!(
            "took {} us to decode",
            Instant::now().duration_since(t).as_micros()
        );
        t = Instant::now();

        let mut yuv_frame = unsafe { ffmpeg::av_frame_alloc() };
        let res2 = unsafe { ffmpeg::avcodec_receive_frame(self.decoder, yuv_frame) };
        println!("receive_frame={}", res2);

        if res2 < 0 {
            return;
        }

        let mut rgb_frame = unsafe { ffmpeg::av_frame_alloc() };
        let res3 = unsafe { ffmpeg::sws_scale_frame(self.scaler, rgb_frame, yuv_frame) };
        unsafe { ffmpeg::av_frame_free(std::ptr::addr_of_mut!(yuv_frame)) };
        println!("sws_scale_frame={}", res3);
        println!(
            "took {} us to convert",
            Instant::now().duration_since(t).as_micros()
        );
        t = Instant::now();

        let output = self.surface.get_current_texture().unwrap();

        let rescaled_width = unsafe { *rgb_frame }.width as u32;
        let rescaled_height = unsafe { *rgb_frame }.height as u32;

        self.queue.write_texture(
            wgpu::ImageCopyTextureBase {
                texture: &output.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            unsafe {
                std::slice::from_raw_parts((*rgb_frame).data[0], (*(*rgb_frame).buf[0]).size)
            },
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * rescaled_width),
                rows_per_image: Some(rescaled_height),
            },
            wgpu::Extent3d {
                width: rescaled_width,
                height: rescaled_height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(std::iter::empty());
        println!(
            "took {} us to submit to queue",
            Instant::now().duration_since(t).as_micros()
        );
        t = Instant::now();
        output.present();
        println!(
            "took {} us to present",
            Instant::now().duration_since(t).as_micros()
        );

        unsafe { ffmpeg::av_frame_free(std::ptr::addr_of_mut!(rgb_frame)) };

        // println!("presenting")
        // }
    }

    fn accumulate_nalus(&mut self, mut msg: &[u8]) {
        let mut pkt = unsafe { ffmpeg::av_packet_alloc() };
        while msg.len() > 0 {
            let n = unsafe {
                ffmpeg::av_parser_parse2(
                    self.parser,
                    self.decoder,
                    std::ptr::addr_of_mut!((*pkt).data),
                    std::ptr::addr_of_mut!((*pkt).size),
                    msg.as_ptr(),
                    msg.len() as i32,
                    ffmpeg::AV_NOPTS_VALUE,
                    ffmpeg::AV_NOPTS_VALUE,
                    0,
                )
            };

            if n < 0 {
                panic!("av_parser_parse2={}", n);
            }

            msg = &msg[n as usize..];

            if unsafe { *pkt }.size > 0 {
                self.consume_nalu(pkt);
                pkt = unsafe { ffmpeg::av_packet_alloc() };
            }
        }
    }
}

struct UdpStream {
    next_seq: i64,
    last_in_seq: Instant,
    rearrange_buf: Vec<Msg>,
}

impl UdpStream {
    fn new() -> Self {
        return Self {
            next_seq: 0,
            last_in_seq: Instant::now(),
            rearrange_buf: vec![],
        };
    }

    fn recv(&mut self, msg: Msg) -> Vec<Msg> {
        let mut out = vec![];

        if Instant::now().duration_since(self.last_in_seq).as_micros()
            > FRAME_DURATION.as_micros() * 2
            && msg.seq - self.next_seq > 1
        {
            self.next_seq += 2;
        }

        if msg.seq != self.next_seq {
            // Add it to the rearrange buf
            self.rearrange_buf.push(msg);
            println!("storing packet in rearrange buf")
        } else {
            // Write it
            out.push(msg);
            self.next_seq += 1;
            self.last_in_seq = Instant::now();
        }

        // Try flush the rearrange buf
        loop {
            let mut del_idx = -1;
            for (idx, m) in self.rearrange_buf.iter().enumerate() {
                if m.seq == self.next_seq {
                    del_idx = idx as i32;
                }
            }

            if del_idx >= 0 {
                out.push(self.rearrange_buf.remove(del_idx as usize));
                self.next_seq += 1;
            } else {
                break;
            }
        }

        out
    }
}

async fn run() {
    // Create audio stream on main thread
    let host = cpal::default_host();
    let device = host.default_output_device().unwrap();
    let decoded_audio = Arc::new(Mutex::new(vec![]));
    let decoded_audio_cb = decoded_audio.clone();

    let stream = device
        .build_output_stream(
            &StreamConfig {
                sample_rate: SampleRate(48000),
                channels: 2,
                buffer_size: cpal::BufferSize::Default,
            },
            move |data: &mut [f32], &_| {
                if decoded_audio_cb.lock().unwrap().len() >= data.len() {
                    data.copy_from_slice(&decoded_audio_cb.lock().unwrap()[0..data.len()]);
                    decoded_audio_cb.lock().unwrap().drain(0..data.len());
                } else {
                    data.fill(Sample::EQUILIBRIUM);
                }
            },
            move |err| {
                panic!("{}", err);
            },
            None,
        )
        .unwrap();

    stream.play().unwrap();

    // Create window
    let event_loop = EventLoop::new().unwrap();
    let window = WindowBuilder::new().build(&event_loop).unwrap();
    window.set_resizable(false);
    window.set_cursor_visible(false);

    let _ = window.request_inner_size(Size::Physical(PhysicalSize {
        width: 1920,
        height: 1080,
    }));

    // Pray that the window changes size
    sleep(Duration::from_millis(100)).await;

    let mut c = init(&window, decoded_audio).await;

    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).unwrap();

    #[cfg(not(target_os = "macos"))]
    socket.set_recv_buffer_size(8 << 20).unwrap();

    let sock_addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
    socket.bind(&sock_addr.into()).unwrap();

    let sock: UdpSocket = socket.into();
    sock.connect("dw.superkooks.com:42069").unwrap();
    sock.send(&vec![1]).unwrap();
    sock.recv(&mut vec![0]).unwrap();

    let mut tcp_sock = TcpStream::connect("dw.superkooks.com:42069").unwrap();
    tcp_sock.set_nonblocking(true).unwrap();
    tcp_sock.set_nodelay(true).unwrap();

    let mut video_stream = UdpStream::new();
    // let mut video_stream = TcpStream::connect("localhost:9999").unwrap();

    let mut last_poll = Instant::now();

    // Run the windows event loop
    let mut t = Instant::now();
    event_loop
        .run(move |event, control_flow| match event {
            Event::WindowEvent {
                window_id,
                ref event,
            } => {
                if window_id == window.id() {
                    match event {
                        WindowEvent::CloseRequested => {
                            control_flow.exit();
                        }
                        WindowEvent::Resized(_) => {}
                        WindowEvent::KeyboardInput {
                            device_id: _,
                            event,
                            is_synthetic: _,
                        } => {
                            println!("got keyboard event {:?}", event.physical_key);
                            let key_text = event.logical_key.to_text();
                            match key_text {
                                Some(t) => {
                                    tcp_sock
                                        .write(
                                            &rmp_serde::to_vec(&KeyEvent::Key {
                                                letter: t.chars().nth(0).unwrap(),
                                                state: match event.state {
                                                    winit::event::ElementState::Pressed => true,
                                                    winit::event::ElementState::Released => false,
                                                },
                                            })
                                            .unwrap(),
                                        )
                                        .unwrap();
                                }
                                None => {}
                            }
                        }
                        WindowEvent::CursorMoved {
                            device_id: _,
                            position,
                        } => {
                            let size = window.inner_size();

                            // Send the delta position
                            println!(
                                "last mouse {} us ago",
                                Instant::now().duration_since(t).as_micros()
                            );
                            t = Instant::now();
                            tcp_sock
                                .write(
                                    &rmp_serde::to_vec(&KeyEvent::Mouse {
                                        x: position.x - size.width as f64 / 2.,
                                        y: position.y - size.height as f64 / 2.,
                                    })
                                    .unwrap(),
                                )
                                .unwrap();

                            // Reset the position of the mouse to the centre
                            window
                                .set_cursor_position(PhysicalPosition::new(
                                    size.width / 2,
                                    size.height / 2,
                                ))
                                .unwrap();
                        }
                        WindowEvent::MouseInput {
                            device_id: _,
                            state,
                            button,
                        } => {
                            let but = match button {
                                winit::event::MouseButton::Left => 0,
                                winit::event::MouseButton::Middle => 1,
                                winit::event::MouseButton::Right => 2,
                                _ => 3,
                            };
                            if but < 3 {
                                tcp_sock
                                    .write(
                                        &rmp_serde::to_vec(&KeyEvent::Click {
                                            button: but,
                                            state: state.is_pressed(),
                                        })
                                        .unwrap(),
                                    )
                                    .unwrap();
                            }
                        }
                        WindowEvent::RedrawRequested => {
                            println!(
                                "time since last poll {} us",
                                Instant::now().duration_since(last_poll).as_micros()
                            );
                            let mut buf = vec![0; 2048];
                            sock.recv(&mut buf).unwrap();
                            last_poll = Instant::now();

                            let msg: Msg = rmp_serde::from_slice(&buf).unwrap();
                            if msg.is_audio {
                                let mut output = vec![0f32; 1920 * 4];
                                c.audio_decoder
                                    .decode_float(
                                        Some(Packet::try_from(&msg.data).unwrap()),
                                        MutSignals::try_from(&mut output).unwrap(),
                                        false,
                                    )
                                    .unwrap();
                                c.decoded_audio.lock().unwrap().extend_from_slice(&output);
                            } else {
                                for msg in video_stream.recv(msg) {
                                    c.accumulate_nalus(&msg.data);
                                }
                            }
                            // let mut b = vec![0; 8192];
                            // let n = video_stream.read(&mut b).unwrap();
                            // c.accumulate_nal(&b);
                        }
                        _ => {}
                    }
                }
            }
            Event::AboutToWait => {
                window.request_redraw();
            }
            _ => {}
        })
        .unwrap();
}

fn main() {
    pollster::block_on(run());
}
