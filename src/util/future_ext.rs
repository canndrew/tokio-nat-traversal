use priv_prelude::*;
use util::with_timeout::WithTimeout;

pub trait UtilFutureExt: Future {
    fn with_timeout(self, handle: &Handle, duration: Duration, error: Self::Error) -> WithTimeout<Self>
    where
        Self: Sized
    {
        WithTimeout::new(handle, self, duration, error)
    }
}

impl<F> UtilFutureExt for F
where F: Future
{
}

