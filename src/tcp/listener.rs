use priv_prelude::*;

use tcp::builder::TcpBuilderExt;
use socket_addr::SocketAddrExt;
use mc::{self, MapTcpError};
use igd_async::{self, GetAnyAddressError};

quick_error! {
    #[derive(Debug)]
    pub enum BindPublicError {
        BindListener(e: io::Error) {
            description("error binding listener")
            display("error binding listener: {}", e)
            cause(e)
        }
        InconsistentAddrs(a0: SocketAddr, a1: SocketAddr, igd_err: GetAnyAddressError) {
            description("NAT did not give us a consistent port mapping")
            display("NAT did not give us a consistent port mapping, got addresses {} and {}. \
                     IGD is not available either: {}",
                     a0, a1, igd_err)
        }
        HitErrorLimit(v: Vec<MapTcpError>, igd_err: GetAnyAddressError) {
            description("hit error limit contacting traversal servers")
            display("hit error limit contacting traversal servers. {} errors: {:#?} \
                     IGD not available either: {}",
                     v.len(), v, igd_err)
        }
        LackOfServers(igd_err: GetAnyAddressError) {
            description("lack of traversal servers necessary to map port")
            display("lack of traversal servers necessary to map port. \
                     IGD not available either: {}", igd_err)
        }
    }
}

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
        bind_public_inner(addr, handle)
        .map(|(listener, _bind_addr, public_addr)| {
            (listener, public_addr)
        })
        .into_boxed()
    }
}

pub fn bind_public_inner(
    addr: &SocketAddr,
    handle: &Handle,
) -> BoxFuture<(TcpListener, SocketAddr, SocketAddr), BindPublicError> {
    let handle = handle.clone();
    let try = || {
        let listener = {
            TcpListener::bind_reusable(&addr, &handle)
            .map_err(BindPublicError::BindListener)
        }?;
        let bind_addr = {
            listener
            .local_addr()
            .map_err(BindPublicError::BindListener)
        }?;
        Ok({
            igd_async::tcp_get_any_address(bind_addr)
            .or_else(move |igd_err| {
                BindPublic {
                    igd_err: Some(igd_err),
                    handle: handle.clone(),
                    bind_addr: bind_addr,
                    known_addr_opt: None,
                    traversal_servers: mc::tcp_traversal_servers(),
                    active_queries: stream::FuturesUnordered::new(),
                    errors: Vec::new(),
                    more_servers_timeout: None,
                }
            })
            .map(move |public_addr| {
                (listener, bind_addr, public_addr)
            })
        })
    };
    future::result(try()).flatten().into_boxed()
}

struct BindPublic {
    igd_err: Option<GetAnyAddressError>,
    handle: Handle,
    bind_addr: SocketAddr,
    known_addr_opt: Option<SocketAddr>,
    traversal_servers: mc::TraversalServers,
    active_queries: stream::FuturesUnordered<BoxFuture<SocketAddr, MapTcpError>>,
    errors: Vec<MapTcpError>,
    more_servers_timeout: Option<Timeout>,
}

impl Future for BindPublic {
    type Item = SocketAddr;
    type Error = BindPublicError;

    fn poll(&mut self) -> Result<Async<SocketAddr>, BindPublicError> {
        loop {
            loop {
                match self.active_queries.poll() {
                    Err(e) => self.errors.push(e),
                    Ok(Async::Ready(Some(addr))) => {
                        if let Some(known_addr) = self.known_addr_opt {
                            if known_addr == addr {
                                return Ok(Async::Ready(addr));
                            } else {
                                return Err(BindPublicError::InconsistentAddrs(known_addr, addr, unwrap!(self.igd_err.take())));
                            }
                        }
                        self.known_addr_opt = Some(addr);
                    },
                    _ => break,
                }
            }

            if self.errors.len() >= 5 {
                let errors = mem::replace(&mut self.errors, Vec::new());
                return Err(BindPublicError::HitErrorLimit(errors, unwrap!(self.igd_err.take())));
            }

            if self.active_queries.len() == 2 {
                return Ok(Async::NotReady);
            }

            match self.traversal_servers.poll().void_unwrap() {
                Async::Ready(Some(server_addr)) => {
                    let active_query = mc::tcp_query_public_addr(&self.bind_addr, &server_addr, &self.handle);
                    self.active_queries.push(active_query);
                    self.more_servers_timeout = None;
                },
                Async::Ready(None) => {
                    if self.active_queries.len() == 0 {
                        if let Some(known_addr) = self.known_addr_opt {
                            return Ok(Async::Ready(known_addr));
                        }
                        return Err(BindPublicError::LackOfServers(unwrap!(self.igd_err.take())));
                    }
                },
                Async::NotReady => {
                    if self.active_queries.len() == 0 {
                        loop {
                            if let Some(ref mut timeout) = self.more_servers_timeout {
                                if let Async::Ready(()) = timeout.poll().void_unwrap() {
                                    if let Some(known_addr) = self.known_addr_opt {
                                        return Ok(Async::Ready(known_addr));
                                    }
                                    return Err(BindPublicError::LackOfServers(unwrap!(self.igd_err.take())));
                                }
                                break;
                            } else {
                                self.more_servers_timeout = Some(
                                    Timeout::new(Duration::from_secs(2), &self.handle)
                                );
                            }
                        }
                    }
                    return Ok(Async::NotReady);
                },
            }
        }
    }
}

