use std::{
    env::current_exe,
    io::Read,
    net::{TcpListener, TcpStream},
    process::Command,
};

use crate::usb;
use common::chan;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub enum IPCMsg {
    StartUSBIP {},
}

pub fn start_priveleged_process() -> Option<chan::SubChanWriter> {
    if cfg!(target_os = "linux") || cfg!(target_os = "macos") {
        let listener = TcpListener::bind("127.0.0.1:49856").unwrap();

        let _cmd = Command::new("sudo")
            .args([current_exe().unwrap().to_str().unwrap(), "priveleged"])
            .spawn()
            .unwrap();

        let (ts, _) = listener.accept().unwrap();
        let master = chan::TcpChan::new(ts);
        let (w, r) = master.create_subchan(chan::ChannelId::IPC);

        return Some(w);
    } else {
        // noop on windows
        // no need for priveleges yet
        return None;
    }
}

pub fn priveleged_entrypoint() -> ! {
    let ts = TcpStream::connect("127.0.0.1:49856").unwrap();
    let master = chan::TcpChan::new(ts);
    let (_w, mut r) = master.create_subchan(chan::ChannelId::IPC);

    loop {
        let msg: IPCMsg = rmp_serde::from_read(r.by_ref()).unwrap();

        match msg {
            IPCMsg::StartUSBIP {} => {
                usb::internal_start_usbip();
            }
        }
    }
}
