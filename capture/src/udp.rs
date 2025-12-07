use std::{
    collections::VecDeque,
    net::UdpSocket,
    time::{Duration, Instant},
};

use common::msgs::RTMsg;

const UDP_HISTORY: Duration = Duration::from_millis(1000);

pub struct UdpStream {
    sock: UdpSocket,
    history: VecDeque<(RTMsg, Instant)>,

    cur_seq: i64,
}

impl UdpStream {
    pub fn new(sock: UdpSocket) -> Self {
        Self {
            sock,
            history: VecDeque::new(),
            cur_seq: 0,
        }
    }

    pub fn send_packet(&mut self, data: Vec<u8>, is_audio: bool) {
        // Create msg struct and increment seq numbers
        let msg = RTMsg {
            seq: self.cur_seq,
            is_audio,
            data,
        };
        self.cur_seq += 1;

        // Serialize and send
        let buf = rmp_serde::to_vec(&msg).unwrap();
        self.sock.send(&buf).unwrap();

        // Store in history
        self.history.push_back((msg, Instant::now()));

        // Remove old packets
        while let Some((_, i)) = self.history.front() {
            if i.duration_since(Instant::now()) < UDP_HISTORY {
                break;
            }

            self.history.pop_front();
        }
    }

    pub fn process_nack(&mut self, seq: i64) {
        // Find the old message in the history
        match self.history.iter().find(|(m, _)| m.seq == seq) {
            Some((m, _)) => {
                // Retransmit the message
                let buf = rmp_serde::to_vec(m).unwrap();
                self.sock.send(&buf).unwrap();
            }
            None => {
                // Can't find it, don't bother
                log::info!("couldn't find packet {:?} in history", seq);
            }
        }
    }
}
