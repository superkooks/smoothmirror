use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    sync::{Arc, Mutex},
    thread,
};

use crate::chan::{self, SubChanReader, SubChanWriter};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub enum ForwardMsg {
    Connect { chan_id: u64, addr: String },
    Close { chan_id: u64 }, // TODO Not used yet
}

#[derive(Clone)]
pub struct PortForwarder {
    master: Arc<Mutex<chan::TcpChan>>,
    control_writer: Arc<Mutex<SubChanWriter>>,
}

impl PortForwarder {
    pub fn new(master: Arc<Mutex<chan::TcpChan>>) -> Self {
        // Create port forwarding control
        let (control_writer, mut control_reader) = master
            .lock()
            .unwrap()
            .create_subchan(chan::ChannelId::PortForwardControl);

        // Create instance of self
        let master_c = master.clone();
        let p = Self {
            control_writer: Arc::new(Mutex::new(control_writer)),
            master,
        };

        thread::spawn(move || {
            let mut open_conns = HashMap::new();

            loop {
                let msg: ForwardMsg = rmp_serde::from_read(control_reader.by_ref()).unwrap();

                match msg {
                    ForwardMsg::Connect { chan_id, addr } => {
                        // Open the tcp connection
                        let mut ts =
                            TcpStream::connect(addr.to_socket_addrs().unwrap().next().unwrap())
                                .unwrap();
                        open_conns.insert(chan_id, ts.try_clone().unwrap());

                        // Create a sub channel
                        let (mut sc_w, mut sc_r) = master_c
                            .lock()
                            .unwrap()
                            .create_subchan(chan::ChannelId::PortForwardSub(chan_id));

                        // Create threads to copy between them
                        let mut ts_c = ts.try_clone().unwrap();
                        thread::spawn(move || std::io::copy(&mut sc_r, &mut ts_c));
                        thread::spawn(move || std::io::copy(&mut ts, &mut sc_w));
                    }

                    ForwardMsg::Close { chan_id } => {
                        open_conns[&chan_id]
                            .shutdown(std::net::Shutdown::Both)
                            .unwrap();
                    }
                }
            }
        });

        return p;
    }

    pub fn request_connection(&self, addr: String) -> (SubChanWriter, SubChanReader) {
        let chan_id = rand::random();
        let b = rmp_serde::to_vec(&ForwardMsg::Connect { chan_id, addr }).unwrap();
        self.control_writer.lock().unwrap().write_all(&b).unwrap();

        return self
            .master
            .lock()
            .unwrap()
            .create_subchan(chan::ChannelId::PortForwardSub(chan_id));
    }

    pub fn listen_and_forward(&self, listen: SocketAddr, forward: String) {
        let socket = TcpListener::bind(listen).unwrap();
        let selfc = self.clone();

        thread::spawn(move || loop {
            let (mut ts, _) = socket.accept().unwrap();
            let (mut w, mut r) = selfc.request_connection(forward.clone());

            let mut ts_c = ts.try_clone().unwrap();
            thread::spawn(move || std::io::copy(&mut r, &mut ts_c));
            thread::spawn(move || std::io::copy(&mut ts, &mut w));
        });
    }
}
