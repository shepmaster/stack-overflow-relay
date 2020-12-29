use crate::{
    domain::IncomingNotification,
    flow::NotifyFlow,
    stack_overflow::{self, AccessToken, AccountId},
    GlobalStackOverflowConfig,
};
use futures::{future::RemoteHandle, FutureExt};
use std::{collections::HashMap, time::Duration};
use tokio::time;
use tracing::{trace, trace_span, warn, Instrument};

#[derive(Debug)]
pub struct PollSpawner {
    so_config: GlobalStackOverflowConfig,
    pollers: HashMap<AccountId, RemoteHandle<()>>,
    flow: NotifyFlow,
}

// Not part of actor API
impl PollSpawner {
    pub fn new(so_config: GlobalStackOverflowConfig, flow: NotifyFlow) -> Self {
        Self {
            so_config,
            pollers: Default::default(),
            flow,
        }
    }
}

// Actor API
#[alictor::alictor(kind = async)]
impl PollSpawner {
    fn start_many(&mut self, registrations: Vec<(AccountId, AccessToken)>) {
        for (account_id, access_token) in registrations {
            self.start_polling(account_id, access_token);
        }
    }

    fn start_polling(&mut self, account_id: AccountId, access_token: AccessToken) {
        let s = trace_span!("start_polling", account_id = account_id.0);
        let _s = s.enter();
        trace!("Starting new polling task");

        let Self {
            so_config,
            pollers,
            flow,
        } = self;

        // `remote_handle` should kill the future when the
        // `handle` is dropped, which will happen if we replace
        // the hashmap entry for the same account.
        let (work, handle) =
            poll_one_account(so_config, account_id, access_token, flow.clone()).remote_handle();
        tokio::spawn(work);
        let old_work = pollers.insert(account_id, handle);

        if let Some(old_work) = old_work {
            if old_work.now_or_never().is_none() {
                warn!("Second worker started");
            }
        }
    }
}

async fn poll_one_account(
    so_config: GlobalStackOverflowConfig,
    account_id: AccountId,
    access_token: AccessToken,
    mut flow: NotifyFlow,
) {
    let s = trace_span!("poll_one_account", account_id = account_id.0);
    async {
        trace!("Starting polling");

        let so_client = stack_overflow::AuthClient::new(so_config.clone(), access_token);

        loop {
            let r = so_client.unread_notifications().await.expect("TODO");

            let r = r
                .into_iter()
                .map(|n| IncomingNotification {
                    account_id,
                    text: n.body,
                })
                .collect();

            flow.notify(r).await.expect("TODO");

            time::delay_for(Duration::from_secs(60)).await;
        }
    }
    .instrument(s)
    .await
}
