use crate::{domain::OutgoingNotification, error::IsTransient};
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
use std::env;
use tracing::{trace, trace_span, Instrument};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserKey(pub String);

#[derive(Debug, Clone)]
pub struct Config {
    token: String,
    notify_url: Url,
}

impl Config {
    pub fn from_environment() -> Result<Self> {
        let token = env::var("PUSHOVER_API_TOKEN").context(UnknownApiToken)?;

        let notify_url = Url::parse("https://api.pushover.net/1/messages.json")
            .context(UnableToConfigureNotifyUrl)?;

        Ok(Self { token, notify_url })
    }

    pub fn into_client(self) -> Client {
        Client {
            client: super::reqwest_client(),
            config: self,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Client {
    client: reqwest::Client,
    config: Config,
}

impl Client {
    pub async fn notify(&self, notifications: Vec<OutgoingNotification>) -> Result<()> {
        let Self { client, config } = self;
        let s = trace_span!("notify", count = notifications.len());

        #[derive(Debug, Serialize)]
        struct NotifyParams<'a> {
            token: &'a str,
            user: &'a UserKey,
            title: &'a str,
            message: &'a str,
            html: u8,
        }

        async {
            trace!("Performing notifications");

            let notifications = notifications.iter().map(|n| NotifyParams {
                token: &config.token,
                user: &n.user,
                title: "Stack Overflow notification",
                message: &n.text,
                html: 1,
            });

            for n in notifications {
                client
                    .post(config.notify_url.clone())
                    .query(&n)
                    .send()
                    .await
                    .context(UnableToSendNotification)?;
            }

            Ok(())
        }
        .instrument(s)
        .await
    }
}

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("PUSHOVER_API_TOKEN must be set"))]
    UnknownApiToken {
        source: env::VarError,
    },

    UnableToConfigureNotifyUrl {
        source: url::ParseError,
    },

    UnableToSendNotification {
        source: reqwest::Error,
    },
}

impl IsTransient for Error {
    fn is_transient(&self) -> bool {
        match self {
            Self::UnableToSendNotification { source } => source.is_transient(),
            _ => false,
        }
    }
}

type Result<T, E = Error> = std::result::Result<T, E>;
