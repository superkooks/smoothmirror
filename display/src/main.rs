use std::{
    io::{Read, Write},
    net::{TcpStream, UdpSocket},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_std::task::sleep;
use audiopus::{packet::Packet, MutSignals};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Sample, SampleRate, StreamConfig,
};
use ffmpeg_next::{
    codec, decoder,
    format::Pixel,
    frame::Video,
    software::scaling::{self, Flags},
};
use h264_reader::{
    annexb::AnnexBReader,
    nal::{Nal, RefNal},
    push::{AccumulatedNalHandler, NalAccumulator, NalInterest},
};
use serde::{Deserialize, Serialize};
use winit::{
    dpi::{PhysicalSize, Size},
    event::{Event, WindowEvent},
    event_loop::EventLoop,
    window::{Window, WindowBuilder},
};

const ENCODED_WIDTH: u32 = 1920;
const ENCODED_HEIGHT: u32 = 1080;
const FRAME_DURATION: Duration = Duration::from_micros(16_666);

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

struct Accumulator(Vec<Vec<u8>>);
impl AccumulatedNalHandler for Accumulator {
    fn nal(&mut self, nal: RefNal<'_>) -> NalInterest {
        if !nal.is_complete() {
            return NalInterest::Buffer;
        }

        // println!("have complete nal");
        let mut nal_data = vec![0, 0, 0, 1];
        nal.reader().read_to_end(&mut nal_data).unwrap();
        self.0.push(nal_data);
        return NalInterest::Buffer;
    }
}

struct Client {
    queue: wgpu::Queue,
    decoder: ffmpeg_next::decoder::Video,
    surface: wgpu::Surface,
    scaler: scaling::Context,
    annexb: h264_reader::annexb::AnnexBReader<NalAccumulator<Accumulator>>,

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

    // Create decoder
    ffmpeg_next::init().unwrap();

    let codec = decoder::find(codec::Id::H264).unwrap();
    let decoder = codec::Context::new_with_codec(codec)
        .decoder()
        .video()
        .unwrap();

    let scaler = scaling::Context::get(
        Pixel::YUV420P,
        ENCODED_WIDTH,
        ENCODED_HEIGHT,
        Pixel::BGRA,
        ENCODED_WIDTH,
        ENCODED_HEIGHT,
        Flags::empty(),
    )
    .unwrap();

    // Create audio stream

    Client {
        queue,
        decoder,
        surface,
        scaler,
        annexb: AnnexBReader::accumulate(Accumulator(vec![])),
        audio_decoder: audiopus::coder::Decoder::new(
            audiopus::SampleRate::Hz48000,
            audiopus::Channels::Stereo,
        )
        .unwrap(),
        decoded_audio,
    }
}

impl Client {
    fn consume_nal(&mut self, nal: &[u8]) {
        // let mut t = Instant::now();
        let res = self.decoder.send_packet(&ffmpeg_next::Packet::copy(nal));
        // println!(
        //     "took {} us to decode",
        //     Instant::now().duration_since(t).as_micros()
        // );
        // t = Instant::now();

        let mut frame = Video::empty();
        if res.is_ok() && self.decoder.receive_frame(&mut frame).is_ok() {
            let mut rgb_frame = Video::empty();
            self.scaler.run(&frame, &mut rgb_frame).unwrap();
            // println!(
            //     "took {} us to convert",
            //     Instant::now().duration_since(t).as_micros()
            // );

            let output = self.surface.get_current_texture().unwrap();

            self.queue.write_texture(
                wgpu::ImageCopyTextureBase {
                    texture: &output.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &rgb_frame.data(0),
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * ENCODED_WIDTH),
                    rows_per_image: Some(ENCODED_HEIGHT),
                },
                wgpu::Extent3d {
                    width: ENCODED_WIDTH,
                    height: ENCODED_HEIGHT,
                    depth_or_array_layers: 1,
                },
            );

            self.queue.submit(std::iter::empty());
            output.present();

            // println!("presenting")
        }
    }

    fn accumulate_nal(&mut self, msg: Msg) {
        // println!("accumulating nals");
        self.annexb.push(&msg.data);

        loop {
            if self.annexb.nal_handler_ref().0.len() == 0 {
                break;
            }

            // println!("about to consume nal");
            let nalu = self.annexb.nal_handler_mut().0.remove(0);
            self.consume_nal(&nalu);
        }
    }
}

struct UdpStream {
    next_seq: u64,
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

    let _ = window.request_inner_size(Size::Physical(PhysicalSize {
        width: ENCODED_WIDTH,
        height: ENCODED_HEIGHT,
    }));

    // Pray that the window changes size
    sleep(Duration::from_millis(100)).await;

    let mut c = init(&window, decoded_audio).await;

    let sock = UdpSocket::bind("0.0.0.0:0").unwrap();
    sock.connect("10.8.0.1:42069").unwrap();

    sock.send(&vec![1]).unwrap();

    let mut buf = vec![0; 2048];
    let recv_bytes = sock.recv(&mut buf).unwrap();
    sock.connect(std::str::from_utf8(&buf[..recv_bytes]).unwrap())
        .unwrap();
    let mut tcp_sock = TcpStream::connect("10.8.0.1:42069").unwrap();

    let mut video_stream = UdpStream::new();

    // Run the windows event loop
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
                            // println!("got keyboard event {:?}", event.physical_key);
                            let key_text = event.logical_key.to_text();
                            match key_text {
                                Some(t) => {
                                    tcp_sock
                                        .write(
                                            &rmp_serde::to_vec(&KeyEvent {
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
                        WindowEvent::RedrawRequested => {
                            let mut buf = vec![0; 2048];
                            sock.recv(&mut buf).unwrap();

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
                                    c.accumulate_nal(msg);
                                }
                            }
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
