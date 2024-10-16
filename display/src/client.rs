use std::{
    net::{SocketAddr, TcpStream, UdpSocket},
    sync::{Arc, Mutex},
    time::Instant,
};

use audiopus::{packet::Packet, MutSignals};
use ffmpeg_sys_next::{self as ffmpeg};
use glium::winit::event_loop::EventLoopProxy;
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};

use crate::{ENCODED_HEIGHT, ENCODED_WIDTH, FRAME_DURATION};

#[derive(Clone, Copy)]
struct FFMPEGLater {
    decoder: *mut ffmpeg::AVCodecContext,
    parser: *mut ffmpeg::AVCodecParserContext,
}

pub struct Client {
    ff: Option<FFMPEGLater>,
    audio_decoder: audiopus::coder::Decoder,
    decoded_audio: Arc<Mutex<Vec<f32>>>,
    el_proxy: EventLoopProxy<Vec<u8>>,
}

unsafe impl Send for Client {}

pub fn init_client(
    decoded_audio: Arc<Mutex<Vec<f32>>>,
    el_proxy: EventLoopProxy<Vec<u8>>,
) -> Client {
    Client {
        ff: None,
        audio_decoder: audiopus::coder::Decoder::new(
            audiopus::SampleRate::Hz48000,
            audiopus::Channels::Stereo,
        )
        .unwrap(),
        decoded_audio,
        el_proxy,
    }
}

impl Client {
    fn consume_nalu(&mut self, mut nals: *mut ffmpeg::AVPacket) {
        let mut t = Instant::now();
        let res = unsafe { ffmpeg::avcodec_send_packet(self.ff.unwrap().decoder, nals) };
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
        let res2 = unsafe { ffmpeg::avcodec_receive_frame(self.ff.unwrap().decoder, yuv_frame) };
        println!("receive_frame={}", res2);

        if res2 < 0 {
            return;
        }

        // Convert frame to rgb
        let mut y_plane = unsafe {
            std::slice::from_raw_parts_mut((*yuv_frame).data[0], (*(*yuv_frame).buf[0]).size)
        };
        let mut u_plane = unsafe {
            std::slice::from_raw_parts_mut((*yuv_frame).data[1], (*(*yuv_frame).buf[0]).size)
        };
        let mut v_plane = unsafe {
            std::slice::from_raw_parts_mut((*yuv_frame).data[2], (*(*yuv_frame).buf[0]).size)
        };

        let mut image = vec![0; (ENCODED_WIDTH * ENCODED_HEIGHT * 4) as usize];

        yuvutils_rs::yuv420_to_rgba(
            &mut y_plane,
            unsafe { (*yuv_frame).linesize[0] } as u32,
            &mut u_plane,
            unsafe { (*yuv_frame).linesize[1] } as u32,
            &mut v_plane,
            unsafe { (*yuv_frame).linesize[2] } as u32,
            &mut image,
            4 * ENCODED_WIDTH,
            ENCODED_WIDTH,
            ENCODED_HEIGHT,
            yuvutils_rs::YuvRange::Full,
            yuvutils_rs::YuvStandardMatrix::Bt709,
        );

        // let mut rgb_frame = unsafe { ffmpeg::av_frame_alloc() };
        // let res3 =
        //     unsafe { ffmpeg::sws_scale_frame(self.ff.unwrap().scaler, rgb_frame, yuv_frame) };
        unsafe { ffmpeg::av_frame_free(std::ptr::addr_of_mut!(yuv_frame)) };
        // println!("sws_scale_frame={}", res3);
        println!(
            "took {} us to convert",
            Instant::now().duration_since(t).as_micros()
        );
        t = Instant::now();

        // let img = unsafe {
        //     std::slice::from_raw_parts((*rgb_frame).data[0], (*(*rgb_frame).buf[0]).size)
        // };

        // self.image.lock().unwrap().clear();
        // self.image.lock().unwrap().extend_from_slice(img);

        // unsafe { ffmpeg::av_frame_free(std::ptr::addr_of_mut!(rgb_frame)) };

        self.el_proxy.send_event(image).unwrap();

        println!(
            "took {} us to finish consuming nalus",
            Instant::now().duration_since(t).as_micros()
        );
    }

    fn accumulate_nalus(&mut self, mut msg: &[u8]) {
        let mut pkt = unsafe { ffmpeg::av_packet_alloc() };
        while msg.len() > 0 {
            let n = unsafe {
                ffmpeg::av_parser_parse2(
                    self.ff.unwrap().parser,
                    self.ff.unwrap().decoder,
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

    pub fn init(&mut self) {
        let codec = unsafe { ffmpeg::avcodec_find_decoder(ffmpeg::AVCodecID::AV_CODEC_ID_H264) };

        let parser = unsafe { ffmpeg::av_parser_init((*codec).id as i32) };

        let decoder = unsafe { ffmpeg::avcodec_alloc_context3(codec) };

        unsafe {
            ffmpeg::avcodec_open2(decoder, codec, std::ptr::null_mut());
        }

        self.ff = Some(FFMPEGLater { decoder, parser });
    }

    pub fn run(&mut self, tcp_sock: TcpStream) {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).unwrap();

        #[cfg(not(target_os = "macos"))]
        socket.set_recv_buffer_size(8 << 20).unwrap();

        let sock_addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
        socket.bind(&sock_addr.into()).unwrap();

        let mut sock: UdpSocket = socket.into();
        sock.connect("dw.superkooks.com:42069").unwrap();
        sock.send(&vec![1]).unwrap();
        sock.recv(&mut vec![0]).unwrap();

        tcp_sock.set_nodelay(true).unwrap();

        let mut video_stream = UdpStream::new();

        let mut t = Instant::now();
        loop {
            println!(
                "Reading packets again after {} us",
                Instant::now().duration_since(t).as_micros()
            );
            let mut buf = vec![0; 2048];
            sock.recv(&mut buf).unwrap();
            t = Instant::now();

            let msg: Msg = rmp_serde::from_slice(&buf).unwrap();
            if msg.is_audio {
                let mut output = vec![0f32; 1920 * 4];
                self.audio_decoder
                    .decode_float(
                        Some(Packet::try_from(&msg.data).unwrap()),
                        MutSignals::try_from(&mut output).unwrap(),
                        false,
                    )
                    .unwrap();
                self.decoded_audio
                    .lock()
                    .unwrap()
                    .extend_from_slice(&output);
            } else {
                for msg in video_stream.recv(msg, &mut sock) {
                    self.accumulate_nalus(&msg.data);
                }
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Msg {
    seq: i64,
    is_audio: bool,

    #[serde(with = "serde_bytes")]
    data: Vec<u8>,
}

struct UdpStream {
    next_seq: i64,
    last_in_seq: Instant,
    rearrange_buf: Vec<Msg>,
    nacked_seq: i64,
}

impl UdpStream {
    fn new() -> Self {
        return Self {
            next_seq: 0,
            last_in_seq: Instant::now(),
            rearrange_buf: vec![],
            nacked_seq: 0,
        };
    }

    fn recv(&mut self, msg: Msg, udp_sock: &mut UdpSocket) -> Vec<Msg> {
        let mut out = vec![];

        if Instant::now().duration_since(self.last_in_seq).as_micros()
            > FRAME_DURATION.as_micros() * 50
            && msg.seq - self.next_seq > 1
        {
            self.next_seq = msg.seq;
        }

        if msg.seq != self.next_seq {
            // Add it to the rearrange buf
            println!(
                "storing packet {:?} instead of {:?} in rearrange buf (sending nack > {:?})",
                msg.seq, self.next_seq, self.nacked_seq
            );

            for i in self.next_seq.max(self.nacked_seq)..msg.seq {
                udp_sock
                    .send(
                        &rmp_serde::to_vec(&Msg {
                            seq: i,
                            is_audio: false,
                            data: vec![],
                        })
                        .unwrap(),
                    )
                    .unwrap();
            }
            self.nacked_seq = msg.seq;

            self.rearrange_buf.push(msg);
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
