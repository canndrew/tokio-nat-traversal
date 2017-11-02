#[macro_use]
extern crate lazy_static;
extern crate tokio_io;
extern crate tokio_core;
extern crate futures;
extern crate rust_sodium as sodium;
extern crate get_if_addrs;
#[macro_use]
extern crate unwrap;
extern crate rand;
extern crate bytes;
extern crate net2;
#[macro_use]
extern crate net_literals;
extern crate rust_sodium;
extern crate bincode;
extern crate future_utils;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate env_logger;
extern crate void;
extern crate log;
#[macro_use]
extern crate quick_error;
extern crate igd;

mod priv_prelude;
mod prelude;

mod ip_addr;
mod socket_addr;
mod tcp;
mod udp;
mod util;
mod mc;
mod igd_async;

pub use prelude::*;

const ECHO_REQ: [u8; 8] = [b'E', b'C', b'H', b'O', b'A', b'D', b'D', b'R'];

//mod subnet;
//mod stun;
//mod nat;
//mod interface;
//mod udp;

