use serde::{Deserialize, Serialize};
use snafu::{OptionExt, ResultExt, Snafu};
use url::Url;

const OAUTH_ENTRY_URI: &str = "https://stackoverflow.com/oauth";
const OAUTH_ACCESS_TOKEN_URI: &str = "https://stackoverflow.com/oauth/access_token/json";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccessToken(pub String);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct AccountId(pub i32);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct UserId(pub i32);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct Date(i64);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct Duration(i64);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct PostId(i64);

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Wrapper<T> {
    Error(ApiError),
    Success(ApiSuccess<T>),
}

impl<T> Wrapper<T> {
    fn into_result(self) -> Result<ApiSuccess<T>, ApiError> {
        match self {
            Wrapper::Error(e) => Err(e),
            Wrapper::Success(s) => Ok(s),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ApiSuccess<T> {
    items: Vec<T>,
    backoff: Option<i32>,
    has_more: bool,
    #[serde(flatten)]
    quota: Quota,
}

impl<T> ApiSuccess<T> {
    fn into_singleton(mut self) -> Option<T> {
        let v = self.items.pop();
        v.filter(|_| self.items.is_empty())
    }
}

#[derive(Debug, Deserialize)]
pub struct Quota {
    #[serde(rename = "quota_max")]
    max: i32,
    #[serde(rename = "quota_remaining")]
    remaining: i32,
}

#[derive(Debug, Snafu, Deserialize)]
pub struct ApiError {
    #[serde(rename = "error_id")]
    id: i32,
    #[serde(rename = "error_message")]
    message: String,
    #[serde(rename = "error_name")]
    name: String,
}

#[allow(unused)]
impl ApiError {
    const BAD_PARAMETER: i32 = 400;
    const ACCESS_TOKEN_REQUIRED: i32 = 401;
    const INVALID_ACCESS_TOKEN: i32 = 402;
    const ACCESS_DENIED: i32 = 403;
    const NO_METHOD: i32 = 404;
    const KEY_REQUIRED: i32 = 405;
    const ACCESS_TOKEN_COMPROMISED: i32 = 406;
    const WRITE_FAILED: i32 = 407;
    const DUPLICATE_REQUEST: i32 = 409;
    const INTERNAL_ERROR: i32 = 500;
    const THROTTLE_VIOLATION: i32 = 502;
    const TEMPORARILY_UNAVAILABLE: i32 = 503;
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

#[derive(Debug, Deserialize)]
pub struct User {
    pub account_id: AccountId,
    pub user_id: UserId,
}

#[derive(Debug)]
pub struct Config {
    client_id: String,
    unread: Url,
    current_user: Url,
}

impl Config {
    pub fn new(client_id: impl Into<String>) -> Result<Self> {
        let client_id = client_id.into();
        let unread = Url::parse("https://api.stackexchange.com/2.2/me/notifications/unread")
            .context(UnableToConfigureUnreadUrl)?;
        let current_user = Url::parse("https://api.stackexchange.com/2.2/me")
            .context(UnableToConfigureCurrentUserUrl)?;

        Ok(Config {
            client_id,
            unread,
            current_user,
        })
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
    pub access_token: AccessToken,
    pub expires: Duration,
}

#[derive(Debug, Serialize)]
pub struct CurrentUserParams<'a> {
    pub key: &'a str,
    pub site: &'a str,
    pub access_token: &'a AccessToken,
    pub filter: &'a str,
}

pub async fn current_user(so_config: &Config, params: &CurrentUserParams<'_>) -> Result<User> {
    let q = serde_urlencoded::to_string(params).context(UnableToBuildCurrentUserRequest)?;
    let mut current = so_config.current_user.clone();
    current.set_query(Some(&q));

    reqwest::get(current)
        .await
        .context(UnableToExecuteCurrentUserRequest)?
        .json::<Wrapper<User>>()
        .await
        .context(UnableToDeserializeCurrentUserRequest)?
        .into_result()
        .context(CurrentUserRequestFailed)?
        .into_singleton()
        .context(CurrentUserRequestDidNotHaveOneResult)
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
    pub access_token: &'a AccessToken,
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

    UnableToConfigureCurrentUserUrl {
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

    UnableToBuildCurrentUserRequest {
        source: serde_urlencoded::ser::Error,
    },

    UnableToExecuteCurrentUserRequest {
        source: reqwest::Error,
    },

    UnableToDeserializeCurrentUserRequest {
        source: reqwest::Error,
    },

    CurrentUserRequestFailed {
        source: ApiError,
    },

    CurrentUserRequestDidNotHaveOneResult {},

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
