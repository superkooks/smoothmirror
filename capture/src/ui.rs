use std::{
    collections::{HashMap, VecDeque},
    io::{self, stdout},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::Instant,
};

use log::{info, Level};

use ratatui::{
    crossterm::{
        event::{self, Event, KeyCode},
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
        ExecutableCommand,
    },
    layout::{Constraint, Direction, Layout},
    prelude::CrosstermBackend,
    widgets::{Block, Paragraph},
    Frame, Terminal,
};

#[derive(Clone)]
pub struct FrameLatencyInfo {
    start: Instant,
    measurements: Vec<(String, u128)>,
}

impl FrameLatencyInfo {
    pub fn new() -> Self {
        FrameLatencyInfo {
            start: Instant::now(),
            measurements: Vec::new(),
        }
    }

    pub fn measure<T: Into<String>>(&mut self, s: T) {
        self.measurements.push((
            s.into(),
            Instant::now().duration_since(self.start).as_micros(),
        ));
    }

    pub fn total(&self) -> u128 {
        self.measurements.iter().map(|f| f.1).sum()
    }
}

#[derive(Clone)]
pub struct UI {
    infos: HashMap<String, VecDeque<FrameLatencyInfo>>,
    log: String,
}

pub fn start_ui() -> (Arc<Mutex<UI>>, JoinHandle<()>) {
    enable_raw_mode().unwrap();
    stdout().execute(EnterAlternateScreen).unwrap();

    let u = Arc::new(Mutex::new(UI {
        infos: HashMap::from([
            ("frame".into(), VecDeque::new()),
            ("packet".into(), VecDeque::new()),
            ("main_loop".into(), VecDeque::new()),
        ]),
        log: String::new(),
    }));

    let ui = u.clone();

    let hand = thread::spawn(move || {
        let mut term = Terminal::new(CrosstermBackend::new(stdout())).unwrap();
        loop {
            term.draw(|frame| {
                let mut u = { ui.lock().unwrap().clone() };
                u.draw(frame)
            })
            .unwrap();

            if event::poll(std::time::Duration::from_millis(10)).unwrap() {
                if ui.lock().unwrap().handle_events().unwrap() {
                    disable_raw_mode().unwrap();
                    stdout().execute(LeaveAlternateScreen).unwrap();
                    return;
                }
            }
        }
    });

    return (u, hand);
}

impl UI {
    pub fn add_frame_latency_info(&mut self, stream: &str, fli: FrameLatencyInfo) {
        self.infos.get_mut(stream).unwrap().push_back(fli);
        if self.infos[stream].len() > 400 {
            self.infos.get_mut(stream).unwrap().pop_front();
        }
    }

    fn handle_events(&mut self) -> io::Result<bool> {
        if let Event::Key(key) = event::read()? {
            if key.kind == event::KeyEventKind::Press && key.code == KeyCode::Char('q') {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn find_worst(&self, stream: &str) -> &FrameLatencyInfo {
        self.infos[stream].iter().max_by_key(|f| f.total()).unwrap()
    }

    fn draw(&mut self, frame: &mut Frame) {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![
                Constraint::Percentage(35),
                Constraint::Percentage(35),
                Constraint::Percentage(30),
            ])
            .split(frame.area());

        let last_streams = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![
                Constraint::Ratio(1, self.infos.len() as u32);
                self.infos.len()
            ])
            .split(layout[0]);

        let worst_streams = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![
                Constraint::Ratio(1, self.infos.len() as u32);
                self.infos.len()
            ])
            .split(layout[1]);

        let mut i = 0;
        for (stream, measurements) in &self.infos {
            if measurements.len() > 0 {
                frame.render_widget(
                    self.framelatency_widget(
                        measurements.back().unwrap(),
                        format!("Last {}", stream),
                    ),
                    last_streams[i],
                );

                frame.render_widget(
                    self.framelatency_widget(self.find_worst(stream), format!("Worst {}", stream)),
                    worst_streams[i],
                );
            }
            i += 1;
        }

        frame.render_widget(
            Paragraph::new(self.log.clone())
                .block(Block::bordered().title("Log"))
                .scroll((
                    (self.log.split('\n').count() as i32 - layout[2].height as i32).max(0) as u16,
                    0,
                )),
            layout[2],
        );
    }

    fn framelatency_widget(&self, l: &FrameLatencyInfo, title: String) -> Paragraph<'static> {
        let mut text = String::new();
        for measurement in &l.measurements {
            text += &format!("{} after {} us\n", measurement.0, measurement.1);
        }

        Paragraph::new(text).block(Block::bordered().title(title))
    }
}

pub struct Logger(pub Arc<Mutex<UI>>);

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= Level::Info
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let mut guard = self.0.lock().unwrap();
            guard.log += &format!("{}", record.args());
            guard.log += "\n";
        }
    }

    fn flush(&self) {}
}
