#[macro_use]
extern crate unwrap;
#[macro_use]
extern crate net_literals;
extern crate tokio_core;
extern crate tokio_nat_traversal;
extern crate futures;

use tokio_core::net::TcpListener;
use futures::{Future, Stream};
use tokio_nat_traversal::TcpListenerExt;

fn main() {
    let mut core = unwrap!(tokio_core::reactor::Core::new());
    let handle = core.handle();
    let res = core.run({
        TcpListener::bind_public(&addr!("0.0.0.0:0"), &handle)
        .map_err(|e| panic!("Error binding listener publicly: {}", e))
        .and_then(|(listener, public_addr)| {
            println!("listening on public socket address {}", public_addr);
            listener
            .incoming()
            .for_each(|(_stream, addr)| {
                println!("got connection from {}", addr);
                Ok(())
            })
        })
    });
    unwrap!(res);
}

