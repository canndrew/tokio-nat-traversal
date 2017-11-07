use priv_prelude::*;
use bincode;
use tokio_shared_udp_socket::{SharedUdpSocket, WithAddress};
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
    SocketWrite(io::Error),
    AllAttemptsFailed(Vec<HolePunchError>, Option<GetAnyAddressError>, Option<RendezvousAddrError>),
}
*/

/// Extension methods for `UdpSocket`.
pub trait UdpSocketExt {
    /// Bind reusably to the given address. This method can be used to create multiple UDP sockets
    /// bound to the same local address.
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
                    rendezvous_addrs: _their_rendezvous_addrs,
                } = their_msg;

                let their_open_addrs = filter_addrs(&our_addrs, &their_open_addrs);
                let incoming = {
                    open_connect(socket, their_open_addrs, true)
                    .into_boxed()
                };
                (their_pk, incoming, None, None)
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
                        let (sockets, rendezvous_addrs) = sockets.into_iter().unzip::<_, Vec<_>, _>();
                        let msg = UdpRendezvousMsg::Init {
                            enc_pk: pk,
                            open_addrs: our_addrs.clone(),
                            rendezvous_addrs: rendezvous_addrs,
                        };
                        exchange_msgs(channel, msg)
                        .map(|their_msg| {
                            let UdpRendezvousMsg::Init {
                                enc_pk: their_pk,
                                open_addrs: their_open_addrs,
                                rendezvous_addrs: their_rendezvous_addrs,
                            } = their_msg;

                            let mut punchers = FuturesUnordered::new();
                            let iter = {
                                sockets
                                .into_iter()
                                .zip(their_rendezvous_addrs)
                                .enumerate()
                            };
                            for (i, (socket, their_addr)) in iter {
                                let ttl_increment = 2 << i;
                                socket.set_ttl(ttl_increment)?;
                                let shared = SharedUdpSocket::share(socket);
                                let with_addr = shared.with_addr(their_addr);
                                let timeout = Instant::now() + Duration::millis(200);
                                let puncher = send_from_syn(handle, shared, timeout, 0, ttl_increment);
                                punchers.push(puncher);
                            }

                            let their_open_addrs = filter_addrs(&our_addrs, &their_open_addrs);

                            let incoming = {
                                open_connect(listen_socket, their_open_addrs, false)
                                .select(punchers)
                                .into_boxed()
                            };
                            (their_pk, incoming, Some(bind_public_error), rendezvous_error_opt)
                        })
                    })
                })
            });
            future::result(try()).flatten()
        })
        .map(|(their_pk, incoming, bind_public_error_opt, rendezouvs_error_opt)| {
            if our_pk > their_pk {
                incoming
                .and_then(|(socket, chosen)| {
                    if chosen {
                        return Err(HolePunchError::UnexpectedMessage);
                    }
                    Ok(socket)
                })
                .first_ok()
                .and_then(choose)
            } else {
                incoming
                .map(take_chosen)
                .futures_unordered()
                .filter_map(|opt| opt)
                .first_ok()
            }
            .map_err(|v| {
                UdpRendezvousConnectError::AllAttemptsFailed(v, bind_public_error_opt, rendezvous_error_opt)
            })
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
quick_error! {
    #[derive(Debug)]
    pub enum HolePunchError {
        SendMessage(e: io::Error) {
            description("error sending message to peer")
            display("error sending message to peer: {}", e)
            cause(e)
        }
        ReadMessage(e: io::Error) {
            description("error receiving message from peer")
            display("error receiving message from peer: {}", e)
            cause(e)
        }
        RecvMessage(e: io::Error) {
            description("error receiving message on socket")
            display("error receiving message on socket: {}", e)
            cause(e)
        }
        Deserialization(e: bincode::Error) {
            description("error deserializing packet from peer")
            display("error deserializing packet from peer: {}", e)
            cause(e)
        }
        UnexpectedMessage {
            description("received unexpected hole-punch message type")
        }
    }
}

fn send_from_syn(
    handle: &Handle,
    socket: WithAddress,
    next_timeout: Instant,
    syns_acks_sent: u32,
    ttl_increment: u32,
) -> BoxFuture<(WithAddress, bool), HolePunchError> {
    let handle = handle.clone();
    let send_msg = unwrap!(bincode::serialize(&HolePunchMsg::Syn, bincode::Infinite));

    socket
    .send(Bytes::from(send_msg))
    .map_err(HolePunchError::SendMessage)
    .and_then(move |socket| {
        let try = || {
            let syns_acks_sent = syns_acks_sent + 1;
            if syns_acks_sent % 5 == 0 && ttl_increment != 0 {
                let ttl = socket.get_ref().ttl()?;
                socket.get_ref().set_ttl(ttl + ttl_increment)?;
            }
            recv_from_syn(&handle, socket, next_timeout, syns_acks_sent, ttl_increment)
        };
        future::result(try().map_err(HolePunchError::SetTtl)).flatten()
    })
    .into_boxed()
}

fn recv_from_syn(
    handle: &Handle,
    socket: WithAddress,
    timeout: Instant,
    syns_acks_sent: u32,
    ttl_increment: u32,
) -> BoxFuture<(WithAddress, bool), HolePunchError> {
    let handle = handle.clone();
    socket
    .with_timeout_at(timeout, &handle)
    .into_future()
    .map_err(|(e, _)| HolePunchError::ReadMessage(e))
    .and_then(move |(msg_opt, socket_timeout)| {
        let socket = socket_timeout.into_inner();
        match msg_opt {
            None => send_from_syn(&handle, socket, timeout + Duration::from_millis(200), syns_acks_sent, ttl_increment),
            Some(recv_msg) => {
                match bincode::deserialize(&recv_msg) {
                    Err(e) => {
                        future::err(HolePunchError::Deserialization(e)).into_boxed()
                    },
                    Ok(HolePunchMsg::Syn) => {
                        send_from_ack(&handle, socket, Instant::now() + Duration::from_millis(200), syns_acks_sent, ttl_increment)
                    },
                    Ok(HolePunchMsg::Ack) => {
                        send_from_ack_ack(&handle, socket, Instant::now() + Duration::from_millis(200), 0)
                    },
                    Ok(HolePunchMsg::AckAck) | Ok(HolePunchMsg::Choose) => {
                        future::err(HolePunchError::UnexpectedMessage).into_boxed()
                    },
                }
            },
        }
    })
    .into_boxed()
}

fn send_from_ack(
    handle: &Handle,
    socket: WithAddress,
    next_timeout: Instant,
    syns_acks_sent: u32,
    ttl_increment: u32,
) -> BoxFuture<(WithAddress, bool), HolePunchError> {
    let handle = handle.clone();
    let send_msg = unwrap!(bincode::serialize(&HolePunchMsg::Ack, bincode::Infinite));

    socket
    .send(Bytes::from(send_msg))
    .map_err(HolePunchError::SendMessage)
    .and_then(move |socket| {
        let try = || {
            let syns_acks_sent = syns_acks_sent + 1;
            if syns_acks_sent % 5 == 0 && ttl_increment != 0 {
                let ttl = socket.get_ref().ttl()?;
                socket.get_ref().set_ttl(ttl + ttl_increment)?;
            }
            recv_from_ack(&handle, socket, next_timeout, syns_acks_sent, ttl_increment)
        };
        future::result(try().map_err(HolePunchError::SetTtl)).flatten()
    })
    .into_boxed()
}

fn recv_from_ack(
    handle: &Handle,
    socket: WithAddress,
    timeout: Instant,
    syns_acks_sent: u32,
    ttl_increment: u32,
) -> BoxFuture<(WithAddress, bool), HolePunchError> {
    let handle = handle.clone();
    socket
    .with_timeout_at(timeout, &handle)
    .into_future()
    .map_err(|(e, _)| HolePunchError::ReadMessage(e))
    .and_then(move |(msg_opt, socket_timeout)| {
        let socket = socket_timeout.into_inner();
        match msg_opt {
            None => send_from_ack(&handle, socket, timeout + Duration::from_millis(200), syns_acks_sent, ttl_increment),
            Some(recv_msg) => {
                match bincode::deserialize(&recv_msg) {
                    Err(e) => {
                        future::err(HolePunchError::Deserialization(e)).into_boxed()
                    },
                    Ok(HolePunchMsg::Syn) => send_from_ack(&handle, socket, Instant::now() + Duration::from_millis(200), syns_acks_sent, ttl_increment),
                    Ok(HolePunchMsg::Ack) => send_from_ack_ack(&handle, socket, Instant::now() + Duration::from_millis(200), 0),
                    Ok(HolePunchMsg::AckAck) => send_final_ack_acks(&handle, socket, Instant::now() + Duration::from_millis(200), 0),
                    Ok(HolePunchMsg::Choose) => future::ok((socket, true)).into_boxed(),
                }
            },
        }
    })
    .into_boxed()
}

fn send_from_ack_ack(handle: &Handle, socket: WithAddress, next_timeout: Instant, ack_acks_sent: u32) -> BoxFuture<(WithAddress, bool), HolePunchError> {
    let handle = handle.clone();
    let send_msg = unwrap!(bincode::serialize(&HolePunchMsg::AckAck, bincode::Infinite));

    socket
    .send(Bytes::from(send_msg))
    .map_err(HolePunchError::SendMessage)
    .and_then(move |socket| {
        recv_from_ack_ack(&handle, socket, next_timeout, ack_acks_sent + 1)
    })
    .into_boxed()
}

fn recv_from_ack_ack(handle: &Handle, socket: WithAddress, timeout: Instant, ack_acks_sent: u32) -> BoxFuture<(WithAddress, bool), HolePunchError> {
    let handle = handle.clone();
    socket
    .with_timeout_at(timeout, &handle)
    .into_future()
    .map_err(|(e, _)| HolePunchError::ReadMessage(e))
    .and_then(move |(msg_opt, socket_timeout)| {
        let socket = socket_timeout.into_inner();
        match msg_opt {
            None => send_from_ack_ack(&handle, socket, timeout + Duration::from_millis(200), ack_acks_sent),
            Some(recv_msg) => {
                match bincode::deserialize(&recv_msg) {
                    Err(e) => {
                        future::err(HolePunchError::Deserialization(e)).into_boxed()
                    },
                    Ok(HolePunchMsg::Syn) => recv_from_ack_ack(&handle, socket, timeout, ack_acks_sent),
                    Ok(HolePunchMsg::Ack) => send_from_ack_ack(&handle, socket, Instant::now() + Duration::from_millis(200), ack_acks_sent),
                    Ok(HolePunchMsg::AckAck) => send_final_ack_acks(&handle, socket, Instant::now() + Duration::from_millis(200), ack_acks_sent),
                    Ok(HolePunchMsg::Choose) => future::ok((socket, true)).into_boxed(),
                }
            },
        }
    })
    .into_boxed()
}

fn send_final_ack_acks(handle: &Handle, socket: WithAddress, next_timeout: Instant, ack_acks_sent: u32) -> BoxFuture<(WithAddress, bool), HolePunchError> {
    let handle = handle.clone();
    if ack_acks_sent >= 5 {
        return future::ok((socket, false)).into_boxed();
    }

    Timeout::new_at(next_timeout, &handle)
    .infallible()
    .and_then(move |()| {
        send_final_ack_acks(&handle, socket, next_timeout + Duration::from_millis(200), ack_acks_sent + 1)
    })
    .into_boxed()
}

fn open_connect(socket: UdpSocket, their_addrs: HashSet<SocketAddr>, we_are_open: bool) -> BoxStream<WithAddress, HolePunchError> {
    let shared = SharedUdpSocket::share(socket);
    let mut punchers = FuturesUnordered::new();
    for addr in their_addrs {
        let with_addr = shared.with_address(addr);
        punchers.push(send_from_syn(with_addr, Instant::now() + Duration::from_millis(200), 0, 0))
    }

    future::poll_fn(|| {
        loop {
            match shared.poll()? {
                Async::Ready(Some(with_addr)) => {
                    punchers.push(send_from_ack(with_addr, Instant::now() + Duration::from_millis(200)));
                },
                Async::Ready(None) => unreachable!(),
                Async::NotReady => break,
            }
        }

        match punchers.poll()? {
            Ok(Async::Ready(Some(addr))) => {
                Ok(Async::Ready((shared, punchers, addr)))
            }
            Ok(Async::Ready(None)) => {
                if we_are_open {
                    Ok(Async::NotReady)
                } else {
                    Ok(Async::Ready(None))
                }
            },
        }
    })
}

fn choose(socket: WithAddress, chooses_sent: u32) -> BoxFuture<(UdpSocket, SocketAddr), UdpRendezvousConnectError> {
    if chooses_sent >= 5 {
        let addr = socket.remote_addr();
        return {
            unwrap!(socket.try_take())
            .map(|socket| (socket, addr))
            .infallible()
            .into_boxed()
        };
    }

    socket
    .send(HolePunchMsg::Choose)
    .map_err(UdpRendezvousConnectError::SocketSend)
    .and_then(|socket| {
        Timeout::new(Duration::from_millis(200), handle)
        .infallible()
        .and_then(move |()| {
            choose(socket, chooses_sent + 1)
        })
    })
    .into_boxed()
}

fn take_chosen(socket: WithAddress) -> BoxFuture<Option<(UdpSocket, SocketAddr)>, UdpRendezvousConnectError> {
    ...
}

#[derive(Serialize, Deserialize)]
enum HolePunchMsg {
    Syn,
    Ack,
    AckAck,
}
*/

