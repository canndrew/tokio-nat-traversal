use priv_prelude::*;

use tcp::builder::TcpBuilderExt;
use socket_addr::SocketAddrExt;
use open_addr::{open_addr, BindPublicError};

pub trait TcpListenerExt {
    fn bind_reusable(addr: &SocketAddr, handle: &Handle) -> io::Result<TcpListener>;
    fn expanded_local_addrs(&self) -> io::Result<Vec<SocketAddr>>;

    /// Returns a `TcpListener` listening on the given address along with a public `SocketAddr`
    /// that can be used to connect to the listener from across the internet.
    fn bind_public(
        addr: &SocketAddr,
        handle: &Handle,
    ) -> BoxFuture<(TcpListener, SocketAddr), BindPublicError>;
}

impl TcpListenerExt for TcpListener {
    fn bind_reusable(addr: &SocketAddr, handle: &Handle) -> io::Result<TcpListener> {
        let builder = TcpBuilder::bind_reusable(addr)?;
        let bind_addr = builder.local_addr()?;
        let listener = builder.listen(1024)?;
        let listener = TcpListener::from_listener(listener, &bind_addr, handle)?;
        Ok(listener)
    }

    fn expanded_local_addrs(&self) -> io::Result<Vec<SocketAddr>> {
        let addr = self.local_addr()?;
        let addrs = addr.expand_local_unspecified()?;
        Ok(addrs)
    }

    fn bind_public(
        addr: &SocketAddr,
        handle: &Handle,
    ) -> BoxFuture<(TcpListener, SocketAddr), BindPublicError> {
        bind_public_with_addr(addr, handle)
        .map(|(listener, _bind_addr, public_addr)| {
            (listener, public_addr)
        })
        .into_boxed()
    }
}

pub fn bind_public_with_addr(
    addr: &SocketAddr,
    handle: &Handle,
) -> BoxFuture<(TcpListener, SocketAddr, SocketAddr), BindPublicError> {
    let handle = handle.clone();
    let try = || {
        let listener = {
            TcpListener::bind_reusable(&addr, &handle)
            .map_err(BindPublicError::Bind)
        }?;
        let bind_addr = {
            listener
            .local_addr()
            .map_err(BindPublicError::Bind)
        }?;
        Ok({
            open_addr(Protocol::Tcp, &bind_addr, &handle)
            .map_err(BindPublicError::OpenAddr)
            .map(move |public_addr| {
                (listener, bind_addr, public_addr)
            })
        })
    };
    future::result(try()).flatten().into_boxed()
}

