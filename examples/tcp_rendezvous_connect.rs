#[macro_use]
extern crate unwrap;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_nat_traversal;
extern crate futures;
extern crate future_utils;
extern crate bytes;
extern crate void;

use std::{env, fmt};
use std::net::{SocketAddr, Shutdown};
use std::str::FromStr;
use tokio_core::net::TcpStream;
use futures::{Future, Stream, Sink, Async, AsyncSink};
use tokio_io::codec::length_delimited::Framed;
use tokio_nat_traversal::TcpStreamExt;
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
    let mut args = env::args().skip(1);

    let relay_addr_str = match args.next() {
        Some(relay_addr_str) => relay_addr_str,
        None => {
            println!("missing relay server address!");
            println!("Usage: tcp_rendezvous_connect <relay_server_addr>");
            return;
        },
    };

    let relay_addr = match SocketAddr::from_str(&relay_addr_str) {
        Ok(relay_addr) => relay_addr,
        Err(e) => {
            println!("error parsing relay server address: {}", e);
            println!("Usage: tcp_rendezvous_connect <relay_server_addr>");
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
            TcpStream::rendezvous_connect(relay_channel, &handle)
            .map_err(|e| panic!("rendezvous connect failed: {}", e))
            .and_then(|stream| {
                println!("connected!");
                tokio_io::io::write_all(stream, message)
                .map_err(|e| panic!("error writing to tcp stream: {}", e))
                .and_then(|(stream, _)| {
                    unwrap!(stream.shutdown(Shutdown::Write));
                    tokio_io::io::read_to_end(stream, Vec::new())
                    .map_err(|e| panic!("error reading from tcp stream: {}", e))
                    .map(|(_, data)| {
                        let recv_message = String::from_utf8_lossy(&data);
                        println!("got message: {}", recv_message);
                    })
                })
            })
        })
    });
    res.void_unwrap()
}

