use futures::sync::oneshot;
use igd::{self, AddAnyPortError, PortMappingProtocol, SearchError};
use priv_prelude::*;
use get_if_addrs::{self, IfAddr};

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

pub fn get_any_address(
    protocol: Protocol,
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
                let protocol = match protocol {
                    Protocol::Tcp => PortMappingProtocol::TCP,
                    Protocol::Udp => PortMappingProtocol::UDP,
                };
                gateway.get_any_address(protocol, socket_addr_v4, 0, "tokio-nat-traversal")
                .map_err(GetAnyAddressError::RequestPort)
                .map(SocketAddr::V4)
            })
        })
    };
    future::result(try()).flatten().into_boxed()
}

/// # Returns
///
/// Local IP address that is on the same subnet as gateway address. Returned address is always
/// IPv4 because gateway always has IPv4 address as well.
fn local_addr_to_gateway(gateway_addr: Ipv4Addr) -> io::Result<Ipv4Addr> {
    let ifs = get_if_addrs::get_if_addrs()?;
    for interface in ifs {
        if let IfAddr::V4(addr) = interface.addr {
            if in_same_subnet(addr.ip, gateway_addr, addr.netmask) {
                return Ok(addr.ip);
            }
        }
    }

    Err(io::Error::new(io::ErrorKind::NotFound, "No local addresses to gateway"))
}

/// # Returns
///
/// `true` if given IP addresses are in the same subnet, `false` otherwise.
fn in_same_subnet(addr1: Ipv4Addr, addr2: Ipv4Addr, subnet_mask: Ipv4Addr) -> bool {
    addr1.octets().iter().zip(subnet_mask.octets().iter()).map(|(o1, o2)| o1 & o2)
        .eq(addr2.octets().iter().zip(subnet_mask.octets().iter()).map(|(o1, o2)| o1 & o2))
}

#[cfg(test)]
mod test {
    use super::*;

    mod in_same_subnet {
        use super::*;

        #[test]
        fn it_returns_true_when_given_addresses_are_in_same_subnet() {
            assert!(in_same_subnet(ipv4!("192.168.1.1"), ipv4!("192.168.1.10"), ipv4!("255.255.255.0")));
        }

        #[test]
        fn it_returns_false_when_given_addresses_are_not_in_same_subnet() {
            assert!(!in_same_subnet(ipv4!("192.168.1.1"), ipv4!("172.10.0.5"), ipv4!("255.255.255.0")));
        }
    }
}
