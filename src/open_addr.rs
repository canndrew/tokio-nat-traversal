use priv_prelude::*;
use std::error::Error;
use igd_async::{self, GetAnyAddressError};
use server_set::Servers;
use mc;

quick_error! {
    #[derive(Debug)]
    pub enum BindPublicError {
        Bind(e: io::Error) {
            description("error binding to local address")
            display("error binding to local address: {}", e)
            cause(e)
        }
        OpenAddr(e: OpenAddrError) {
            description("error opening external port")
            display("error opening external port: {}", e)
            cause(e)
        }
    }
}

#[derive(Debug)]
pub struct OpenAddrError {
    igd_err: GetAnyAddressError,
    kind: OpenAddrErrorKind,
}

impl fmt::Display for OpenAddrError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "IGD returned error: {}. \
                   Hole punching failed with error: {}",
                   self.igd_err, self.kind)
    }
}

impl Error for OpenAddrError {
    fn cause(&self) -> Option<&Error> {
        self.kind.cause()
    }

    fn description(&self) -> &str {
        self.kind.description()
    }
}

quick_error! {
    #[derive(Debug)]
    pub enum OpenAddrErrorKind {
        InconsistentAddrs(a0: SocketAddr, a1: SocketAddr) {
            description("NAT did not give us a consistent port mapping")
            display("NAT did not give us a consistent port mapping, got addresses {} and {}",
                     a0, a1)
        }
        HitErrorLimit(v: Vec<QueryPublicAddrError>) {
            description("hit error limit contacting traversal servers")
            display("hit error limit contacting traversal servers. {} errors: {:#?}",
                     v.len(), v)
        }
        LackOfServers {
            description("lack of traversal servers necessary to map port")
        }
    }
}

pub fn open_addr(
    protocol: Protocol,
    bind_addr: &SocketAddr,
    handle: &Handle,
) -> BoxFuture<SocketAddr, OpenAddrError> {
    let bind_addr = *bind_addr;
    let handle = handle.clone();
    igd_async::get_any_address(protocol, bind_addr)
    .or_else(move |igd_err| {
        OpenAddr {
            protocol: protocol,
            igd_err: Some(igd_err),
            handle: handle,
            bind_addr: bind_addr,
            known_addr_opt: None,
            traversal_servers: mc::traversal_servers(protocol),
            active_queries: stream::FuturesUnordered::new(),
            errors: Vec::new(),
            more_servers_timeout: None,
        }
    })
    .into_boxed()
}
    

struct OpenAddr {
    protocol: Protocol,
    igd_err: Option<GetAnyAddressError>,
    handle: Handle,
    bind_addr: SocketAddr,
    known_addr_opt: Option<SocketAddr>,
    traversal_servers: Servers,
    active_queries: stream::FuturesUnordered<BoxFuture<SocketAddr, QueryPublicAddrError>>,
    errors: Vec<QueryPublicAddrError>,
    more_servers_timeout: Option<Timeout>,
}

impl Future for OpenAddr {
    type Item = SocketAddr;
    type Error = OpenAddrError;

    fn poll(&mut self) -> Result<Async<SocketAddr>, OpenAddrError> {
        loop {
            loop {
                match self.active_queries.poll() {
                    Err(e) => self.errors.push(e),
                    Ok(Async::Ready(Some(addr))) => {
                        if let Some(known_addr) = self.known_addr_opt {
                            if known_addr == addr {
                                return Ok(Async::Ready(addr));
                            } else {
                                return Err(OpenAddrError {
                                    igd_err: unwrap!(self.igd_err.take()),
                                    kind: OpenAddrErrorKind::InconsistentAddrs(known_addr, addr),
                                });
                            }
                        }
                        self.known_addr_opt = Some(addr);
                    },
                    _ => break,
                }
            }

            if self.errors.len() >= 5 {
                let errors = mem::replace(&mut self.errors, Vec::new());
                return Err(OpenAddrError {
                    igd_err: unwrap!(self.igd_err.take()),
                    kind: OpenAddrErrorKind::HitErrorLimit(errors),
                });
            }

            if self.active_queries.len() == 2 {
                return Ok(Async::NotReady);
            }

            match self.traversal_servers.poll().void_unwrap() {
                Async::Ready(Some(server_addr)) => {
                    let active_query = mc::query_public_addr(
                        self.protocol,
                        &self.bind_addr,
                        &server_addr,
                        &self.handle
                    );
                    self.active_queries.push(active_query);
                    self.more_servers_timeout = None;
                },
                Async::Ready(None) => {
                    if self.active_queries.len() == 0 {
                        if let Some(known_addr) = self.known_addr_opt {
                            return Ok(Async::Ready(known_addr));
                        }
                        return Err(OpenAddrError {
                            igd_err: unwrap!(self.igd_err.take()),
                            kind: OpenAddrErrorKind::LackOfServers,
                        });
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
                                    return Err(OpenAddrError {
                                        igd_err: unwrap!(self.igd_err.take()),
                                        kind: OpenAddrErrorKind::LackOfServers,
                                    });
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



