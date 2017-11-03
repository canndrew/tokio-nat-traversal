pub use priv_prelude::*;
use tokio_io;
use bincode::{self, Infinite};

use ECHO_REQ;
use tcp::listener::{self, TcpListenerExt};
use open_addr::BindPublicError;
//use socket_addr::MapTcpError;

pub struct TcpRendezvousServer {
    local_addr: SocketAddr,
    _drop_tx: DropNotify,
}

impl TcpRendezvousServer {
    pub fn from_listener(listener: TcpListener, handle: &Handle) -> io::Result<TcpRendezvousServer> {
        let local_addr = listener.local_addr()?;
        Ok(from_listener_inner(listener, &local_addr, handle))
    }

    pub fn bind(addr: &SocketAddr, handle: &Handle) -> io::Result<TcpRendezvousServer> {
        let listener = TcpListener::bind(addr, handle)?;
        let server = TcpRendezvousServer::from_listener(listener, handle)?;
        Ok(server)
    }

    pub fn bind_reusable(addr: &SocketAddr, handle: &Handle) -> io::Result<TcpRendezvousServer> {
        let listener = TcpListener::bind_reusable(addr, handle)?;
        let server = TcpRendezvousServer::from_listener(listener, handle)?;
        Ok(server)
    }

    pub fn bind_public(
        addr: &SocketAddr,
        handle: &Handle,
    ) -> BoxFuture<(TcpRendezvousServer, SocketAddr), BindPublicError> {
        let handle = handle.clone();
        listener::bind_public_with_addr(addr, &handle)
        .map(move |(listener, bind_addr, public_addr)| {
            (from_listener_inner(listener, &bind_addr, &handle), public_addr)
        })
        .into_boxed()
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn expanded_local_addrs(&self) -> io::Result<Vec<SocketAddr>> {
        let addrs = self.local_addr.expand_local_unspecified()?;
        Ok(addrs)
    }
}

fn from_listener_inner(
    listener: TcpListener,
    bind_addr: &SocketAddr,
    handle: &Handle,
) -> TcpRendezvousServer {
    let (drop_tx, drop_rx) = drop_notify();
    let f = {
        let handle = handle.clone();

        listener
        .incoming()
        .map(move |(stream, addr)| {
            let buf = [0u8; 8];
            tokio_io::io::read_exact(stream, buf)
            .and_then(move |(stream, buf)| {
                if buf == ECHO_REQ {
                    let encoded = unwrap!(bincode::serialize(&addr, Infinite));
                    tokio_io::io::write_all(stream, encoded)
                    .map(|(_stream, _encoded)| ())
                    .into_boxed()
                } else {
                    future::ok(()).into_boxed()
                }
            })
            .with_timeout(&handle, Duration::from_secs(2), io::ErrorKind::TimedOut.into())
        })
        .buffer_unordered(1024)
        .log_errors(LogLevel::Info, "processing echo request")
        .until(drop_rx)
        .for_each(|()| Ok(()))
        .infallible()
    };
    handle.spawn(f);
    TcpRendezvousServer {
        _drop_tx: drop_tx,
        local_addr: *bind_addr,
    }
}

