#![deny(rust_2018_idioms)]

use crate::stack_overflow::AccountId;
use diesel::{pg::PgConnection, prelude::*};
use futures::{
    channel::mpsc::{self, Receiver},
    FutureExt, StreamExt,
};
use parking_lot::Mutex;
use snafu::{ResultExt, Snafu};
use std::{collections::HashMap, sync::Arc};
use tracing::{trace, warn};

pub use config::Config;

mod config;
mod stack_overflow;
mod web_ui;

type GlobalConfig = &'static Config;
type GlobalStackOverflowConfig = &'static stack_overflow::Config;

fn main() {
    if let Err(e) = core() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

#[tokio::main]
async fn core() -> Result<()> {
    tracing_subscriber::fmt::init();
    dotenv::dotenv().ok();

    let config = Config::from_environment().context(UnableToConfigure)?;
    let config = Box::leak(Box::new(config));

    let so_config = stack_overflow::Config::new(config.stack_overflow_client_id.clone())
        .context(UnableToConfigureStackOverflow)?;
    let so_config = Box::leak(Box::new(so_config));

    let database_url = &config.database_url;
    PgConnection::establish(database_url).context(UnableToConnect { database_url })?;

    let (_tx, rx) = mpsc::channel(10);

    let web_ui = tokio::spawn(web_ui::serve(config, so_config));
    let poll_spawner = tokio::spawn(poll_spawner(config, so_config, rx));

    tokio::select! {
        web_ui = web_ui => {
            web_ui.context(WebUiFailed)
        }
        poll_spawner = poll_spawner => {
            poll_spawner.context(PollSpawnerFailed)?;
            PollSpawnerExited.fail()
        }
    }
}

async fn poll_spawner(
    config: GlobalConfig,
    so_config: GlobalStackOverflowConfig,
    rx: Receiver<(AccountId, String)>,
) {
    trace!("poll_spawner started");
    let pollers = Arc::new(Mutex::new(HashMap::new()));
    rx.for_each(move |(id, access_token)| {
        trace!("poll_spawner starting new poller");

        let pollers = pollers.clone();
        async move {
            // `remote_handle` should kill the future when the
            // `handle` is dropped, which will happen if we replace
            // the hashmap entry for the same account.
            let (work, handle) =
                poll_one_account(config, so_config, id, access_token).remote_handle();
            tokio::spawn(work);
            let old_work = pollers.lock().insert(id, handle);

            if let Some(old_work) = old_work {
                if old_work.now_or_never().is_none() {
                    warn!("Second worker started for {:?}", id);
                }
            }
        }
    })
    .await
}

async fn poll_one_account(
    config: GlobalConfig,
    so_config: GlobalStackOverflowConfig,
    id: AccountId,
    access_token: String,
) {
    trace!("poll_one_account started for {:?}", id);

    let params = stack_overflow::UnreadParams {
        key: &*config.stack_overflow_client_key,
        site: "stackoverflow",
        access_token: &*access_token,
        filter: "default",
    };

    let r = stack_overflow::unread_notifications(so_config, &params).await;

    dbg!(&r);
}

#[derive(Debug, Snafu)]
enum Error {
    #[snafu(display("Unable to configure application"))]
    UnableToConfigure { source: config::Error },

    #[snafu(display("Unable to configure Stack Overflow integration"))]
    UnableToConfigureStackOverflow { source: stack_overflow::Error },

    #[snafu(display("Error connecting to {}", database_url))]
    UnableToConnect {
        source: diesel::ConnectionError,
        database_url: String,
    },

    #[snafu(display("The web UI failed"))]
    WebUiFailed { source: tokio::task::JoinError },

    #[snafu(display("The poll spawner exited and never should"))]
    PollSpawnerExited,

    #[snafu(display("The poll spawner failed and never should"))]
    PollSpawnerFailed { source: tokio::task::JoinError },
}

type Result<T, E = Error> = std::result::Result<T, E>;

// "threads" that
// - Serve a web front end / Oauth flow
// - polls SO, broadcasts updates
// -
