use priv_prelude::*;

pub trait UdpSocketExt {
    fn bind_reusable(addr: &SocketAddr, handle: &Handle) -> io::Result<UdpSocket>;
    fn expanded_local_addrs(&self) -> io::Result<Vec<SocketAddr>>;

    /*
    /// Returns a `UdpSocket` bound to the given address along with a public `SocketAddr`
    /// that can be used to message the socket from across the internet.
    fn bind_public(
        addr: &SocketAddr,
        handle: &Handle,
    ) -> BoxFuture<(UdpSocket, SocketAddr), BindPublicError>;
    */
}

impl UdpSocketExt for UdpSocket {
    fn bind_reusable(addr: &SocketAddr, handle: &Handle) -> io::Result<UdpSocket> {
        let socket = match addr.ip() {
            IpAddr::V4(..) => UdpBuilder::new_v4()?,
            IpAddr::V6(..) => UdpBuilder::new_v6()?,
        };
        socket.reuse_address(true)?;

        #[cfg(target_family = "unix")]
        {
            use net2::unix::UnixUdpBuilderExt;
            socket.reuse_port(true)?;
        }

        let socket = socket.bind(addr)?;
        let socket = UdpSocket::from_socket(socket, handle)?;

        Ok(socket)
    }

    fn expanded_local_addrs(&self) -> io::Result<Vec<SocketAddr>> {
        let addr = self.local_addr()?;
        let addrs = addr.expand_local_unspecified()?;
        Ok(addrs)
    }

    /*
    fn bind_public(
        addr: &SocketAddr,
        handle: &Handle,
    ) -> BoxFuture<(UdpSocket, SocketAddr), BindPublicError> {
        bind_public_inner(addr, handle)
        .map(|(listener, _bind_addr, public_addr)| {
            (listener, public_addr)
        })
        .into_boxed()
    }
    */
}

/*
pub fn bind_public_inner(
    addr: &SocketAddr,
    handle: &Handle,
) -> BoxFuture<(UdpSocket, SocketAddr, SocketAddr), BindPublicError> {
    let handle = handle.clone();
    let try = || {
        let socket = {
            UdpSocket::bind_reusable(&addr, &handle)
            .map_err(BindPublicError::BindSocket)
        }?;
        Ok({
            igd_async::udp_get_any_address(bind_addr)
            .or_else(move |igd_err| {
                BindPublic {
                    igd_err: Some(igd_err),
                    handle: handle.clone(),
                    bind_addr: bind_addr,
                    known_addr_opt: None,
                    traversal_servers: mc::udp_traversal_servers(),
                    active_queries: stream::FuturesUnordered::new(),
                    errors: Vec::new(),
                    more_servers_timeout: None,
                }
            })
            .map(move |public_addr| {
                (socket, bind_addr, public_addr)
            })
        })
    };
    future::result(try()).flatten().into_boxed()
}
*/

