#![deny(rust_2018_idioms)]

#[macro_use]
extern crate diesel;

use diesel::{pg::PgConnection, prelude::*};
use snafu::{ResultExt, Snafu};

pub use config::Config;

mod config;
mod database;
mod domain;
mod flow;
mod poll_spawner;
mod pushover;
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
    let config = &*Box::leak(Box::new(config));

    let so_config =
        stack_overflow::Config::from_environment().context(UnableToConfigureStackOverflow)?;
    let so_config = &*Box::leak(Box::new(so_config));

    let pushover_config =
        pushover::Config::from_environment().context(UnableToConfigurePushover)?;

    let database_url = &config.database_url;
    let conn = PgConnection::establish(database_url).context(UnableToConnect { database_url })?;

    let (db, db_task) = database::spawn(database::Db::new(conn));

    let pushover = pushover_config.into_client();
    let notify_flow = flow::NotifyFlow::new(db.clone(), pushover);

    let (poll_spawner, poll_spawner_task) =
        poll_spawner::spawn(poll_spawner::PollSpawner::new(so_config, notify_flow));

    let mut boot_flow = flow::BootFlow::new(db.clone(), poll_spawner.clone());
    boot_flow.boot().await.context(UnableToBoot)?;

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
                    let client = reqwest::Client::new();

                    loop {
                        let ping_url = config.public_uri.clone().join("/ping").expect("TODO");
                        client.get(ping_url).send().await.expect("TODO");
                        tokio::time::delay_for(interval).await;
                    }
                })
                .await
            }
            None => futures::future::pending().await,
        }
    };

    tokio::select! {
        web_ui = web_ui => {
            web_ui.context(WebUiFailed)
        }
        caffeine_task = caffeine_task => {
            caffeine_task.context(CaffeineFailed)?;
            CaffeineExited.fail()
        }
        poll_spawner_task = poll_spawner_task => {
            poll_spawner_task.context(PollSpawnerFailed)?;
            PollSpawnerExited.fail()
        }
        db_task = db_task => {
            db_task.context(DatabaseFailed)?;
            DatabaseExited.fail()
        }
    }
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
