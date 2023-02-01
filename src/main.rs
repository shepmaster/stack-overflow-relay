#![deny(rust_2018_idioms)]

#[macro_use]
extern crate diesel;

use diesel::{pg::PgConnection, prelude::*};
use snafu::{ResultExt, Snafu};

pub use config::Config;

mod config;
mod database;
mod domain;
mod error;
mod flow;
mod poll_spawner;
mod pushover;
mod stack_overflow;
mod web_ui;

type GlobalConfig = &'static Config;
type GlobalStackOverflowConfig = &'static stack_overflow::Config;

fn main() {
    if let Err(e) = core() {
        eprintln!("Error: {e}");
        let mut e = &e as &dyn std::error::Error;
        while let Some(e2) = e.source() {
            e = e2;
            eprintln!("     : {e}");
        }

        std::process::exit(1);
    }
}

#[tokio::main]
async fn core() -> Result<()> {
    tracing_subscriber::fmt::init();
    dotenv::dotenv().ok();

    let config = Config::from_environment().context(UnableToConfigureSnafu)?;
    let config = &*Box::leak(Box::new(config));

    let so_config =
        stack_overflow::Config::from_environment().context(UnableToConfigureStackOverflowSnafu)?;
    let so_config = &*Box::leak(Box::new(so_config));

    let pushover_config =
        pushover::Config::from_environment().context(UnableToConfigurePushoverSnafu)?;

    let database_url = &config.database_url;
    let conn =
        PgConnection::establish(database_url).context(UnableToConnectSnafu { database_url })?;

    let (db, db_task) = database::Db::new(conn).spawn();

    let pushover = pushover_config.into_client();
    let notify_flow = flow::ProxyNotificationsFlow::new(so_config, db.clone(), pushover);

    let (poll_spawner, poll_spawner_task) = poll_spawner::PollSpawner::new(notify_flow).spawn();

    let mut boot_flow = flow::BootFlow::new(db.clone(), poll_spawner.clone());
    boot_flow.boot().await.context(UnableToBootSnafu)?;

    let register_flow = flow::RegisterFlow::new(so_config, db.clone(), poll_spawner.clone());
    let set_pushover_user_flow = flow::SetPushoverUserFlow::new(db);

    let web_ui = tokio::spawn(web_ui::serve(
        config,
        so_config,
        register_flow,
        set_pushover_user_flow,
    ));

    let caffeine_task = async {
        match config.caffeine_interval {
            Some(interval) => {
                tokio::spawn(async move {
                    let client = reqwest_client();

                    loop {
                        let ping_url = config.public_uri.clone().join("/ping").expect("TODO");
                        client.get(ping_url).send().await.expect("TODO");
                        tokio::time::sleep(interval).await;
                    }
                })
                .await
            }
            None => futures::future::pending().await,
        }
    };

    tokio::select! {
        web_ui = web_ui => {
            web_ui.context(WebUiFailedSnafu)
        }
        caffeine_task = caffeine_task => {
            caffeine_task.context(CaffeineFailedSnafu)?;
            CaffeineExitedSnafu.fail()
        }
        poll_spawner_task = poll_spawner_task => {
            poll_spawner_task.context(PollSpawnerFailedSnafu)?.context(PollSpawnerErroredSnafu)?;
            PollSpawnerExitedSnafu.fail()
        }
        db_task = db_task => {
            db_task.context(DatabaseFailedSnafu)?;
            DatabaseExitedSnafu.fail()
        }
    }
}

const USER_AGENT: &str = concat!(
    "stack-overflow-relay (version:",
    env!("VERGEN_GIT_SHA"),
    ")"
);

fn reqwest_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .expect("Unable to configure reqwest::Client")
}

#[derive(Debug, Snafu)]
enum Error {
    #[snafu(display("Unable to configure application"))]
    UnableToConfigure { source: config::Error },

    #[snafu(display("Unable to configure Stack Overflow integration"))]
    UnableToConfigureStackOverflow { source: stack_overflow::Error },

    #[snafu(display("Unable to configure Pushover integration"))]
    UnableToConfigurePushover { source: pushover::Error },

    #[snafu(display("Error connecting to {}", database_url))]
    UnableToConnect {
        source: diesel::ConnectionError,
        database_url: String,
    },

    #[snafu(display("Unable to boot background workers"))]
    UnableToBoot { source: flow::Error },

    #[snafu(display("The web UI failed"))]
    WebUiFailed { source: tokio::task::JoinError },

    #[snafu(display("The poll spawner exited and never should"))]
    PollSpawnerExited,

    #[snafu(display("The poll spawner failed and never should"))]
    PollSpawnerFailed { source: tokio::task::JoinError },

    #[snafu(display("The poll spawner errored and never should"))]
    PollSpawnerErrored { source: poll_spawner::Error },

    #[snafu(display("The database exited and never should"))]
    DatabaseExited,

    #[snafu(display("The database failed and never should"))]
    DatabaseFailed { source: tokio::task::JoinError },

    #[snafu(display("The caffeine task exited and never should"))]
    CaffeineExited,

    #[snafu(display("The caffeine task failed and never should"))]
    CaffeineFailed { source: tokio::task::JoinError },
}

type Result<T, E = Error> = std::result::Result<T, E>;

// "threads" that
// - Serve a web front end / Oauth flow
// - polls SO, broadcasts updates
// -
