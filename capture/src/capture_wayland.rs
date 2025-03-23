use std::{
    os::fd::OwnedFd,
    sync::{Arc, Mutex},
    thread,
};

use ashpd::desktop::screencast::{SourceType, Stream};
use pipewire::{self as pw, properties::properties, stream::StreamRef};

use crate::{ui::FrameLatencyInfo, CAPTURE_HEIGHT, CAPTURE_WIDTH, FRAME_RATE};

pub struct VideoCapturer {
    cur_image: Arc<Mutex<Vec<u8>>>,
}

impl VideoCapturer {
    pub fn new() -> Self {
        let tokio_rt = tokio::runtime::Runtime::new().unwrap();
        let (sel_stream, fd) = tokio_rt.block_on(Self::get_stream());

        let cur_image = Arc::new(Mutex::new(vec![]));
        let cur_image2 = cur_image.clone();

        thread::spawn(move || {
            pw::init();

            let mainloop = pw::main_loop::MainLoop::new(None).unwrap();
            let context = pw::context::Context::new(&mainloop).unwrap();
            let core = context.connect_fd(fd, None).unwrap();

            let stream = pw::stream::Stream::new(
                &core,
                "smoothmirror",
                properties! {
                    *pw::keys::MEDIA_TYPE => "Video",
                    *pw::keys::MEDIA_CATEGORY => "Capture",
                    *pw::keys::MEDIA_ROLE => "Screen",
                },
            )
            .unwrap();

            let _listener = stream
                .add_local_listener()
                .process(
                    move |stream: &StreamRef, _: &mut ()| match stream.dequeue_buffer() {
                        None => log::warn!("out of buffers"),
                        Some(mut buffer) => {
                            let datas = buffer.datas_mut();
                            if datas.is_empty() {
                                return;
                            }

                            let mut guard = cur_image2.lock().unwrap();
                            guard.clear();
                            guard.extend_from_slice(datas[0].data().unwrap());
                        }
                    },
                )
                .register()
                .unwrap();

            let obj = pw::spa::pod::object!(
                pw::spa::utils::SpaTypes::ObjectParamFormat,
                pw::spa::param::ParamType::EnumFormat,
                pw::spa::pod::property!(
                    pw::spa::param::format::FormatProperties::MediaType,
                    Id,
                    pw::spa::param::format::MediaType::Video
                ),
                pw::spa::pod::property!(
                    pw::spa::param::format::FormatProperties::MediaSubtype,
                    Id,
                    pw::spa::param::format::MediaSubtype::Raw
                ),
                pw::spa::pod::property!(
                    pw::spa::param::format::FormatProperties::VideoFormat,
                    Id,
                    pw::spa::param::video::VideoFormat::BGRA
                ),
                pw::spa::pod::property!(
                    pw::spa::param::format::FormatProperties::VideoSize,
                    Rectangle,
                    pw::spa::utils::Rectangle {
                        width: CAPTURE_WIDTH,
                        height: CAPTURE_HEIGHT
                    }
                ),
                pw::spa::pod::property!(
                    pw::spa::param::format::FormatProperties::VideoFramerate,
                    Choice,
                    Range,
                    Fraction,
                    pw::spa::utils::Fraction {
                        num: FRAME_RATE,
                        denom: 1
                    },
                    pw::spa::utils::Fraction { num: 0, denom: 1 },
                    pw::spa::utils::Fraction {
                        num: 1000,
                        denom: 1
                    }
                ),
            );
            let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
                std::io::Cursor::new(Vec::new()),
                &pw::spa::pod::Value::Object(obj),
            )
            .unwrap()
            .0
            .into_inner();

            let mut params = [pw::spa::pod::Pod::from_bytes(&values).unwrap()];

            stream
                .connect(
                    pw::spa::utils::Direction::Input,
                    Some(sel_stream.pipe_wire_node_id()),
                    pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
                    &mut params,
                )
                .unwrap();

            mainloop.run();
        });

        Self { cur_image }
    }

    pub async fn get_stream() -> (Stream, OwnedFd) {
        let proxy = ashpd::desktop::screencast::Screencast::new().await.unwrap();
        let session = proxy.create_session().await.unwrap();

        proxy
            .select_sources(
                &session,
                ashpd::desktop::screencast::CursorMode::Embedded,
                SourceType::Monitor.into(),
                false,
                None,
                ashpd::desktop::PersistMode::DoNot,
            )
            .await
            .unwrap();

        let resp = proxy
            .start(&session, None)
            .await
            .unwrap()
            .response()
            .unwrap();

        let fd = proxy.open_pipe_wire_remote(&session).await.unwrap();
        (resp.streams().first().unwrap().clone(), fd)
    }

    pub fn capture_frame(&mut self) -> (Vec<u8>, FrameLatencyInfo) {
        let mut f = FrameLatencyInfo::new();
        let v = self.cur_image.lock().unwrap().clone();
        f.measure("cur_image clone");

        (v, f)
    }
}
