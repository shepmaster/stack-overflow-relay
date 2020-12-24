use crate::domain::OutgoingNotification;
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
use std::env;
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
            client: reqwest::Client::new(),
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

        #[derive(Debug, Serialize)]
        struct NotifyParams<'a> {
            token: &'a str,
            user: &'a UserKey,
            title: &'a str,
            message: &'a str,
        }

        let notifications = notifications.iter().map(|n| NotifyParams {
            token: &config.token,
            user: &n.user,
            title: "Stack Overflow notification",
            message: &n.text,
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
    }
}

type Result<T, E = Error> = std::result::Result<T, E>;
