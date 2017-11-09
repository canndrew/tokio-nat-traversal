#[macro_use]
extern crate unwrap;
#[macro_use]
extern crate net_literals;
extern crate tokio_core;
extern crate tokio_nat_traversal;
extern crate futures;

use futures::{future, Future};
use tokio_nat_traversal::UdpRendezvousServer;

fn main() {
    let mut core = unwrap!(tokio_core::reactor::Core::new());
    let handle = core.handle();
    let res = core.run({
        UdpRendezvousServer::bind_public(&addr!("0.0.0.0:0"), &handle)
        .map_err(|e| panic!("Error binding server publicly: {}", e))
        .and_then(|(server, public_addr)| {
            println!("listening on public socket address {}", public_addr);

            future::empty()
            .map(|()| drop(server))
        })
    });
    unwrap!(res);
}

