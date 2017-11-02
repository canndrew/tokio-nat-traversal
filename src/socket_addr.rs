use priv_prelude::*;

pub trait SocketAddrExt {
    fn expand_local_unspecified(&self) -> io::Result<Vec<SocketAddr>>;
}

impl SocketAddrExt for SocketAddr {
    fn expand_local_unspecified(&self) -> io::Result<Vec<SocketAddr>> {
        let ret = match *self {
            SocketAddr::V4(v4_addr) => {
                v4_addr.expand_local_unspecified()?
                .into_iter()
                .map(|v4_addr| SocketAddr::V4(v4_addr))
                .collect()
            },
            SocketAddr::V6(v6_addr) => {
                v6_addr.expand_local_unspecified()?
                .into_iter()
                .map(|v6_addr| SocketAddr::V6(v6_addr))
                .collect()
            },
        };
        Ok(ret)
    }
}

pub trait SocketAddrV4Ext {
    fn expand_local_unspecified(&self) -> io::Result<Vec<SocketAddrV4>>;
}

pub trait SocketAddrV6Ext {
    fn expand_local_unspecified(&self) -> io::Result<Vec<SocketAddrV6>>;
}

impl SocketAddrV4Ext for SocketAddrV4 {
    fn expand_local_unspecified(&self) -> io::Result<Vec<SocketAddrV4>> {
        Ok({
            self
            .ip()
            .expand_local_unspecified()?
            .into_iter()
            .map(|ip| SocketAddrV4::new(ip, self.port()))
            .collect()
        })
    }
}

impl SocketAddrV6Ext for SocketAddrV6 {
    fn expand_local_unspecified(&self) -> io::Result<Vec<SocketAddrV6>> {
        Ok({
            self
            .ip()
            .expand_local_unspecified()?
            .into_iter()
            .map(|ip| SocketAddrV6::new(ip, self.port(), self.flowinfo(), self.scope_id()))
            .collect()
        })
    }
}

