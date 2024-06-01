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

    // Begin capture
    sock.send_to(&vec![1], capture.unwrap()).unwrap();

    // Forward packets
    loop {
        let mut buf = vec![0; 2048];
        sock.recv(&mut buf).unwrap();
        sock.send_to(&buf, display.unwrap()).unwrap();
    }
}
