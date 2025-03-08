use std::{io::Write, process, sync::Arc, thread};

use common::chan::SubChanWriter;
use usbip::UsbIpServer;

use crate::priveleged::IPCMsg;

pub fn start_usbip_server(ipc_chan: Option<&mut SubChanWriter>) {
    match ipc_chan {
        Some(ipc_chan) => {
            let b = rmp_serde::to_vec(&IPCMsg::StartUSBIP {}).unwrap();
            ipc_chan.write_all(&b).unwrap();
        }
        None => {
            internal_start_usbip();
        }
    }
}

pub fn internal_start_usbip() {
    println!("starting usbip from pid {}", process::id());

    let usbip_server = UsbIpServer::new_from_host_with_filter(|d| {
        if d.device_descriptor().unwrap().product_id() == 0x6010 {
            println!("will offer usb {:?}", d.bus_number());
            return true;
        }

        return false;
    });

    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(usbip::server(
            "127.0.0.1:3240".parse().unwrap(),
            Arc::new(usbip_server),
        ));
    });
}
