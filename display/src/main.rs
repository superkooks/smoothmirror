use std::{
    io::Write,
    net::TcpStream,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use client::init_client;
use common::chan;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Sample, SampleRate, StreamConfig,
};
use egui_glium::{egui_winit::egui::ViewportId, EguiGlium};
use glium::{
    backend::winit::{
        event::WindowEvent,
        event_loop::{ControlFlow, EventLoop},
        window::Window,
    },
    glutin::surface::WindowSurface,
    index::NoIndices,
    texture::RawImage2d,
    uniform,
    vertex::VerticesSource,
    winit::{
        application::ApplicationHandler,
        event::{DeviceEvent, ElementState, MouseButton},
        keyboard::KeyCode,
        window::CursorGrabMode,
    },
    Display, Surface,
};
use serde::{Deserialize, Serialize};
use ui::Ui;

mod client;
mod ui;

const ENCODED_WIDTH: u32 = 2560;
const ENCODED_HEIGHT: u32 = 1440;
const FRAME_DURATION: Duration = Duration::from_micros(16_666);

// If you are experiencing packet loss on linux, you may need to increase you udp buffer size
// sudo sysctl -w net.core.rmem_max=20000000

#[derive(Serialize, Deserialize)]
enum KeyEvent {
    Key { letter: char, state: bool },
    Mouse { x: f64, y: f64 },
    Click { button: i32, state: bool },
}

struct AppDisplay {
    key_chan: chan::SubChan,
    window: Window,
    display: Display<WindowSurface>,

    texture: glium::Texture2d,
    program: glium::Program,

    tredraw: Instant,
    ui: Ui,
}

impl AppDisplay {
    fn new(
        window: Window,
        display: Display<WindowSurface>,
        key_chan: chan::SubChan,
        egui_glium: EguiGlium,
        volume: Arc<Mutex<f32>>,
    ) -> Self {
        let texture = glium::Texture2d::empty(&display, ENCODED_WIDTH, ENCODED_HEIGHT).unwrap();

        let program = glium::Program::from_source(
            &display,
            include_str!("simple.vert"),
            include_str!("simple.frag"),
            None,
        )
        .unwrap();

        AppDisplay {
            window,
            display,
            key_chan,

            texture,
            program,

            tredraw: Instant::now(),
            ui: Ui {
                egui_glium,
                open: true,
                volume,
                quit: false,
            },
        }
    }
}

impl ApplicationHandler<Vec<u8>> for AppDisplay {
    fn resumed(&mut self, _event_loop: &glium::winit::event_loop::ActiveEventLoop) {}

    fn user_event(
        &mut self,
        _event_loop: &glium::winit::event_loop::ActiveEventLoop,
        event: Vec<u8>,
    ) {
        // Write image to texture
        let t = Instant::now();
        self.texture = glium::Texture2d::with_mipmaps(
            &self.display,
            RawImage2d::from_raw_rgba(event, (ENCODED_WIDTH, ENCODED_HEIGHT)),
            glium::texture::MipmapsOption::NoMipmap,
        )
        .unwrap();
        println!(
            "!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!  written image to texture after {} us",
            Instant::now().duration_since(t).as_micros()
        );
    }

    fn about_to_wait(&mut self, _event_loop: &glium::winit::event_loop::ActiveEventLoop) {
        if Instant::now().duration_since(self.tredraw) > FRAME_DURATION {
            self.window.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &glium::winit::event_loop::ActiveEventLoop,
        window_id: glium::winit::window::WindowId,
        event: WindowEvent,
    ) {
        if window_id == self.window.id() {
            match event {
                WindowEvent::CloseRequested => {
                    event_loop.exit();
                }
                WindowEvent::Resized(_) => {}
                WindowEvent::KeyboardInput {
                    device_id: _,
                    event: ref kevent,
                    is_synthetic: _,
                } => {
                    println!("got keyboard event {:?}", kevent.physical_key);
                    match kevent.physical_key {
                        glium::winit::keyboard::PhysicalKey::Code(KeyCode::F7) => {
                            if kevent.state != ElementState::Released {
                                return;
                            }

                            self.ui.open = !self.ui.open;

                            // Lock and hide the cursor, or inverse
                            self.window.set_cursor_visible(self.ui.open);
                            if self.ui.open {
                                self.window.set_cursor_grab(CursorGrabMode::None).unwrap();
                            } else {
                                self.window
                                    .set_cursor_grab(CursorGrabMode::Confined)
                                    .or_else(|_e| {
                                        self.window.set_cursor_grab(CursorGrabMode::Locked)
                                    })
                                    .unwrap();
                            }

                            return;
                        }
                        _ => {}
                    }

                    if self.ui.open {
                        let _ = self.ui.egui_glium.on_event(&self.window, &event);
                        return;
                    }

                    let key_text = kevent.logical_key.to_text();
                    match key_text {
                        Some(t) => {
                            self.key_chan
                                .write_all(
                                    &rmp_serde::to_vec(&KeyEvent::Key {
                                        letter: t.chars().nth(0).unwrap(),
                                        state: match kevent.state {
                                            ElementState::Pressed => true,
                                            ElementState::Released => false,
                                        },
                                    })
                                    .unwrap(),
                                )
                                .unwrap();
                        }
                        None => {}
                    }
                }
                WindowEvent::MouseInput {
                    device_id: _,
                    state,
                    button,
                } => {
                    if self.ui.open {
                        let _ = self.ui.egui_glium.on_event(&self.window, &event);
                        return;
                    }

                    let but = match button {
                        MouseButton::Left => 0,
                        MouseButton::Middle => 1,
                        MouseButton::Right => 2,
                        _ => 3,
                    };
                    if but < 3 {
                        self.key_chan
                            .write_all(
                                &rmp_serde::to_vec(&KeyEvent::Click {
                                    button: but,
                                    state: state.is_pressed(),
                                })
                                .unwrap(),
                            )
                            .unwrap();
                    }
                }
                WindowEvent::CursorMoved {
                    device_id: _,
                    position: _,
                } => {
                    if self.ui.open {
                        let _ = self.ui.egui_glium.on_event(&self.window, &event);
                    }
                }
                WindowEvent::RedrawRequested => {
                    if self.ui.quit {
                        event_loop.exit();
                        return;
                    }

                    println!(
                        "redrawing after {} us",
                        Instant::now().duration_since(self.tredraw).as_micros()
                    );
                    self.tredraw = Instant::now();

                    println!("*****************************  redrawing");

                    let mut target = self.display.draw();
                    target
                        .draw(
                            VerticesSource::Marker {
                                len: 3,
                                per_instance: false,
                            },
                            NoIndices(glium::index::PrimitiveType::TrianglesList),
                            &self.program,
                            &uniform! {frag_tex: self.texture.sampled()},
                            &glium::DrawParameters::default(),
                        )
                        .unwrap();

                    self.ui.redraw(&self.window, &self.display, &mut target);

                    target.finish().unwrap();
                }
                _ => {}
            }
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &glium::winit::event_loop::ActiveEventLoop,
        _device_id: glium::winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        match event {
            DeviceEvent::MouseMotion { delta } => {
                if !self.ui.open {
                    // Send the delta position
                    self.key_chan
                        .write_all(
                            &rmp_serde::to_vec(&KeyEvent::Mouse {
                                x: delta.0,
                                y: delta.1,
                            })
                            .unwrap(),
                        )
                        .unwrap();
                }
            }
            _ => {}
        }
    }
}

fn main() {
    // Create audio stream on main thread
    let host = cpal::default_host();
    let device = host.default_output_device().unwrap();
    let decoded_audio = Arc::new(Mutex::new(vec![]));
    let decoded_audio_cb = decoded_audio.clone();

    let volume = Arc::new(Mutex::new(100.0f32));
    let volume_cb = volume.clone();

    let stream = device
        .build_output_stream(
            &StreamConfig {
                sample_rate: SampleRate(48000),
                channels: 2,
                buffer_size: cpal::BufferSize::Default,
            },
            move |data: &mut [f32], &_| {
                if decoded_audio_cb.lock().unwrap().len() >= data.len() {
                    let volume_guard = volume_cb.lock().unwrap();
                    println!("volume {}", *volume_guard / 100.);
                    let decoded: Vec<f32> = decoded_audio_cb.lock().unwrap()[0..data.len()]
                        .iter()
                        .map(|x| x * (*volume_guard) / 100.)
                        .collect();

                    decoded_audio_cb.lock().unwrap().drain(0..data.len());
                    data.copy_from_slice(&decoded);
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
    let event_loop = EventLoop::<Vec<u8>>::with_user_event().build().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    let (window, display) = glium::backend::glutin::SimpleWindowBuilder::new()
        .with_inner_size(1920, 1080)
        .build(&event_loop);

    window.set_resizable(false);

    let egui_glium = egui_glium::EguiGlium::new(ViewportId::ROOT, &display, &window, &event_loop);

    let mut c = init_client(decoded_audio, event_loop.create_proxy());

    let tcp_sock = TcpStream::connect("dw.superkooks.com:42069").unwrap();
    let mut master_chan = chan::TcpChan::new(tcp_sock);

    // Create thread to read udp and decode frames
    thread::spawn(move || {
        c.init();
        c.run()
    });

    // Create instance to display frames and capture events
    let mut d = AppDisplay::new(
        window,
        display,
        master_chan.create_subchan(chan::ChannelId::Keys),
        egui_glium,
        volume,
    );

    // Start networking channel to forward key events back to capture client
    master_chan.start_rw();

    // Run its event loop
    event_loop.run_app(&mut d).unwrap();
}
