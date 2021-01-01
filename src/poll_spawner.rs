use crate::{
    domain::IncomingNotification,
    error::{Breaker, IsTransient},
    flow::NotifyFlow,
    stack_overflow::{self, AccessToken, AccountId},
    GlobalStackOverflowConfig,
};
use futures::{
    channel::mpsc,
    future, select,
    stream::{self, FuturesUnordered},
    SinkExt, StreamExt,
};
use snafu::{ResultExt, Snafu};
use std::{collections::HashMap, time::Duration};
use tokio::{task::JoinHandle, time};
use tracing::{trace, trace_span, warn, Instrument};

#[derive(Debug)]
pub struct PollSpawner {
    so_config: GlobalStackOverflowConfig,
    flow: NotifyFlow,
}

impl PollSpawner {
    pub fn new(so_config: GlobalStackOverflowConfig, flow: NotifyFlow) -> Self {
        Self { so_config, flow }
    }

    pub(crate) fn spawn(self) -> (PollSpawnerHandle, JoinHandle<Result<()>>) {
        let Self { so_config, flow } = self;

        let (tx, mut rx) = mpsc::channel(10);

        let task = tokio::task::spawn(async move {
            let mut pollers = HashMap::new();
            let mut children = FuturesUnordered::new();

            loop {
                select! {
                    (account_id, access_token) = rx.select_next_some() => {
                        trace!("Starting new polling task");

                        let work = poll_one_account(so_config, account_id, access_token, flow.clone());
                        let (work, abort_handle) = future::abortable(work);

                        children.push(tokio::spawn(work));

                        let old_handle = pollers.insert(account_id, abort_handle);
                        if let Some(old_handle) = old_handle {
                            old_handle.abort();
                        }
                    }

                    child = children.select_next_some() => {
                        match child.context(ChildFailed)? {
                            Ok(v) => v?,
                            Err(_) => warn!("Second worker started"),
                        }
                    }
                }
            }
        });

        (PollSpawnerHandle(tx), task)
    }
}

async fn poll_one_account(
    so_config: GlobalStackOverflowConfig,
    account_id: AccountId,
    access_token: AccessToken,
    mut flow: NotifyFlow,
) -> Result<()> {
    let s = trace_span!("poll_one_account", account_id = account_id.0);
    async {
        trace!("Starting polling");

        let so_client = stack_overflow::AuthClient::new(so_config.clone(), access_token);
        let mut breaker = Breaker::default();

        loop {
            let attempt = breaker.run(async {
                let r = so_client
                    .unread_notifications()
                    .await
                    .context(UnableToGetUnreadNotifications)?;

                let r = r
                    .into_iter()
                    .map(|n| IncomingNotification {
                        account_id,
                        text: n.body,
                    })
                    .collect();

                flow.notify(r).await.context(UnableToSendNotifications)?;

                Ok(())
            });

            if let Some(attempt) = attempt.await.context(TooManyTransientFailures)? {
                attempt?;
            }

            time::delay_for(Duration::from_secs(60)).await;
        }
    }
    .instrument(s)
    .await
}

type Pair = (AccountId, AccessToken);

#[derive(Debug, Clone)]
pub struct PollSpawnerHandle(mpsc::Sender<Pair>);

impl PollSpawnerHandle {
    pub async fn try_start_many(&mut self, registrations: Vec<Pair>) -> Option<()> {
        self.0
            .send_all(&mut stream::iter(registrations).map(Ok))
            .await
            .ok()
    }

    pub async fn start_many(&mut self, registrations: Vec<Pair>) {
        self.try_start_many(registrations)
            .await
            .expect("The actor is no longer running")
    }

    pub async fn try_start_polling(
        &mut self,
        account_id: AccountId,
        access_token: AccessToken,
    ) -> Option<()> {
        self.0.send((account_id, access_token)).await.ok()
    }

    pub async fn start_polling(&mut self, account_id: AccountId, access_token: AccessToken) {
        self.try_start_polling(account_id, access_token)
            .await
            .expect("The actor is no longer running")
    }
}

#[derive(Debug, Snafu)]
pub(crate) enum Error {
    ChildFailed { source: tokio::task::JoinError },

    UnableToGetUnreadNotifications { source: stack_overflow::Error },

    UnableToSendNotifications { source: crate::flow::Error },

    TooManyTransientFailures { source: crate::error::BreakerError },
}

impl IsTransient for Error {
    fn is_transient(&self) -> bool {
        match self {
            Self::ChildFailed { .. } => false,
            Self::UnableToGetUnreadNotifications { source } => source.is_transient(),
            Self::UnableToSendNotifications { source } => source.is_transient(),
            Self::TooManyTransientFailures { .. } => false,
        }
    }
}

type Result<T, E = Error> = std::result::Result<T, E>;
