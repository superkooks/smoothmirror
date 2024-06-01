use std::{io::Read, net::UdpSocket, time::Instant};

use async_winit::{
    dpi::{PhysicalSize, Size},
    event_loop::EventLoop,
    window::Window,
    ThreadUnsafe,
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

use crate::Msg;

struct Accumulator(Vec<Vec<u8>>);
impl AccumulatedNalHandler for Accumulator {
    fn nal(&mut self, nal: RefNal<'_>) -> NalInterest {
        if !nal.is_complete() {
            return NalInterest::Buffer;
        }

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
}

async fn init(window: &Window<ThreadUnsafe>) -> Client {
    // Init graphics
    let size = window.inner_size().await;

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

    // Create decoder
    ffmpeg_next::init().unwrap();

    let codec = decoder::find(codec::Id::H264).unwrap();
    let decoder = codec::Context::new_with_codec(codec)
        .decoder()
        .video()
        .unwrap();

    let scaler = scaling::Context::get(
        Pixel::YUV420P,
        2560,
        1440,
        Pixel::BGRA,
        2560,
        1440,
        Flags::empty(),
    )
    .unwrap();

    Client {
        queue,
        decoder,
        surface,
        scaler,
        annexb: AnnexBReader::accumulate(Accumulator(vec![])),
    }
}

impl Client {
    async fn consume_nal(&mut self, nal: &[u8]) {
        let mut t = Instant::now();
        let res = self.decoder.send_packet(&ffmpeg_next::Packet::copy(nal));
        println!(
            "took {} us to decode",
            Instant::now().duration_since(t).as_micros()
        );
        t = Instant::now();

        let mut frame = Video::empty();
        if res.is_ok() && self.decoder.receive_frame(&mut frame).is_ok() {
            let mut rgb_frame = Video::empty();
            self.scaler.run(&frame, &mut rgb_frame).unwrap();
            println!(
                "took {} us to convert",
                Instant::now().duration_since(t).as_micros()
            );

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
                    bytes_per_row: Some(4 * 2560),
                    rows_per_image: Some(1440),
                },
                wgpu::Extent3d {
                    width: 2560,
                    height: 1440,
                    depth_or_array_layers: 1,
                },
            );

            self.queue.submit(std::iter::empty());
            output.present();

            println!("presenting")
        }
    }

    async fn accumulate_nal(&mut self, msg: Msg) {
        self.annexb.push(&msg.data);

        loop {
            if self.annexb.nal_handler_ref().0.len() == 0 {
                break;
            }

            let nalu = self.annexb.nal_handler_mut().0.remove(0);
            self.consume_nal(&nalu).await;
        }
    }
}

pub async fn start_client() {
    let evl: EventLoop<ThreadUnsafe> = EventLoop::new();
    let target = evl.window_target().clone();
    evl.block_on(async move {
        target.resumed().await;
        let window = Window::new().await.unwrap();
        window
            .set_inner_size(Size::Physical(PhysicalSize {
                width: 2560,
                height: 1440,
            }))
            .await;

        let mut c = init(&window).await;

        let sock = UdpSocket::bind("localhost:0").unwrap();
        sock.connect("localhost:42069").unwrap();

        sock.send(&vec![1u8]).unwrap();

        let mut next_seq = 0;
        let mut rearrange_buf = vec![];
        let mut last_in_seq = Instant::now();

        loop {
            let mut buf = vec![0; 2048];
            println!("waiting packet");
            sock.recv(&mut buf).unwrap();
            println!("definitely have packet");

            if Instant::now().duration_since(last_in_seq).as_millis() > 100 {
                next_seq += 1;
            }

            let msg: Msg = rmp_serde::from_slice(&buf).unwrap();
            println!("got packet? {} (waiting for {})", msg.seq, next_seq);
            if msg.seq != next_seq {
                // Add it to the rearrange buf
                rearrange_buf.push(msg);
                println!("storing packet in rearrange buf")
            } else {
                // Write it
                c.accumulate_nal(msg).await;
                next_seq += 1;
                last_in_seq = Instant::now();

                // Try flush the rearrange buf
                println!("attempting to flush");
                loop {
                    let mut del_idx = -1;
                    for (idx, m) in rearrange_buf.iter().enumerate() {
                        if m.seq == next_seq {
                            del_idx = idx as i32;
                        }
                    }

                    if del_idx >= 0 {
                        println!("flushing rearrange buf");
                        c.accumulate_nal(rearrange_buf.remove(del_idx as usize))
                            .await;
                        next_seq += 1;
                    } else {
                        break;
                    }
                }
                println!("flush done");
            }
        }
    });
}
