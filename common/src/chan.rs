use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    net::TcpStream,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy)]
pub enum ChannelId {
    Keys = 0,
}

#[derive(Serialize, Deserialize)]
struct ChanPacket {
    chan_id: u32,
    #[serde(with = "serde_bytes")]
    data: Vec<u8>,
}

pub struct TcpChan {
    tcp: TcpStream,
    subchans: HashMap<u32, Sender<ChanPacket>>,
    subchan_rx: Receiver<ChanPacket>,
    subchan_tx: Sender<ChanPacket>, // only used duriung initialization
}

impl TcpChan {
    pub fn new(tcp: TcpStream) -> Self {
        let (subchan_tx, subchan_rx) = mpsc::channel();

        Self {
            tcp,
            subchans: HashMap::new(),
            subchan_rx,
            subchan_tx,
        }
    }

    pub fn create_subchan(&mut self, id: ChannelId) -> SubChan {
        let (tx, rx) = mpsc::channel();
        self.subchans.insert(id as u32, tx);

        SubChan {
            chan_id: id as u32,
            rx,
            tx: self.subchan_tx.clone(),
            r_buf: VecDeque::new(),
        }
    }

    // Creates a new thread to multiplex and demultiplex packets. Non-blocking.
    pub fn start_rw(self) {
        let r_ts = self.tcp.try_clone().unwrap();
        thread::spawn(move || {
            TcpChan::_read(r_ts, self.subchans);
        });

        thread::spawn(move || {
            TcpChan::_write(self.tcp, self.subchan_rx);
        });
    }

    fn _read(ts: TcpStream, subchans: HashMap<u32, Sender<ChanPacket>>) {
        loop {
            let p: ChanPacket = rmp_serde::from_read(ts.try_clone().unwrap()).unwrap();
            subchans[&p.chan_id].send(p).unwrap();
        }
    }

    fn _write(mut ts: TcpStream, subchan_rx: Receiver<ChanPacket>) {
        loop {
            let p = subchan_rx.recv().unwrap();
            let b = rmp_serde::to_vec(&p).unwrap();
            ts.write_all(&b).unwrap();
        }
    }
}

pub struct SubChan {
    chan_id: u32,
    rx: Receiver<ChanPacket>,
    tx: Sender<ChanPacket>,

    r_buf: VecDeque<u8>,
}

impl Read for SubChan {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // Recv a packet if the buf is empty
        if self.r_buf.len() == 0 {
            let packet = self.rx.recv();
            match packet {
                Ok(packet) => {
                    self.r_buf.extend(packet.data);
                }
                Err(e) => {
                    return Err(std::io::Error::other(e));
                }
            }
        }

        // Read from the buf
        self.r_buf.read(buf)
    }
}

impl Write for SubChan {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Serialize to packet and send
        match self.tx.send(ChanPacket {
            chan_id: self.chan_id,
            data: buf.to_vec(),
        }) {
            Ok(_) => Ok(buf.len()),
            Err(e) => Err(std::io::Error::other(e)),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        // noop
        Ok(())
    }
}
