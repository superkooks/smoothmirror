use std::net::UdpSocket;

fn main() {
    let sock = UdpSocket::bind("0.0.0.0:42069").unwrap();

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
}
