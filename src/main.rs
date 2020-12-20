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
mod ios;
mod poll_spawner;
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
    let conn = PgConnection::establish(database_url).context(UnableToConnect { database_url })?;

    let (db, db_task) = database::spawn(database::Db::new(conn));

    let notify_flow = flow::NotifyFlow::new(db.clone());

    let (poll_spawner, poll_spawner_task) = poll_spawner::spawn(poll_spawner::PollSpawner::new(
        config,
        so_config,
        notify_flow,
    ));

    let register_flow = flow::RegisterFlow::new(config, so_config, db, poll_spawner.clone());

    let web_ui = tokio::spawn(web_ui::serve(config, so_config, register_flow));

    tokio::select! {
        web_ui = web_ui => {
            web_ui.context(WebUiFailed)
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

    #[snafu(display("The database exited and never should"))]
    DatabaseExited,

    #[snafu(display("The database failed and never should"))]
    DatabaseFailed { source: tokio::task::JoinError },
}

type Result<T, E = Error> = std::result::Result<T, E>;

// "threads" that
// - Serve a web front end / Oauth flow
// - polls SO, broadcasts updates
// -
