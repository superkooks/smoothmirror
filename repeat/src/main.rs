use std::io::copy;
use std::net::TcpListener;
use std::net::UdpSocket;
use std::thread;

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

    // Exchange addresses
    sock.send_to(display.unwrap().to_string().as_bytes(), capture.unwrap())
        .unwrap();
    sock.send_to(capture.unwrap().to_string().as_bytes(), display.unwrap())
        .unwrap();

    // Accept tcp connections
    println!("will accept");
    let mut client1 = tcp_sock.accept().unwrap();
    let mut client2 = tcp_sock.accept().unwrap();
    println!("accepted");

    let mut client12 = client1.0.try_clone().unwrap();
    let mut client22 = client2.0.try_clone().unwrap();

    thread::spawn(move || copy(&mut client12, &mut client22).unwrap());
    copy(&mut client2.0, &mut client1.0).unwrap();
}
