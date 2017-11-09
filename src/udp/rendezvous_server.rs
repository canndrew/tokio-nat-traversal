pub use priv_prelude::*;
use bincode::{self, Infinite};
use tokio_shared_udp_socket::SharedUdpSocket;
use bytes::Bytes;

use ECHO_REQ;
use udp::socket;
use open_addr::BindPublicError;

pub struct UdpRendezvousServer {
    local_addr: SocketAddr,
    _drop_tx: DropNotify,
}

impl UdpRendezvousServer {
    pub fn from_socket(socket: UdpSocket, handle: &Handle) -> io::Result<UdpRendezvousServer> {
        let local_addr = socket.local_addr()?;
        Ok(from_socket_inner(socket, &local_addr, handle))
    }

    pub fn bind(addr: &SocketAddr, handle: &Handle) -> io::Result<UdpRendezvousServer> {
        let socket = UdpSocket::bind(addr, handle)?;
        let server = UdpRendezvousServer::from_socket(socket, handle)?;
        Ok(server)
    }

    pub fn bind_reusable(addr: &SocketAddr, handle: &Handle) -> io::Result<UdpRendezvousServer> {
        let socket = UdpSocket::bind_reusable(addr, handle)?;
        let server = UdpRendezvousServer::from_socket(socket, handle)?;
        Ok(server)
    }

    pub fn bind_public(
        addr: &SocketAddr,
        handle: &Handle,
    ) -> BoxFuture<(UdpRendezvousServer, SocketAddr), BindPublicError> {
        let handle = handle.clone();
        socket::bind_public_with_addr(addr, &handle)
        .map(move |(socket, bind_addr, public_addr)| {
            (from_socket_inner(socket, &bind_addr, &handle), public_addr)
        })
        .into_boxed()
    }

    /// Returns the local address that this rendezvous server is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Returns all local addresses of this rendezvous server, expanding the unspecified address
    /// into a vector of all local interface addresses.
    pub fn expanded_local_addrs(&self) -> io::Result<Vec<SocketAddr>> {
        let addrs = self.local_addr.expand_local_unspecified()?;
        Ok(addrs)
    }
}

fn from_socket_inner(
    socket: UdpSocket,
    bind_addr: &SocketAddr,
    handle: &Handle,
) -> UdpRendezvousServer {
    let (drop_tx, drop_rx) = drop_notify();
    let f = {
        let socket = SharedUdpSocket::share(socket);

        socket
        .map(move |with_addr| {
            with_addr
            .into_future()
            .map_err(|(e, _with_addr)| e)
            .and_then(|(msg_opt, with_addr)| {
                if let Some(msg) = msg_opt {
                    if &msg == &ECHO_REQ[..] {
                        let addr = with_addr.remote_addr();
                        let encoded = unwrap!(bincode::serialize(&addr, Infinite));

                        return {
                            with_addr
                            .send(Bytes::from(encoded))
                            .map(|_with_addr| ())
                            .into_boxed()
                        }
                    }
                }
                future::ok(()).into_boxed()
            })
        })
        .buffer_unordered(1024)
        .log_errors(LogLevel::Info, "processing echo request")
        .until(drop_rx)
        .for_each(|()| Ok(()))
        .infallible()
    };
    handle.spawn(f);
    UdpRendezvousServer {
        _drop_tx: drop_tx,
        local_addr: *bind_addr,
    }
}
