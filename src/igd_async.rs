use futures::sync::oneshot;
use igd::{self, AddAnyPortError, PortMappingProtocol, SearchError};
use priv_prelude::*;

use std::thread;

#[derive(Debug)]
pub struct SearchGatewayFromTimeout {
    rx: oneshot::Receiver<Result<Gateway, SearchError>>,
}

impl Future for SearchGatewayFromTimeout {
    type Item = Gateway;
    type Error = SearchError;

    fn poll(&mut self) -> Result<Async<Gateway>, SearchError> {
        match unwrap!(self.rx.poll()) {
            Async::Ready(res) => Ok(Async::Ready(res?)),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

pub fn search_gateway_from_timeout(ipv4: Ipv4Addr, timeout: Duration) -> SearchGatewayFromTimeout {
    let (tx, rx) = oneshot::channel();
    let _ = thread::spawn(move || {
        let res = igd::search_gateway_from_timeout(ipv4, timeout);
        let res = res.map(|gateway| Gateway { inner: gateway });
        tx.send(res)
    });
    SearchGatewayFromTimeout { rx: rx }
}

#[derive(Debug)]
pub struct Gateway {
    inner: igd::Gateway,
}

#[derive(Debug)]
pub struct GetAnyAddress {
    rx: oneshot::Receiver<Result<SocketAddrV4, AddAnyPortError>>,
}

impl Future for GetAnyAddress {
    type Item = SocketAddrV4;
    type Error = AddAnyPortError;

    fn poll(&mut self) -> Result<Async<SocketAddrV4>, AddAnyPortError> {
        match unwrap!(self.rx.poll()) {
            Async::Ready(res) => Ok(Async::Ready(res?)),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

impl Gateway {
    pub fn get_any_address(
        &self,
        protocol: PortMappingProtocol,
        local_addr: SocketAddrV4,
        lease_duration: u32,
        description: &str,
    ) -> GetAnyAddress {
        let gateway = self.inner.clone();
        let description = String::from(description);
        let (tx, rx) = oneshot::channel();
        let _ = thread::spawn(move || {
            let res = gateway.get_any_address(protocol, local_addr, lease_duration, &description);
            tx.send(res)
        });
        GetAnyAddress { rx: rx }
    }
}

quick_error! {
    #[derive(Debug)]
    pub enum GetAnyAddressError {
        Ipv6NotSupported {
            description("IPv6 not supported for UPnP")
        }
        FindGateway(e: SearchError) {
            description("failed to find IGD gateway")
            display("failed to find IGD gateway: {}", e)
            cause(e)
        }
        RequestPort(e: AddAnyPortError) {
            description("error opening port on UPnP router")
            display("error opening port on UPnP router: {}", e)
            cause(e)
        }
    }
}

pub fn tcp_get_any_address(
    local_addr: SocketAddr,
) -> BoxFuture<SocketAddr, GetAnyAddressError> {
    let try = || {
        let socket_addr_v4 = match local_addr {
            SocketAddr::V4(socket_addr_v4) => socket_addr_v4,
            SocketAddr::V6(..) => return Err(GetAnyAddressError::Ipv6NotSupported),
        };
        Ok({
            search_gateway_from_timeout(*socket_addr_v4.ip(), Duration::from_millis(200))
            .map_err(GetAnyAddressError::FindGateway)
            .and_then(move |gateway| {
                gateway.get_any_address(PortMappingProtocol::TCP, socket_addr_v4, 0, "tokio-nat-traversal")
                .map_err(GetAnyAddressError::RequestPort)
                .map(SocketAddr::V4)
            })
        })
    };
    future::result(try()).flatten().into_boxed()
}

