use std::io::copy;
use std::net::TcpListener;
use std::net::UdpSocket;
use std::thread;

use common::msgs::RTMsg;

fn main() {
    let sock = UdpSocket::bind("0.0.0.0:42069").unwrap();
    let tcp_sock = TcpListener::bind("0.0.0.0:42069").unwrap();

    let mut display = None;
    let mut capture = None;

    while display.is_none() || capture.is_none() {
        println!("listening");
        let mut buf = vec![0; 2048];
        let (_, from) = sock.recv_from(&mut buf).unwrap();
        println!("got packet from {:?}", from);

        match buf[0] {
            0 => capture = Some(from),
            1 => display = Some(from),
            _ => panic!("unknown packet"),
        }
    }

    // Send confirmation
    sock.send_to(&vec![1], capture.unwrap()).unwrap();
    sock.send_to(&vec![1], display.unwrap()).unwrap();

    // Accept and copy tcp connections
    thread::spawn(move || {
        println!("will accept");
        let mut client1 = tcp_sock.accept().unwrap();
        let mut client2 = tcp_sock.accept().unwrap();
        println!("accepted");

        client1.0.set_nodelay(true).unwrap();
        client2.0.set_nodelay(true).unwrap();

        let mut client12 = client1.0.try_clone().unwrap();
        let mut client22 = client2.0.try_clone().unwrap();

        thread::spawn(move || copy(&mut client12, &mut client22).unwrap());
        copy(&mut client2.0, &mut client1.0).unwrap();
    });

    // Transfer between udp connections
    let mut next_seq = 0;
    loop {
        let mut buf = vec![0; 2048];
        let (size, from) = sock.recv_from(&mut buf).unwrap();

        let msg: RTMsg = rmp_serde::from_slice(&buf[..size]).unwrap();
        if msg.seq != next_seq {
            println!(
                "missing packet from capture {} instead of {}",
                msg.seq, next_seq
            );
        }
        next_seq = msg.seq + 1;

        if from == display.unwrap() {
            sock.send_to(&buf[..size], capture.unwrap()).unwrap();
        } else if from == capture.unwrap() {
            sock.send_to(&buf[..size], display.unwrap()).unwrap();
        } else {
            println!("received packet from unknown host")
        }
    }
}
