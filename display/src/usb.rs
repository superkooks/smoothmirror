use std::{io::Write, process, sync::Arc, thread};

use common::chan::SubChanWriter;
use usbip::UsbIpServer;

#[cfg(target_os = "windows")]
use std::time::Duration;
#[cfg(target_os = "windows")]
use wdi::{CreateListOptions, InstallDriverOptions, PrepareDriverOptions};

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

    simple_logging::log_to_file("z.log", log::LevelFilter::Trace).unwrap();

    #[cfg(target_os = "windows")]
    {
        println!("using libwdi to install winusb driver");
        let mut list = wdi::create_list(CreateListOptions {
            list_all: true,
            list_hubs: false,
            trim_whitespaces: true,
        })
        .unwrap();

        let wdi_info = list.iter_mut().filter(|d| d.pid == 0x6010).next().unwrap();

        wdi::prepare_driver(
            wdi_info,
            std::env::temp_dir().to_str().unwrap(),
            "wdi.inf",
            &mut PrepareDriverOptions::default(),
        )
        .unwrap();

        wdi::install_driver(
            wdi_info,
            std::env::temp_dir().to_str().unwrap(),
            "wdi.inf",
            &mut InstallDriverOptions::default(),
        )
        .unwrap();

        thread::sleep(Duration::from_millis(3000));
    }

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
