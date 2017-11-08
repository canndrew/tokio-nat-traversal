use priv_prelude::*;
use std::error::Error;
use bincode;
use futures::future::Loop;
use futures::stream::FuturesUnordered;
use tokio_shared_udp_socket::{SharedUdpSocket, WithAddress};
use open_addr::{open_addr, BindPublicError};
use rendezvous_addr::{rendezvous_addr, RendezvousAddrError};
use filter_addrs::filter_addrs;
use udp::msg::UdpRendezvousMsg;

/// Errors returned by `UdpSocketExt::rendezvous_connect`.
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
    SetTtl(io::Error),
    AllAttemptsFailed(Vec<HolePunchError>, Option<BindPublicError>, Option<RendezvousAddrError>),
}

impl<C> fmt::Display for UdpRendezvousConnectError<C>
where
    C: Stream<Item=Bytes>,
    C: Sink<SinkItem=Bytes>,
    <C as Stream>::Error: Error,
    <C as Sink>::SinkError: Error,
    C: fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.description())?;
        if let UdpRendezvousConnectError::AllAttemptsFailed(ref v, ref bind_public, ref rendezvous) = *self {
            if let Some(ref bind_public) = *bind_public {
                write!(f, "attempt to create publicly-connectable socket gave error: {}. ", bind_public)?;
            }
            if let Some(ref rendezvous) = *rendezvous {
                write!(f, "attempt to create rendezvous socket gave: {}. ", rendezvous)?;
            }
            
            let num_errors = v.len();
            write!(f, "there were {} failed hole-punch attempts:", num_errors)?;
            for (i, error) in v.iter().enumerate() {
                write!(f, ". [{} of {}] {}", i, num_errors, error)?;
            }

        } else if let Some(error) = self.cause() {
            write!(f, ". {}", error)?;
        }
        Ok(())
    }
}

impl<C> Error for UdpRendezvousConnectError<C>
where
    C: Stream<Item=Bytes>,
    C: Sink<SinkItem=Bytes>,
    <C as Stream>::Error: Error,
    <C as Sink>::SinkError: Error,
    C: fmt::Debug + 'static,
{
    fn cause(&self) -> Option<&Error> {
        use self::UdpRendezvousConnectError::*;

        match *self {
            Bind(ref e) | Rebind(ref e) | IfAddrs(ref e) | SocketWrite(ref e) | SetTtl(ref e) => Some(e),
            ChannelClosed => None,
            ChannelRead(ref e) => Some(e),
            ChannelWrite(ref e) => Some(e),
            DeserializeMsg(ref e) => Some(e),
            AllAttemptsFailed(..) => None,
        }
    }

    fn description(&self) -> &str {
        use self::UdpRendezvousConnectError::*;

        match *self {
            Bind(..) => "error binding to local port",
            Rebind(..) => "error rebinding to same reusably-bound port",
            IfAddrs(..) => "error getting network interface addresses",
            ChannelClosed => "rendezvous channel closed unexpectedly",
            ChannelRead(..) => "error reading from rendezvous channel",
            ChannelWrite(..) => "error writing to rendezvous channel",
            DeserializeMsg(..) => "error deserializing message from rendezvous channel",
            SocketWrite(..) => "error writing to socket",
            SetTtl(..) => "error setting ttl value on socket",
            AllAttemptsFailed(..) => "all attempts to contact the remote peer failed",
        }
    }
}

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

    /// Perform a UDP rendezvous connection to another peer. Both peers must call this
    /// simultaneously and `channel` must provide a channel through which the peers can communicate
    /// out-of-band.
    fn rendezvous_connect<C>(channel: C, handle: &Handle) -> BoxFuture<(UdpSocket, SocketAddr), UdpRendezvousConnectError<C>>
    where
        C: Stream<Item=Bytes>,
        C: Sink<SinkItem=Bytes>,
        <C as Stream>::Error: fmt::Debug,
        <C as Sink>::SinkError: fmt::Debug,
        C: 'static;
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

    fn rendezvous_connect<C>(channel: C, handle: &Handle) -> BoxFuture<(UdpSocket, SocketAddr), UdpRendezvousConnectError<C>>
    where
        C: Stream<Item=Bytes>,
        C: Sink<SinkItem=Bytes>,
        <C as Stream>::Error: fmt::Debug,
        <C as Sink>::SinkError: fmt::Debug,
        C: 'static,
    {
        let handle0 = handle.clone();
        let handle1 = handle.clone();
        let (our_pk, _sk) = crypto::box_::gen_keypair();

        UdpSocket::bind_public(&addr!("0.0.0.0:0"), handle)
        .then(move |res| match res {
            Ok((socket, public_addr)) => {
                let try = || {
                    let mut our_addrs = {
                        socket
                        .expanded_local_addrs()
                        .map_err(UdpRendezvousConnectError::IfAddrs)?
                    };
                    our_addrs.push(public_addr);
                    let our_addrs = our_addrs.into_iter().collect::<HashSet<_>>();
                    let msg = UdpRendezvousMsg::Init {
                        enc_pk: our_pk,
                        open_addrs: our_addrs.clone(),
                        rendezvous_addrs: Vec::new(),
                    };

                    Ok({
                        exchange_msgs(channel, msg)
                        .map(move |their_msg| {
                            let UdpRendezvousMsg::Init {
                                enc_pk: their_pk,
                                open_addrs: their_open_addrs,
                                rendezvous_addrs: _their_rendezvous_addrs,
                            } = their_msg;

                            let their_open_addrs = filter_addrs(&our_addrs, &their_open_addrs);
                            let incoming = {
                                open_connect(&handle0, socket, their_open_addrs, true)
                                .into_boxed()
                            };
                            (their_pk, incoming, None, None)
                        })
                    })
                };
                future::result(try()).flatten().into_boxed()
            },
            Err(bind_public_error) => {
                let try = || {
                    let listen_socket = {
                        UdpSocket::bind_reusable(&addr!("0.0.0.0:0"), &handle0)
                        .map_err(UdpRendezvousConnectError::Bind)?
                    };
                    let our_addrs = {
                        listen_socket
                        .expanded_local_addrs()
                        .map_err(UdpRendezvousConnectError::IfAddrs)?
                    };
                    let our_addrs = our_addrs.into_iter().collect::<HashSet<_>>();

                    Ok({
                        let handle2 = handle0.clone();
                        future::loop_fn(Vec::new(), move |mut sockets| {
                            if sockets.len() == 6 {
                                return future::ok(Loop::Break((sockets, None))).into_boxed();
                            }
                            let try = || {
                                let ttl_increments = 2 << sockets.len();
                                let socket = {
                                    UdpSocket::bind_reusable(&addr!("0.0.0.0:0"), &handle0)
                                    .map_err(UdpRendezvousConnectError::Rebind)
                                }?;
                                let bind_addr = {
                                    socket
                                    .local_addr()
                                    .map_err(UdpRendezvousConnectError::Rebind)?
                                };
                                socket.set_ttl(ttl_increments).map_err(UdpRendezvousConnectError::SetTtl)?;
                                Ok({
                                    rendezvous_addr(Protocol::Udp, &bind_addr, &handle0)
                                    .then(move |res| {
                                        match res {
                                            Ok(addr) => {
                                                sockets.push((socket, addr));
                                                Ok(Loop::Continue(sockets))
                                            },
                                            Err(err) => {
                                                Ok(Loop::Break((sockets, Some(err))))
                                            },
                                        }
                                    })
                                })
                            };
                            future::result(try()).flatten().into_boxed()
                        })
                        .and_then(move |(sockets, rendezvous_error_opt)| {
                            let (sockets, rendezvous_addrs) = sockets.into_iter().unzip::<_, _, Vec<_>, _>();
                            let msg = UdpRendezvousMsg::Init {
                                enc_pk: our_pk,
                                open_addrs: our_addrs.clone(),
                                rendezvous_addrs: rendezvous_addrs,
                            };
                            exchange_msgs(channel, msg)
                            .and_then(move |their_msg| {
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
                                    socket.set_ttl(ttl_increment).map_err(UdpRendezvousConnectError::SetTtl)?;
                                    let shared = SharedUdpSocket::share(socket);
                                    let with_addr = shared.with_address(their_addr);
                                    let timeout = Instant::now() + Duration::from_millis(200);
                                    let puncher = send_from_syn(&handle2, with_addr, timeout, 0, ttl_increment);
                                    punchers.push(puncher);
                                }

                                let their_open_addrs = filter_addrs(&our_addrs, &their_open_addrs);

                                let incoming = {
                                    open_connect(&handle2, listen_socket, their_open_addrs, false)
                                    .select(punchers)
                                    .into_boxed()
                                };
                                Ok((their_pk, incoming, Some(bind_public_error), rendezvous_error_opt))
                            })
                        })
                    })
                };
                future::result(try()).flatten().into_boxed()
            },
        })
        .and_then(move |(their_pk, incoming, bind_public_error_opt, rendezvous_error_opt)| {
            if our_pk > their_pk {
                incoming
                .and_then(|(socket, chosen)| {
                    if chosen {
                        return Err(HolePunchError::UnexpectedMessage);
                    }
                    Ok(socket)
                })
                .first_ok()
                .map_err(|v| {
                    UdpRendezvousConnectError::AllAttemptsFailed(v, bind_public_error_opt, rendezvous_error_opt)
                })
                .and_then(move |socket| {
                    choose(&handle1, socket, 0)
                })
                .into_boxed()
            } else {
                incoming
                .map(|(socket, chosen)| {
                    if chosen {
                        return got_chosen(socket)
                    }
                    take_chosen(socket)
                })
                .buffer_unordered(256)
                .filter_map(|opt| opt)
                .first_ok()
                .map_err(|v| {
                    UdpRendezvousConnectError::AllAttemptsFailed(v, bind_public_error_opt, rendezvous_error_opt)
                })
                .into_boxed()
            }
        })
        .into_boxed()
    }
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

quick_error! {
    /// Error resulting from a single failed hole-punching attempt.
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
        UnexpectedMessage {
            description("received unexpected hole-punch message type")
        }
        SetTtl(e: io::Error) {
            description("error setting ttl on socket")
            display("error setting ttl on socket: {}", e)
            cause(e)
        }
    }
}

// send a HolePunchMsg::Syn to the other peer, and complete hole-punching from there.
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
            Ok(recv_from_syn(&handle, socket, next_timeout, syns_acks_sent, ttl_increment))
        };
        future::result(try().map_err(HolePunchError::SetTtl)).flatten()
    })
    .into_boxed()
}

// we have just sent a syn packet, listen for incoming packets until it's time to send another one.
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
                        warn!("error deserializing packet from peer: {}", e);
                        recv_from_syn(&handle, socket, timeout, syns_acks_sent, ttl_increment)
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

// send an ack to the other peer, then complete hole-punching from there.
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
            Ok(recv_from_ack(&handle, socket, next_timeout, syns_acks_sent, ttl_increment))
        };
        future::result(try().map_err(HolePunchError::SetTtl)).flatten()
    })
    .into_boxed()
}

// we have just sent an ack to the peer, listen for incoming packets until it's time to send
// another one.
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
                        warn!("error deserializing packet from peer: {}", e);
                        recv_from_ack(&handle, socket, timeout, syns_acks_sent, ttl_increment)
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

// send an ack-ack to the remote peer, then finish hole punching.
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

// we have just sent an ack-ack to the remote peer, listen for incoming packets until it's time to
// send another one.
fn recv_from_ack_ack(handle: &Handle, socket: WithAddress, timeout: Instant, ack_acks_sent: u32) -> BoxFuture<(WithAddress, bool), HolePunchError> {
    let handle = handle.clone();
    socket
    .with_timeout_at(timeout, &handle)
    .into_future()
    .map_err(|(e, _)| HolePunchError::ReadMessage(e))
    .and_then(move |(msg_opt, socket_timeout)| {
        let socket = socket_timeout.into_inner();
        match msg_opt {
            // TODO: broooooken
            None => send_from_ack_ack(&handle, socket, timeout + Duration::from_millis(200), ack_acks_sent),
            Some(recv_msg) => {
                match bincode::deserialize(&recv_msg) {
                    Err(e) => {
                        warn!("error deserializing packet from peer: {}", e);
                        recv_from_ack_ack(&handle, socket, timeout, ack_acks_sent)
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

// send the last few ack-acks (we send it several times just in case packets get dropped)
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

// Perform a connect where one of the peers has an open port.
fn open_connect(
    handle: &Handle,
    socket: UdpSocket,
    their_addrs: HashSet<SocketAddr>,
    we_are_open: bool,
) -> BoxStream<(WithAddress, bool), HolePunchError> {
    let mut shared = SharedUdpSocket::share(socket);
    let mut punchers = FuturesUnordered::new();
    for addr in their_addrs {
        let with_addr = shared.with_address(addr);
        punchers.push(send_from_syn(handle, with_addr, Instant::now() + Duration::from_millis(200), 0, 0))
    }

    let handle = handle.clone();
    stream::poll_fn(move || {
        loop {
            match shared.poll() {
                Ok(Async::Ready(Some(with_addr))) => {
                    punchers.push(send_from_ack(&handle, with_addr, Instant::now() + Duration::from_millis(200), 0, 0));
                },
                Ok(Async::Ready(None)) => break,
                Ok(Async::NotReady) => break,
                Err(e) => {
                    error!("error reading from shared socket: {}", e);
                    break
                },
            }
        }

        match punchers.poll()? {
            Async::Ready(Some(x)) => {
                Ok(Async::Ready(Some(x)))
            }
            Async::Ready(None) => {
                if we_are_open {
                    Ok(Async::NotReady)
                } else {
                    Ok(Async::Ready(None))
                }
            },
            Async::NotReady => Ok(Async::NotReady),
        }
    })
    .into_boxed()
}

// choose the given socket+address to be the socket+address we return successfully with.
fn choose<C>(
    handle: &Handle,
    socket: WithAddress,
    chooses_sent: u32,
) -> BoxFuture<(UdpSocket, SocketAddr), UdpRendezvousConnectError<C>>
where
    C: Stream<Item=Bytes>,
    C: Sink<SinkItem=Bytes>,
    <C as Stream>::Error: fmt::Debug,
    <C as Sink>::SinkError: fmt::Debug,
    C: 'static,
{
    if chooses_sent >= 5 {
        let addr = socket.remote_addr();
        return {
            unwrap!(socket.try_take().map_err(|_| "another task tried to take the socket?"))
            .map(move |socket| (socket, addr))
            .infallible()
            .into_boxed()
        };
    }

    let handle = handle.clone();
    let msg = unwrap!(bincode::serialize(&HolePunchMsg::Choose, bincode::Infinite));

    socket
    .send(Bytes::from(msg))
    .map_err(UdpRendezvousConnectError::SocketWrite)
    .and_then(move |socket| {
        Timeout::new(Duration::from_millis(200), &handle)
        .infallible()
        .and_then(move |()| {
            choose(&handle, socket, chooses_sent + 1)
        })
    })
    .into_boxed()
}

// listen on the socket to if the peer sends us a HolePunchMsg::Choose to indicate that they're
// choosing this socket+address to communicate with us.
fn take_chosen(socket: WithAddress) -> BoxFuture<Option<(UdpSocket, SocketAddr)>, HolePunchError> {
    socket
    .into_future()
    .map_err(|(e, _)| HolePunchError::ReadMessage(e))
    .and_then(|(msg_opt, socket)| {
        match msg_opt {
            None => future::ok(None).into_boxed(),
            Some(msg) => {
                match bincode::deserialize(&msg) {
                    Err(e) => {
                        warn!("error deserializing packet from peer: {}", e);
                        take_chosen(socket)
                    },
                    Ok(HolePunchMsg::Choose) => {
                        got_chosen(socket)
                    },
                    Ok(..) => take_chosen(socket)
                }
            },
        }
    })
    .into_boxed()
}

// this socket got chosen by the remote peer. Return success with it.
fn got_chosen(socket: WithAddress) -> BoxFuture<Option<(UdpSocket, SocketAddr)>, HolePunchError> {
    let addr = socket.remote_addr();
    match socket.try_take() {
        Ok(try_take) => {
            try_take
            .infallible()
            .map(move |socket| Some((socket, addr)))
            .into_boxed()
        }
        Err(..) => future::ok(None).into_boxed()
    }
}

// exchange rendezvous messages along the channel
fn exchange_msgs<C>(channel: C, msg: UdpRendezvousMsg) -> BoxFuture<UdpRendezvousMsg, UdpRendezvousConnectError<C>>
where
    C: Stream<Item=Bytes>,
    C: Sink<SinkItem=Bytes>,
    <C as Stream>::Error: fmt::Debug,
    <C as Sink>::SinkError: fmt::Debug,
    C: 'static,
{
    let msg = unwrap!(bincode::serialize(&msg, bincode::Infinite));
    channel
    .send(Bytes::from(msg))
    .map_err(UdpRendezvousConnectError::ChannelWrite)
    .and_then(|channel| {
        channel
        .map_err(UdpRendezvousConnectError::ChannelRead)
        .next_or_else(|| UdpRendezvousConnectError::ChannelClosed)
        .and_then(|(msg, _channel)| {
            bincode::deserialize(&msg)
            .map_err(UdpRendezvousConnectError::DeserializeMsg)
        })
    })
    .into_boxed()
}

#[derive(Serialize, Deserialize)]
enum HolePunchMsg {
    Syn,
    Ack,
    AckAck,
    Choose,
}

