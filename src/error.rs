use snafu::{ensure, Snafu};
use std::{error::Error, future::Future};
use tracing::warn;

pub(crate) trait IsTransient {
    fn is_transient(&self) -> bool;
}

impl IsTransient for reqwest::Error {
    fn is_transient(&self) -> bool {
        self.is_request()
            && self.source().map_or(false, |s| {
                s.downcast_ref::<hyper::Error>()
                    .map_or(false, |e| e.is_incomplete_message())
            })
    }
}

#[derive(Debug, Default)]
pub(crate) struct Breaker {
    failure_count: usize,
}

impl Breaker {
    pub(crate) async fn run<F, T, E>(&mut self, f: F) -> Result<Option<Result<T, E>>, BreakerError>
    where
        F: Future<Output = Result<T, E>>,
        E: Error + IsTransient,
    {
        self.check(f.await)
    }

    pub(crate) fn check<T, E>(
        &mut self,
        r: Result<T, E>,
    ) -> Result<Option<Result<T, E>>, BreakerError>
    where
        E: Error + IsTransient,
    {
        match r {
            Ok(v) => {
                self.failure_count = 0;
                Ok(Some(Ok(v)))
            }
            Err(e) if e.is_transient() => {
                self.failure_count += 1;
                ensure!(self.failure_count < 10, BreakerContext);
                warn!(
                    "{} sequential transient errors occurred, ignoring: {}",
                    self.failure_count, e,
                );
                Ok(None)
            }
            Err(e) => Ok(Some(Err(e))),
        }
    }
}

#[derive(Debug, Snafu)]
pub(crate) struct BreakerError {}
