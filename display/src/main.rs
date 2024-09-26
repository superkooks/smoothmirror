use std::{
    io::Write,
    net::TcpStream,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use async_std::task::sleep;
use client::init_client;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Sample, SampleRate, StreamConfig,
};
use serde::{Deserialize, Serialize};
use winit::{
    dpi::{PhysicalPosition, PhysicalSize, Size},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::{Window, WindowBuilder},
};

mod client;

const ENCODED_WIDTH: u32 = 2560;
const ENCODED_HEIGHT: u32 = 1440;
const FRAME_DURATION: Duration = Duration::from_micros(16_666);

#[derive(Serialize, Deserialize)]
enum KeyEvent {
    Key { letter: char, state: bool },
    Mouse { x: f64, y: f64 },
    Click { button: i32, state: bool },
}

struct Display {
    queue: wgpu::Queue,
    surface: wgpu::Surface,
}

async fn init_display(window: &Window) -> Display {
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

    Display { queue, surface }
}

impl Display {
    fn display_frame(&mut self, frame: &[u8]) {
        if frame.len() == 0 {
            return;
        }

        let output = self.surface.get_current_texture().unwrap();

        self.queue.write_texture(
            wgpu::ImageCopyTextureBase {
                texture: &output.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            frame,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * output.texture.width()),
                rows_per_image: Some(output.texture.height()),
            },
            wgpu::Extent3d {
                width: output.texture.width(),
                height: output.texture.height(),
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit(std::iter::empty());
        output.present();
    }
}

async fn run() {
    // Create audio stream on main thread
    let host = cpal::default_host();
    let device = host.default_output_device().unwrap();
    let decoded_audio = Arc::new(Mutex::new(vec![]));
    let decoded_audio_cb = decoded_audio.clone();

    // let stream = device
    //     .build_output_stream(
    //         &StreamConfig {
    //             sample_rate: SampleRate(48000),
    //             channels: 2,
    //             buffer_size: cpal::BufferSize::Default,
    //         },
    //         move |data: &mut [f32], &_| {
    //             if decoded_audio_cb.lock().unwrap().len() >= data.len() {
    //                 data.copy_from_slice(&decoded_audio_cb.lock().unwrap()[0..data.len()]);
    //                 decoded_audio_cb.lock().unwrap().drain(0..data.len());
    //             } else {
    //                 data.fill(Sample::EQUILIBRIUM);
    //             }
    //         },
    //         move |err| {
    //             panic!("{}", err);
    //         },
    //         None,
    //     )
    //     .unwrap();

    // stream.play().unwrap();

    // Create window
    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    let window = WindowBuilder::new().build(&event_loop).unwrap();
    window.set_resizable(false);
    window.set_cursor_visible(false);

    let _ = window.request_inner_size(Size::Physical(PhysicalSize {
        width: 1920,
        height: 1080,
    }));

    // Pray that the window changes size
    sleep(Duration::from_millis(100)).await;

    let image = Arc::new(Mutex::new(vec![]));
    let mut d = init_display(&window).await;
    let mut c = init_client(window.inner_size(), decoded_audio, image.clone());

    let mut tcp_sock = TcpStream::connect("dw.superkooks.com:42069").unwrap();
    let ts = tcp_sock.try_clone().unwrap();
    thread::spawn(move || {
        c.init();
        c.run(ts)
    });

    // let mut video_stream = TcpStream::connect("localhost:9999").unwrap();

    // Run the windows event loop
    let mut t = Instant::now();
    let mut tredraw = Instant::now();
    event_loop
        .run(move |event, control_flow| {
            if Instant::now().duration_since(tredraw) > FRAME_DURATION {
                window.request_redraw();
            }
            match event {
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
                                                        winit::event::ElementState::Released => {
                                                            false
                                                        }
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
                                    "redrawing after {} us",
                                    Instant::now().duration_since(tredraw).as_micros()
                                );
                                tredraw = Instant::now();
                                d.display_frame(&image.lock().unwrap());
                            }
                            _ => {}
                        }
                    }
                }
                Event::AboutToWait => {
                    // window.request_redraw();
                }
                _ => {}
            }
        })
        .unwrap();
}

fn main() {
    pollster::block_on(run());
}
