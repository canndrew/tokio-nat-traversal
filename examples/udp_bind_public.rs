#[macro_use]
extern crate unwrap;
#[macro_use]
extern crate net_literals;
extern crate tokio_core;
extern crate tokio_nat_traversal;
extern crate futures;
extern crate void;

use std::{io, str};
use tokio_core::net::UdpSocket;
use futures::{future, Future, Async};
use tokio_nat_traversal::UdpSocketExt;
use void::ResultVoidExt;

fn main() {
    let mut core = unwrap!(tokio_core::reactor::Core::new());
    let handle = core.handle();
    let res = core.run({
        UdpSocket::bind_public(&addr!("0.0.0.0:0"), &handle)
        .map_err(|e| panic!("Error binding socket publicly: {}", e))
        .and_then(|(socket, public_addr)| {
            println!("Success! Listening on public socket address {}", public_addr);

            // print incoming packets
            future::poll_fn(move || {
                while let Async::Ready(()) = socket.poll_read() {
                    let mut buffer = [0; 64];
                    match socket.recv_from(&mut buffer[..]) {
                        Ok((n, addr)) => {
                            let buffer = &buffer[..n];
                            match str::from_utf8(buffer) {
                                Ok(s) => {
                                    println!("received from {}: {:?}", addr, s);
                                },
                                Err(..) => {
                                    println!("received binary data from {}", addr);
                                },
                            }
                        },
                        Err(e) => {
                            if e.kind() != io::ErrorKind::WouldBlock {
                                panic!("error reading from socket! {}", e);
                            }
                        },
                    }
                }
                Ok(Async::NotReady)
            })
        })
    });
    res.void_unwrap()
}

