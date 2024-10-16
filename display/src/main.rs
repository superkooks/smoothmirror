use std::{
    io::Write,
    net::TcpStream,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use client::init_client;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Sample, SampleRate, StreamConfig,
};
use gilrs::Gilrs;
use glium::{
    backend::winit::{
        dpi::PhysicalPosition,
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
        event::{ElementState, MouseButton},
    },
    Display, Surface,
};
use serde::{Deserialize, Serialize};

mod client;

const ENCODED_WIDTH: u32 = 2560;
const ENCODED_HEIGHT: u32 = 1440;
const FRAME_DURATION: Duration = Duration::from_micros(16_666);

#[derive(Serialize, Deserialize)]
enum KeyEvent {
    Key { letter: char, state: bool },
    Mouse { x: f64, y: f64 },
    Click { button: i32, state: bool },
    GamepadButton { button: gilrs::Button, state: u8 },
    GamepadAxis { axis: gilrs::Axis, state: f32 },
}

struct AppDisplay {
    tcp_sock: TcpStream,
    window: Window,
    display: Display<WindowSurface>,

    texture: glium::Texture2d,
    program: glium::Program,

    tredraw: Instant,
}

impl AppDisplay {
    fn new(window: Window, display: Display<WindowSurface>, tcp_sock: TcpStream) -> Self {
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
            tcp_sock,

            texture,
            program,

            tredraw: Instant::now(),
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
                    event,
                    is_synthetic: _,
                } => {
                    println!("got keyboard event {:?}", event.physical_key);
                    let key_text = event.logical_key.to_text();
                    match key_text {
                        Some(t) => {
                            self.tcp_sock
                                .write(
                                    &rmp_serde::to_vec(&KeyEvent::Key {
                                        letter: t.chars().nth(0).unwrap(),
                                        state: match event.state {
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
                WindowEvent::CursorMoved {
                    device_id: _,
                    position,
                } => {
                    let size = self.window.inner_size();

                    // Send the delta position
                    self.tcp_sock
                        .write(
                            &rmp_serde::to_vec(&KeyEvent::Mouse {
                                x: position.x - size.width as f64 / 2.,
                                y: position.y - size.height as f64 / 2.,
                            })
                            .unwrap(),
                        )
                        .unwrap();

                    // Reset the position of the mouse to the centre
                    self.window
                        .set_cursor_position(PhysicalPosition::new(size.width / 2, size.height / 2))
                        .unwrap();
                }
                WindowEvent::MouseInput {
                    device_id: _,
                    state,
                    button,
                } => {
                    let but = match button {
                        MouseButton::Left => 0,
                        MouseButton::Middle => 1,
                        MouseButton::Right => 2,
                        _ => 3,
                    };
                    if but < 3 {
                        self.tcp_sock
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
                    target.finish().unwrap();
                }
                _ => {}
            }
        }
    }
}

fn main() {
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
    let event_loop = EventLoop::<Vec<u8>>::with_user_event().build().unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    let (window, display) = glium::backend::glutin::SimpleWindowBuilder::new()
        .with_inner_size(1920, 1080)
        .build(&event_loop);

    window.set_resizable(false);
    window.set_cursor_visible(false);

    let mut c = init_client(decoded_audio, event_loop.create_proxy());

    let tcp_sock = TcpStream::connect("dw.superkooks.com:42069").unwrap();
    let cts = tcp_sock.try_clone().unwrap();
    let mut gts = tcp_sock.try_clone().unwrap();
    thread::spawn(move || {
        c.init();
        c.run(cts)
    });

    thread::spawn(move || {
        let mut gilrs = Gilrs::new().unwrap();
        loop {
            while let Some(ev) = gilrs.next_event() {
                match ev.event {
                    gilrs::EventType::ButtonPressed(button, _code) => {
                        gts.write(
                            &rmp_serde::to_vec(&KeyEvent::GamepadButton { button, state: 1 })
                                .unwrap(),
                        )
                        .unwrap();
                    }
                    gilrs::EventType::ButtonReleased(button, _code) => {
                        gts.write(
                            &rmp_serde::to_vec(&KeyEvent::GamepadButton { button, state: 0 })
                                .unwrap(),
                        )
                        .unwrap();
                    }
                    gilrs::EventType::ButtonChanged(button, state, _code) => {
                        let axis = match button {
                            gilrs::Button::LeftTrigger2 => gilrs::Axis::LeftZ,
                            gilrs::Button::RightTrigger2 => gilrs::Axis::RightZ,
                            _ => gilrs::Axis::Unknown,
                        };
                        gts.write(
                            &rmp_serde::to_vec(&KeyEvent::GamepadAxis { axis, state }).unwrap(),
                        )
                        .unwrap();
                    }
                    gilrs::EventType::AxisChanged(axis, state, _code) => {
                        gts.write(
                            &rmp_serde::to_vec(&KeyEvent::GamepadAxis { axis, state }).unwrap(),
                        )
                        .unwrap();
                    }
                    _ => {}
                };
                // println!("{:?} New event from {}: {:?}", time, id, event);
            }
        }
    });

    let mut d = AppDisplay::new(window, display, tcp_sock);

    // let mut video_stream = TcpStream::connect("localhost:9999").unwrap();

    // Run the event loop
    event_loop.run_app(&mut d).unwrap();
}
