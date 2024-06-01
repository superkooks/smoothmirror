use std::{env, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::{net::UdpSocket, task};

mod capture;
mod client;

#[derive(Serialize, Deserialize)]
struct Msg {
    seq: u64,

    #[serde(with = "serde_bytes")]
    data: Vec<u8>,
}

#[tokio::main]
async fn main() {
    let local = task::LocalSet::new();
    local
        .run_until(async move {
            let args: Vec<String> = env::args().collect();
            if args.len() < 2 {
                panic!("must be client or server");
            }

            match args[1].as_str() {
                "client" => {
                    client::start_client().await;
                }
                "server" => {
                    let mut enc = capture::new_encoder().await;
                    let sock = UdpSocket::bind("localhost:42069").await.unwrap();

                    println!("waiting for a client");
                    let mut buf = vec![0u8];
                    let (_, from) = sock.recv_from(&mut buf).await.unwrap();
                    sock.connect(from).await.unwrap();
                    println!("got client");

                    // Begin capturing
                    let mut cur_seq = 0u64;
                    let mut ticker = tokio::time::interval(Duration::from_micros(33_333));

                    loop {
                        println!("capturing...");
                        let nalus = enc.capture().await;
                        println!("captured image");

                        // Packetize the nalus into mtu sized blocks
                        let chunks: Vec<&[u8]> = nalus.chunks(1400).collect();
                        for chunk in chunks {
                            let m = Msg {
                                seq: cur_seq,
                                data: chunk.into(),
                            };
                            cur_seq += 1;

                            let buf = rmp_serde::to_vec(&m).unwrap();
                            sock.send(&buf).await.unwrap();
                            println!("sent packet");
                        }

                        ticker.tick().await;
                    }
                }
                _ => panic!("must be client or server"),
            }
        })
        .await;
}
