use std::{cell::RefCell, ops::Deref, rc::Rc};

use pulse::{def::BufferAttr, mainloop::standard::IterateResult, stream::PeekResult};

// Capture audio with pulseaudio
// pactl load-module module-null-sink sink_name=sink1
// Use qpwgraph to connect applications

pub struct AudioCapturer {
    stream: Rc<RefCell<pulse::stream::Stream>>,
    mainloop: Rc<RefCell<pulse::mainloop::standard::Mainloop>>,
    _ctx: Rc<RefCell<pulse::context::Context>>,
}

impl AudioCapturer {
    pub fn new() -> Self {
        // Create a spec for our input
        let spec = pulse::sample::Spec {
            format: pulse::sample::Format::F32le,
            channels: 2,
            rate: 48000,
        };
        assert!(spec.is_valid());

        // Create a mainloop and context
        let mainloop = Rc::new(RefCell::new(
            pulse::mainloop::standard::Mainloop::new().unwrap(),
        ));
        let ctx = Rc::new(RefCell::new(
            pulse::context::Context::new(mainloop.borrow().deref(), "prospectivegopher").unwrap(),
        ));

        // Connect to pulseaudio
        ctx.borrow_mut()
            .connect(None, pulse::context::FlagSet::empty(), None)
            .unwrap();
        wait_until_ctx_ready(&mainloop, &ctx);

        // Create a stream from the desired source
        let stream = Rc::new(RefCell::new(
            pulse::stream::Stream::new(&mut ctx.borrow_mut(), "desktop audio", &spec, None)
                .unwrap(),
        ));
        stream
            .borrow_mut()
            .connect_record(
                Some("sink1.monitor"),
                Some(&BufferAttr {
                    maxlength: 7680 * 8,
                    tlength: u32::MAX,
                    prebuf: u32::MAX,
                    minreq: u32::MAX,
                    fragsize: 7680 * 4, // encode in 20ms blocks
                }),
                pulse::stream::FlagSet::START_CORKED | pulse::stream::FlagSet::ADJUST_LATENCY,
            )
            .unwrap();
        wait_until_stream_ready(&mainloop, &stream);

        Self {
            stream,
            mainloop,
            _ctx: ctx,
        }
    }

    pub fn capture_audio(&mut self) -> &[f32] {
        // Iterate the mainloop
        match self.mainloop.borrow_mut().iterate(false) {
            IterateResult::Quit(_) | IterateResult::Err(_) => {
                eprintln!("failed to iterate pulseaudio mainloop while capturing audio");
                return &[];
            }
            IterateResult::Success(_) => {}
        }

        // Peek the stream to see if there is any data
        let peek_res = self.stream.borrow_mut().peek().unwrap();
        match peek_res {
            PeekResult::Data(data) => {
                self.stream.borrow_mut().discard().unwrap();

                // Align bytes to f32
                let (prefix, floats, suffix) = unsafe { data.align_to::<f32>() };
                assert!(prefix.len() == 0 && suffix.len() == 0);

                // Return the data
                return floats;
            }
            PeekResult::Empty => {}
            PeekResult::Hole(_) => self.stream.borrow_mut().discard().unwrap(),
        };

        &[]
    }

    pub fn uncork(&self) {
        self.stream.borrow_mut().uncork(None);
    }
}

fn wait_until_ctx_ready(
    mainloop: &Rc<RefCell<pulse::mainloop::standard::Mainloop>>,
    ctx: &Rc<RefCell<pulse::context::Context>>,
) {
    loop {
        match mainloop.borrow_mut().iterate(false) {
            IterateResult::Quit(_) | IterateResult::Err(_) => {
                panic!("failed to iterate pulseaudio mainloop")
            }
            IterateResult::Success(_) => {}
        }
        match ctx.borrow().get_state() {
            pulse::context::State::Ready => break,
            pulse::context::State::Failed | pulse::context::State::Terminated => {
                panic!("pulseaudio context is failed")
            }
            _ => {}
        }
    }
}

fn wait_until_stream_ready(
    mainloop: &Rc<RefCell<pulse::mainloop::standard::Mainloop>>,
    stream: &Rc<RefCell<pulse::stream::Stream>>,
) {
    loop {
        match mainloop.borrow_mut().iterate(false) {
            IterateResult::Quit(_) | IterateResult::Err(_) => {
                panic!("failed to iterate pulseaudio mainloop")
            }
            IterateResult::Success(_) => {}
        }
        match stream.borrow().get_state() {
            pulse::stream::State::Ready => break,
            pulse::stream::State::Failed | pulse::stream::State::Terminated => {
                panic!("pulseaudio stream is failed")
            }
            _ => {}
        }
    }
}
