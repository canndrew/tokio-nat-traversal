use priv_prelude::*;

pub struct WithTimeout<F>
where
    F: Future,
{
    future: F,
    timeout: Timeout,
    error: Option<F::Error>,
}

impl<F> WithTimeout<F>
where
    F: Future,
{
    pub fn new(handle: &Handle, future: F, duration: Duration, error: F::Error) -> WithTimeout<F> {
        WithTimeout {
            future: future,
            timeout: Timeout::new(duration, handle),
            error: Some(error),
        }
    }
}

impl<F> Future for WithTimeout<F>
where
    F: Future
{
    type Item = F::Item;
    type Error = F::Error;

    fn poll(&mut self) -> Result<Async<F::Item>, F::Error> {
        match self.timeout.poll().void_unwrap() {
            Async::Ready(()) => return Err(unwrap!(self.error.take())),
            Async::NotReady => (),
        };

        self.future.poll()
    }
}

