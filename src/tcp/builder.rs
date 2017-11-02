use priv_prelude::*;

//use socket_addr::{SocketAddrExt, MapTcpError};
use socket_addr::SocketAddrExt;

/*
quick_error! {
    #[derive(Debug)]
    pub enum BindPublicError {
        Bind(e: io::Error) {
            description("error binding to port")
            display("error binding to port: {}", e)
            cause(e)
        }
        MapPublicAddrs(e: Vec<MapTcpError>) {
            description("error mapping public addresses")
            display("error mapping public addresses: {:?}", e)
        }
    }
}
*/

pub trait TcpBuilderExt {
    fn bind_reusable(addr: &SocketAddr) -> io::Result<TcpBuilder>;
    fn expanded_local_addrs(&self) -> io::Result<Vec<SocketAddr>>;
    /*
    fn public_addrs(&self, handle: &Handle) -> io::Result<BoxStream<SocketAddr, MapTcpError>>;
    fn bind_public(addr: &SocketAddr, handle: &Handle) -> BoxFuture<(TcpBuilder, HashSet<SocketAddr>), BindPublicError>;
    */
}

impl TcpBuilderExt for TcpBuilder {
    fn bind_reusable(addr: &SocketAddr) -> io::Result<TcpBuilder> {
        let socket = match addr.ip() {
            IpAddr::V4(..) => TcpBuilder::new_v4()?,
            IpAddr::V6(..) => TcpBuilder::new_v6()?,
        };
        socket.reuse_address(true)?;

        #[cfg(target_family = "unix")]
        {
            use net2::unix::UnixTcpBuilderExt;
            socket.reuse_port(true)?;
        }

        socket.bind(addr)?;

        Ok(socket)
    }

    fn expanded_local_addrs(&self) -> io::Result<Vec<SocketAddr>> {
        let addr = self.local_addr()?;
        let addrs = addr.expand_local_unspecified()?;
        Ok(addrs)
    }

    /*
    fn public_addrs(&self, handle: &Handle) -> io::Result<BoxStream<SocketAddr, MapTcpError>> {
        let local_addr = self.local_addr()?;
        Ok(local_addr.public_tcp_addrs(handle))
    }

    fn bind_public(addr: &SocketAddr, handle: &Handle) -> BoxFuture<(TcpBuilder, HashSet<SocketAddr>), BindPublicError> {
        let try = || {
            let builder = TcpBuilder::bind_reusable(addr)?;
            let mut local_addrs = builder.expanded_local_addrs()?.into_iter().collect::<HashSet<_>>();
            let mut public_addrs = HashSet::new();
            let mut errors = Vec::new();
            let mut rx = builder.public_addrs(handle)?;
            let mut delay = Duration::from_secs(3);
            let mut timeout = Timeout::new(delay, handle);
        
            Ok({
                future::poll_fn(move || {
                    let done = match timeout.poll().void_unwrap() {
                        Async::Ready(()) => true,
                        Async::NotReady => false,
                    };

                    let done = done || loop {
                        match rx.poll() {
                            Err(e) => {
                                timeout.reset(Instant::now() + delay);
                                errors.push(e);
                            },
                            Ok(Async::NotReady) => break false,
                            Ok(Async::Ready(None)) => break true,
                            Ok(Async::Ready(Some(addr))) => {
                                if IpAddrExt::is_global(&addr.ip()) {
                                    delay /= 2;
                                }
                                timeout.reset(Instant::now() + delay);
                                public_addrs.insert(addr);
                            },
                        }
                    };

                    if done {
                        let public_addrs = mem::replace(&mut public_addrs, HashSet::new());
                        let errors = mem::replace(&mut errors, Vec::new());
                        return Ok(Async::Ready((public_addrs, errors)))
                    }

                    Ok(Async::NotReady)
                })
                .and_then(|(public_addrs, errors)| {
                    if public_addrs.iter().any(|addr| IpAddrExt::is_global(&addr.ip())) {
                        local_addrs.extend(public_addrs);
                        Ok((builder, local_addrs))
                    } else {
                        Err(BindPublicError::MapPublicAddrs(errors))
                    }
                })
            })
        };
        future::result(try().map_err(BindPublicError::Bind)).flatten().into_boxed()
    }
    */
}

