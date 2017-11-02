use priv_prelude::*;
use tokio_core;

pub struct Timeout {
    inner: tokio_core::reactor::Timeout,
}

impl Timeout {
    pub fn new(duration: Duration, handle: &Handle) -> Timeout {
        Timeout {
            inner: unwrap!(tokio_core::reactor::Timeout::new(duration, handle)),
        }
    }

    /*
    pub fn reset(&mut self, at: Instant) {
        self.inner.reset(at)
    }
    */
}

impl Future for Timeout {
    type Item = ();
    type Error = Void;

    fn poll(&mut self) -> Result<Async<()>, Void> {
        Ok(unwrap!(self.inner.poll()))
    }
}

