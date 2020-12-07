use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
use url::Url;

const OAUTH_ENTRY_URI: &str = "https://stackoverflow.com/oauth";
const OAUTH_ACCESS_TOKEN_URI: &str = "https://stackoverflow.com/oauth/access_token/json";

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct AccountId(pub i32);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct Date(i64);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct Duration(i64);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct PostId(i64);

#[derive(Debug, Deserialize)]
pub struct Wrapper<T> {
    items: Vec<T>,
}

#[derive(Debug, Deserialize)]
pub struct Notification {
    body: String,
    creation_date: Date,
    is_unread: bool,
    notification_type: NotificationType,
    post_id: Option<PostId>,
    //    site: Site,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationType {
    Generic,
    ProfileActivity,
    BountyExpired,
    BountyExpiresInOneDay,
    BadgeEarned,
    BountyExpiresInThreeDays,
    ReputationBonus,
    AccountsAssociated,
    NewPrivilege,
    PostMigrated,
    ModeratorMessage,
    RegistrationReminder,
    EditSuggested,
    SubstantiveEdit,
    BountyGracePeriodStarted,
    Other(String),
}

pub struct Config {
    client_id: String,
    unread: Url,
}

impl Config {
    pub fn new(client_id: impl Into<String>) -> Result<Self> {
        let client_id = client_id.into();
        let unread = Url::parse("https://api.stackexchange.com/2.2/me/notifications/unread")
            .context(UnableToConfigureUnreadUrl)?;

        Ok(Config { client_id, unread })
    }

    pub fn oauth_entry_url(&self, redirect_uri: &str, state: &str) -> Result<Url> {
        Url::parse_with_params(
            OAUTH_ENTRY_URI,
            &[
                ("client_id", &*self.client_id),
                ("scope", "read_inbox"),
                ("redirect_uri", redirect_uri),
                ("state", state),
            ],
        )
        .context(UnableToBuildOauthEntryUrl)
    }
}

#[derive(Debug, Serialize)]
pub struct AccessTokenRequest<'a> {
    pub client_id: &'a str,
    pub client_secret: &'a str,
    pub code: &'a str,
    pub redirect_uri: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct AccessTokenResponse {
    pub access_token: String,
    pub expires: Duration,
}

pub async fn get_access_token(params: &AccessTokenRequest<'_>) -> Result<AccessTokenResponse> {
    let client = reqwest::Client::new();
    let res = client
        .post(OAUTH_ACCESS_TOKEN_URI)
        .form(params)
        .send()
        .await
        .context(UnableToExecuteAccessTokenRequest)?;

    res.json()
        .await
        .context(UnableToDeserializeAccessTokenRequest)
}

#[derive(Debug, Serialize)]
pub struct UnreadParams<'a> {
    pub key: &'a str,
    pub site: &'a str,
    pub access_token: &'a str,
    pub filter: &'a str,
}

pub async fn unread_notifications(
    so_config: &Config,
    params: &UnreadParams<'_>,
) -> Result<Wrapper<Notification>> {
    let q = serde_urlencoded::to_string(params).context(UnableToBuildUnreadRequest)?;
    let mut unread = so_config.unread.clone();
    unread.set_query(Some(&q));

    reqwest::get(unread)
        .await
        .context(UnableToExecuteUnreadRequest)?
        .json()
        .await
        .context(UnableToDeserializeUnreadRequest)
}

#[derive(Debug, Snafu)]
pub enum Error {
    UnableToConfigureUnreadUrl {
        source: url::ParseError,
    },

    UnableToBuildOauthEntryUrl {
        source: url::ParseError,
    },

    UnableToExecuteAccessTokenRequest {
        source: reqwest::Error,
    },

    UnableToDeserializeAccessTokenRequest {
        source: reqwest::Error,
    },

    UnableToBuildUnreadRequest {
        source: serde_urlencoded::ser::Error,
    },

    UnableToExecuteUnreadRequest {
        source: reqwest::Error,
    },

    UnableToDeserializeUnreadRequest {
        source: reqwest::Error,
    },
}

type Result<T, E = Error> = std::result::Result<T, E>;
