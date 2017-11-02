use priv_prelude::*;
use future_utils::mpsc::{unbounded, UnboundedSender, UnboundedReceiver};
use rand;
use bincode;
use tokio_io;
use ECHO_REQ;

lazy_static! {
    static ref MC: Mutex<Mc> = Mutex::new(Mc::new());
}

struct Mc {
    tcp_traversal_servers: HashSet<SocketAddr>,
    tcp_traversal_servers_iterators: Vec<UnboundedSender<(SocketAddr, bool)>>,
}

impl Mc {
    pub fn new() -> Mc {
        Mc {
            tcp_traversal_servers: HashSet::new(),
            tcp_traversal_servers_iterators: Vec::new(),
        }
    }
}

pub fn add_tcp_traversal_server(addr: &SocketAddr) {
    let mut mc = unwrap!(MC.lock());

    mc.tcp_traversal_servers_iterators.retain(|sender| {
        sender.unbounded_send((*addr, true)).is_ok()
    });

    mc.tcp_traversal_servers.insert(*addr);
}

pub fn remove_tcp_traversal_server(addr: &SocketAddr) {
    let mut mc = unwrap!(MC.lock());

    mc.tcp_traversal_servers_iterators.retain(|sender| {
        sender.unbounded_send((*addr, false)).is_ok()
    });

    mc.tcp_traversal_servers.remove(addr);
}

pub fn tcp_traversal_servers() -> TraversalServers {
    let (tx, rx) = unbounded();
    let mut mc = unwrap!(MC.lock());
    mc.tcp_traversal_servers_iterators.push(tx);
    let servers = mc.tcp_traversal_servers.clone();
    TraversalServers {
        tcp_traversal_servers: servers,
        modifications: rx,
    }
}

pub struct TraversalServers {
    tcp_traversal_servers: HashSet<SocketAddr>,
    modifications: UnboundedReceiver<(SocketAddr, bool)>,
}

impl Stream for TraversalServers {
    type Item = SocketAddr;
    type Error = Void;

    fn poll(&mut self) -> Result<Async<Option<SocketAddr>>, Void> {
        while let Async::Ready(Some((server, add))) = self.modifications.poll().void_unwrap() {
            if add {
                self.tcp_traversal_servers.insert(server);
            } else {
                self.tcp_traversal_servers.remove(&server);
            }
        }

        let server = match self.tcp_traversal_servers.remove_random(&mut rand::thread_rng()) {
            Some(server) => server,
            None => return Ok(Async::NotReady),
        };

        Ok(Async::Ready(Some(server)))
    }
}

quick_error! {
    #[derive(Debug)]
    pub enum MapTcpError {
        Bind(e: io::Error) {
            description("error binding to socket address")
            display("error binding to socket address: {}", e)
            cause(e)
        }
        Connect(e: io::Error) {
            description("error connecting to echo server")
            display("error connecting to echo server: {}", e)
            cause(e)
        }
        ConnectTimeout {
            description("timed out contacting server")
        }
        SendRequest(e: io::Error) {
            description("error sending request to echo server")
            display("error sending request to echo server: {}", e)
            cause(e)
        }
        ReadResponse(e: io::Error) {
            description("error reading response from echo server")
            display("error reading response from echo server: {}", e)
            cause(e)
        }
        Deserialize(e: bincode::Error) {
            description("error deserializing response from echo server")
            display("error deserializing response from echo server: {}", e)
            cause(e)
        }
        ResponseTimeout {
            description("timed out waiting for response from echo server")
        }
    }
}

pub fn tcp_query_public_addr(
    bind_addr: &SocketAddr,
    server_addr: &SocketAddr,
    handle: &Handle,
) -> BoxFuture<SocketAddr, MapTcpError> {
    let bind_addr = *bind_addr;
    let server_addr = *server_addr;
    let handle = handle.clone();
    TcpStream::connect_reusable(&bind_addr, &server_addr, &handle)
    .map_err(|err| match err {
        ConnectReusableError::Connect(e) => MapTcpError::Connect(e),
        ConnectReusableError::Bind(e) => MapTcpError::Bind(e),
    })
    .with_timeout(&handle, Duration::from_secs(3), MapTcpError::ConnectTimeout)
    .and_then(|stream| {
        tokio_io::io::write_all(stream, ECHO_REQ)
        .map(|(stream, _buf)| stream)
        .map_err(MapTcpError::SendRequest)
    })
    .and_then(move |stream| {
        tokio_io::io::read_to_end(stream, Vec::new())
        .map_err(MapTcpError::ReadResponse)
        .and_then(|(_stream, data)| {
            bincode::deserialize(&data)
            .map_err(MapTcpError::Deserialize)
        })
        .with_timeout(&handle, Duration::from_secs(2), MapTcpError::ResponseTimeout)
    })
    .into_boxed()
}

