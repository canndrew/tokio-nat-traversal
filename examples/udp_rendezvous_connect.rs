/// To use this, use a server with publicly accessible ports to act as a relay server between two
/// connecting udp sockets. This relay server will act as the channel when negotiating the
/// rendezvous connect.
///
/// For example, log into a VPS and run:
/// $ socat TCP-LISTEN:45666 TCP-LISTEN:45667
///
/// The run this example on two machines, on seperate networks, both hidden behind NATs:
///
/// $ cargo run --example udp_rendezvous_connect -- <address of your vps>:45666 blah blah blah
/// $ cargo run --example udp_rendezvous_connect -- <address of your vps>:45667 blah blah blah
/// 
/// If successful, the peers should be able to communicate directly with each other over UDP.
#[macro_use]
extern crate unwrap;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_nat_traversal;
extern crate futures;
extern crate future_utils;
extern crate bytes;
extern crate void;
extern crate env_logger;

use std::{env, fmt};
use std::net::SocketAddr;
use std::str::FromStr;
use tokio_core::net::{TcpStream, UdpSocket};
use futures::{future, Future, Stream, Sink, Async, AsyncSink};
use futures::future::Loop;
use tokio_io::codec::length_delimited::Framed;
use tokio_nat_traversal::UdpSocketExt;
use void::ResultVoidExt;

// TODO: figure out how to not need this.
struct DummyDebug<S>(S);

impl<S> fmt::Debug for DummyDebug<S> {
    fn fmt(&self, _f: &mut fmt::Formatter) -> fmt::Result {
        Ok(())
    }
}

impl<S: Stream> Stream for DummyDebug<S> {
    type Item = S::Item;
    type Error = S::Error;

    fn poll(&mut self) -> Result<Async<Option<Self::Item>>, Self::Error> {
        self.0.poll()
    }
}

impl<S: Sink> Sink for DummyDebug<S> {
    type SinkItem = S::SinkItem;
    type SinkError = S::SinkError;

    fn start_send(&mut self, item: Self::SinkItem) -> Result<AsyncSink<Self::SinkItem>, Self::SinkError> {
        self.0.start_send(item)
    }

    fn poll_complete(&mut self) -> Result<Async<()>, Self::SinkError> {
        self.0.poll_complete()
    }
}

fn main() {
    unwrap!(env_logger::init());

    let mut args = env::args().skip(1);

    let relay_addr_str = match args.next() {
        Some(relay_addr_str) => relay_addr_str,
        None => {
            println!("missing relay server address!");
            println!("Usage: udp_rendezvous_connect <relay server addr> <message to send>");
            return;
        },
    };

    let relay_addr = match SocketAddr::from_str(&relay_addr_str) {
        Ok(relay_addr) => relay_addr,
        Err(e) => {
            println!("error parsing relay server address: {}", e);
            println!("Usage: tcp_rendezvous_connect <relay server addr> <message to send>");
            return;
        },
    };

    let message = args.next().unwrap_or(String::new());
    let message = args.fold(message, |mut message, arg| {
        message.push(' ');
        message.push_str(&arg);
        message
    });
    let message: Vec<u8> = message.into();

    let mut core = unwrap!(tokio_core::reactor::Core::new());
    let handle = core.handle();
    let res = core.run({
        TcpStream::connect(&relay_addr, &handle)
        .map_err(|e| panic!("error connecting to relay server: {}", e))
        .and_then(move |relay_stream| {
            let relay_channel = DummyDebug(Framed::new(relay_stream).map(|bytes| bytes.freeze()));
            UdpSocket::rendezvous_connect(relay_channel, &handle)
            .map_err(|e| panic!("rendezvous connect failed: {}", e))
            .and_then(|(socket, addr)| {
                println!("connected!");
                socket
                .send_dgram(message, addr)
                .map_err(|e| panic!("error writing to udp socket: {}", e))
                .and_then(move |(socket, _)| {
                    future::loop_fn(socket, move |socket| {
                        let buffer = vec![0u8; 64 * 1024];
                        socket
                        .recv_dgram(buffer)
                        .map_err(|e| panic!("error receiving on udp socket: {}", e))
                        .map(move |(socket, buffer, len, recv_addr)| {
                            if recv_addr == addr {
                                let recv_message = String::from_utf8_lossy(&buffer[..len]);
                                println!("got message: {}", recv_message);
                            }
                            Loop::Continue(socket)
                        })
                    })
                })
            })
        })
    });
    res.void_unwrap()
}


