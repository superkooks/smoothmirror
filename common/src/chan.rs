use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    net::TcpStream,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash, Clone, Copy)]
pub enum ChannelId {
    // Inter-client protocol
    Keys,
    PortForwardControl,
    PortForwardSub(u64),

    // Internal client IPC protocol
    IPC,
}

#[derive(Serialize, Deserialize)]
struct ChanPacket {
    chan_id: ChannelId,
    #[serde(with = "serde_bytes")]
    data: Vec<u8>,
}

pub struct TcpChan {
    subchans: Arc<Mutex<HashMap<ChannelId, Sender<ChanPacket>>>>,
    subchan_tx: Sender<ChanPacket>, // used for creating more subchannels

    pending_subchans: Arc<Mutex<HashMap<ChannelId, VecDeque<u8>>>>,
}

impl TcpChan {
    pub fn new(tcp: TcpStream) -> Self {
        let (subchan_tx, subchan_rx) = mpsc::channel();
        let sbchc = Arc::new(Mutex::new(HashMap::new()));
        let pending = Arc::new(Mutex::new(HashMap::new()));

        let r_ts = tcp.try_clone().unwrap();
        let r_sbchc = sbchc.clone();
        let r_pending = pending.clone();
        thread::spawn(move || {
            TcpChan::_read(r_ts, r_sbchc, r_pending);
        });

        thread::spawn(move || {
            TcpChan::_write(tcp, subchan_rx);
        });

        Self {
            subchans: sbchc,
            subchan_tx,
            pending_subchans: pending,
        }
    }

    pub fn create_subchan(&self, id: ChannelId) -> (SubChanWriter, SubChanReader) {
        let (tx, rx) = mpsc::channel();
        self.subchans.lock().unwrap().insert(id, tx);

        // Check for a pending subchan, and use its buf
        let mut guard = self.pending_subchans.lock().unwrap();
        let r_buf = match guard.get(&id) {
            Some(_) => guard.remove(&id).unwrap(),
            None => VecDeque::new(),
        };

        (
            SubChanWriter {
                chan_id: id,
                tx: self.subchan_tx.clone(),
            },
            SubChanReader { rx, r_buf },
        )
    }

    fn _read(
        mut ts: TcpStream,
        subchans: Arc<Mutex<HashMap<ChannelId, Sender<ChanPacket>>>>,
        pending_subchans: Arc<Mutex<HashMap<ChannelId, VecDeque<u8>>>>,
    ) {
        loop {
            let p: ChanPacket = rmp_serde::from_read(std::io::Read::by_ref(&mut ts)).unwrap();
            let sub_guard = subchans.lock().unwrap();
            if sub_guard.get(&p.chan_id).is_none() {
                // Write to the pending subchan for it
                let mut pending_guard = pending_subchans.lock().unwrap();
                if pending_guard.get(&p.chan_id).is_none() {
                    // Create a pending subchan if it doesn't exist
                    pending_guard.insert(p.chan_id, VecDeque::new());
                }

                pending_guard.get_mut(&p.chan_id).unwrap().extend(p.data);
            } else {
                sub_guard[&p.chan_id].send(p).unwrap();
            }
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

pub struct SubChanReader {
    rx: Receiver<ChanPacket>,
    r_buf: VecDeque<u8>,
}

impl Read for SubChanReader {
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

pub struct SubChanWriter {
    chan_id: ChannelId,
    tx: Sender<ChanPacket>,
}

impl Write for SubChanWriter {
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
