use priv_prelude::*;
use bincode;
use igd_async::{self, GetAnyAddressError};
use open_addr::{open_addr, BindPublicError};

/*
#[derive(Debug)]
pub enum UdpRendezvousConnectError<C>
where
    C: Stream<Item=Bytes>,
    C: Sink<SinkItem=Bytes>,
    <C as Stream>::Error: fmt::Debug,
    <C as Sink>::SinkError: fmt::Debug,
    C: 'static,
{
    Bind(io::Error),
    Rebind(io::Error),
    IfAddrs(io::Error),
    ChannelClosed,
    ChannelRead(<C as Stream>::Error),
    ChannelWrite(<C as Sink>::SinkError),
    DeserializeMsg(bincode::Error),
    AllAttemptsFailed(Vec<SingleRendezvousAttemptError>, Option<GetAnyAddressError>, Option<RendezvousAddrError>),
}

quick_error! {
    #[derive(Debug)]
    pub enum SingleRendezvousAttemptError {
    }
}
*/

pub trait UdpSocketExt {
    fn bind_reusable(addr: &SocketAddr, handle: &Handle) -> io::Result<UdpSocket>;
    fn expanded_local_addrs(&self) -> io::Result<Vec<SocketAddr>>;

    /// Returns a `UdpSocket` bound to the given address along with a public `SocketAddr`
    /// that can be used to message the socket from across the internet.
    fn bind_public(
        addr: &SocketAddr,
        handle: &Handle,
    ) -> BoxFuture<(UdpSocket, SocketAddr), BindPublicError>;

    /*
    fn rendezvous_connect<C>(channel: C, handle: &Handle) -> BoxFuture<UdpSocket, UdpRendezvousConnectError<C>>
    where
        C: Stream<Item=Bytes>,
        C: Sink<SinkItem=Bytes>,
        <C as Stream>::Error: fmt::Debug,
        <C as Sink>::SinkError: fmt::Debug,
        C: 'static;
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

    fn bind_public(
        addr: &SocketAddr,
        handle: &Handle,
    ) -> BoxFuture<(UdpSocket, SocketAddr), BindPublicError> {
        bind_public_with_addr(addr, handle)
        .map(|(socket, _bind_addr, public_addr)| {
            (socket, public_addr)
        })
        .into_boxed()
    }

    /*
    fn rendezvous_connect<C>(channel: C, handle: &Handle) -> BoxFuture<UdpSocket, UdpRendezvousConnectError<C>>
    where
        C: Stream<Item=Bytes>,
        C: Sink<SinkItem=Bytes>,
        <C as Stream>::Error: fmt::Debug,
        <C as Sink>::SinkError: fmt::Debug,
        C: 'static,
    {
        let handle = handle.clone();
        let (pk, _sk) = crypto::box_::gen_keypair();

        UdpSocket::bind_public(&addr!("0.0.0.0:0"), handle)
        .and_then(|(socket, public_addr)| {
            let mut addrs = {
                socket
                .expanded_local_addrs()
                .map_err(UdpRendezvousConnectError::IfAddrs)?
            };
            addrs.push(public_addr);
            let msg = UdpRendezvousMsg::Init {
                enc_pk: pk,
                open_addrs: addrs,
                rendezvous_addrs: Vec::new(),
            };

            exchange_msgs(channel, msg)
            .map(|their_msg| {
                let UdprendezvousMsg::Init {
                    enc_pk: thier_pk,
                    open_addrs: their_open_addrs,
                    rendezvous_addrs: their_rendezvous_addrs,
                } = their_msg;

                let their_open_addrs = filter_addrs(&our_addrs, &their_open_addrs);
                let incoming = OpenConnect::new(socket, their_open_addrs)
                (their_pk, incoming)
            })
        })
        .or_else(|bind_public_error| {
            let try = || {
                let listen_socket = {
                    UdpSocket::bind_reusable(&addr!("0.0.0.0:0"), &handle)
                    .map_err(UdpRendezvousConnectError::Bind)?
                };
                let our_addrs = {
                    listen_socket
                    .expanded_local_addrs()
                    .map_err(UdpRendezvousConnectError::IfAddrs)?
                };

                Ok({
                    future::loop_fn(Vec::new(), |sockets| {
                        if sockets.len() == 6 {
                            return future::ok(Loop::Break((sockets, None))).into_boxed();
                        }
                        let try = || {
                            let ttl_increments = 2 << sockets.len();
                            let socket = {
                                UdpSocket::bind_reusable(&addr!("0.0.0.0:0"), &handle)
                                .map_err(UdpRendezvousConnectError::Rebind)
                            }?;
                            let bind_addr = {
                                socket
                                .local_addr()
                                .map_err(UdpRendezvousConnectError::Rebind)?
                            };
                            socket.set_ttl(ttl_increments).map_err(UdpRendezvousConnectError::SetTtl)?;
                            Ok({
                                rendezvous_addr(Protocol::Udp, &bind_addr, &handle)
                                .map(|addr| {
                                    sockets.push((socket, addr));
                                    Loop::Continue(sockets)
                                })
                                .or_else(|err| {
                                    Ok(Loop::Break((sockets, Some(err))))
                                })
                            })
                        };
                        future::result(try()).flatten()
                    })
                    .and_then(|(sockets, rendezvous_error_opt)| {
                        let rendezvous_addrs = {
                            sockets
                            .iter()
                            .map(|&(ref _socket, addr)| addr)
                            .collect()
                        };
                        let msg = UdpRendezvousMsg::Init {
                            enc_pk: pk,
                            open_addrs: our_addrs.clone(),
                            rendezvous_addrs: rendezvous_addrs,
                        };
                        exchange_msgs(channel, msg)
                        .map(|their_msg| {
                            let UdprendezvousMsg::Init {
                                enc_pk: their_pk,
                                open_addrs: their_open_addrs,
                                rendezvous_addrs: their_rendezvous_addrs,
                            } = their_msg;

                            let punchers = {
                                sockets
                                .into_iter()
                                .map(|(socket, _our_addr)| socket)
                                .zip(their_rendezvous_addrs)
                                .enumerate()
                                .map(|((socket, their_addr), i)| {
                                    HolePunch::new(socket, their_addr, 2 << i)
                                })
                                .collect()
                            };
                            let punchers = stream::futures_unordered(punchers);

                            let their_open_addrs = filter_addrs(&our_addrs, &their_open_addrs);
                            let incoming = {
                                OpenConnect::new(socket, their_open_addrs)
                                .select(punchers)
                            };
                            (their_pk, incoming)
                        })
                    })
                })
            });
            future::result(try()).flatten()
        })
        .map(|(their_pk, incoming)| {
            if our_pk > their_pk {
                incoming
                .first_ok()
                .
            }
        })
    }
    */
}

pub fn bind_public_with_addr(
    addr: &SocketAddr,
    handle: &Handle,
) -> BoxFuture<(UdpSocket, SocketAddr, SocketAddr), BindPublicError> {
    let handle = handle.clone();
    let try = || {
        let socket = {
            UdpSocket::bind_reusable(&addr, &handle)
            .map_err(BindPublicError::Bind)
        }?;
        let bind_addr = {
            socket
            .local_addr()
            .map_err(BindPublicError::Bind)
        }?;
        Ok({
            open_addr(Protocol::Tcp, &bind_addr, &handle)
            .map_err(BindPublicError::OpenAddr)
            .map(move |public_addr| {
                (socket, bind_addr, public_addr)
            })
        })
    };
    future::result(try()).flatten().into_boxed()
}

/*
struct OpenConnect {
    their_addrs: Vec<(SocketAddr, Vec<u8>)>,
    socket: UdpSocket,
    next_send_timeout: Timeout,
    send_index: Vec<usize>,
}

impl OpenConnect {
    fn new(socket: UdpSocket, their_addrs: HashSet<SocketAddr>) -> OpenConnect {
        let mut with_data = Vec::new();
        for addr in their_addrs {
            let data = random_vec(8);
            with_data.push(addr, data);
        }
        OpenConnect {
            their_addrs: with_data,
            socket: socket,
            next_send_timeout: Timeout::new(Duration::from_millis(200)),
            send_index: Vec::new(),
        }
    }
}

impl Stream for OpenConnect {
    type Item = (UdpSocket, SocketAddr),
    type Error = UdpRendezvousConnectError,

    fn poll(&mut self) -> Result<Async<Option<(UdpSocket, SocketAddr)>>, UdpRendezvousConnectError> {
        while let Async::Ready(()) = self.next_send_timeout.poll().void_unwrap() {
            let now = Instant::now();
            self.next_send_timeout.reset(now + Duration::from_millis(200));
            self.send_index.push(0);
        }

        'outer: loop {
            match self.send_index.front_mut() {
                Some(index) => {
                    while *index < self.their_addrs.len() {
                        {
                            let (ref addr, ref data) = self.their_addrs[*index];
                            if let Async::Ready(()) = self.socket.poll_write() {
                                match self.socket.send_to(data, addr) {
                                    Ok(n) => {
                                        if n == data.len() {
                                            *index += 1;
                                        }
                                        continue;
                                    },
                                    Err(e) => {
                                        if e.kind() == io::ErrorKind::WouldBlock {
                                            self.socket.need_write();
                                            break 'outer;
                                        }
                                        self.errors.insert(*addr, e)
                                    },
                                }
                            }
                        }
                        self.their_addrs.remove(*index);
                    }
                }
                None => break,
            }

            self.send_index.remove(0);
        }

        if let Async::Ready(()) = self.socket.read_ready() {
            let mut buff = [0; 256];
            match self.socket.recv_from(&mut buff) {
                Ok((n, recv_addr)) => {
                    if self.their_addrs.iter().any(|(addr, ref data)| {
                        recv_addr == addr && data.len() == n
                    }) {
                        return Ok(Async::Ready(Some(recv_addr)));
                    }
                },
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        self.socket.need_read();
                    } else {
                        return Err(e);
                    }
                },
            }
        }
    }
}

struct SharedUdpSocket {
    socket: Arc<Socket>,
    connections: HashMap<SocketAddr, UnboundedSender<Vec<u8>>>,
}
*/

