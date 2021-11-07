use crate::error::IsTransient;
use futures::{future::BoxFuture, FutureExt};
use serde::{Deserialize, Serialize};
use snafu::{OptionExt, ResultExt, Snafu};
use std::env;
use tracing::{trace, trace_span, Instrument};
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
pub struct Date(pub i64);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct Duration(pub i64);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Deserialize)]
pub struct PostId(pub i64);

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
    pub items: Vec<T>,
    pub backoff: Option<i32>,
    pub has_more: bool,
    #[serde(flatten)]
    pub quota: Quota,
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
    pub max: i32,
    #[serde(rename = "quota_remaining")]
    pub remaining: i32,
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
    pub body: String,
    pub creation_date: Date,
    pub is_unread: bool,
    pub notification_type: NotificationType,
    pub post_id: Option<PostId>,
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
pub struct Inbox {
    pub body: String,
    pub creation_date: Date,
    pub is_unread: bool,
    pub item_type: InboxType,
    pub post_id: Option<PostId>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxType {
    Comment,
    ChatMessage,
    NewAnswer,
    CareersMessage,
    CareersInvitations,
    MetaQuestion,
    PostNotice,
    ModeratorMessage,
    QuestionUpdate,
    FollowedPostActivity,
    SubcommunityEndorsement,
    SubcommunityLeaderboard,
    Other(String),
}

#[derive(Debug, Deserialize)]
pub struct User {
    pub account_id: AccountId,
    pub user_id: UserId,
}

//--

#[derive(Debug, Clone)]
pub struct Config {
    client_id: String,
    client_secret: String,
    client_key: String,
    unread_notifications: Url,
    unread_inbox: Url,
    current_user: Url,
}

impl Config {
    pub fn from_environment() -> Result<Self> {
        let client_id = env::var("STACK_OVERFLOW_CLIENT_ID").context(UnknownClientId)?;
        let client_secret =
            env::var("STACK_OVERFLOW_CLIENT_SECRET").context(UnknownClientSecret)?;
        let client_key = env::var("STACK_OVERFLOW_CLIENT_KEY").context(UnknownClientKey)?;

        Self::new(client_id, client_secret, client_key)
    }

    fn new(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        client_key: impl Into<String>,
    ) -> Result<Self> {
        let client_id = client_id.into();
        let client_secret = client_secret.into();
        let client_key = client_key.into();
        let unread_notifications =
            Url::parse("https://api.stackexchange.com/2.2/me/notifications/unread")
                .context(UnableToConfigureUnreadNotificationsUrl)?;
        let unread_inbox = Url::parse("https://api.stackexchange.com/2.3/me/inbox/unread")
            .context(UnableToConfigureUnreadInboxUrl)?;

        let current_user = Url::parse("https://api.stackexchange.com/2.2/me")
            .context(UnableToConfigureCurrentUserUrl)?;

        Ok(Config {
            client_id,
            client_secret,
            client_key,
            unread_notifications,
            unread_inbox,
            current_user,
        })
    }

    pub fn oauth_entry_url(&self, redirect_uri: &str, state: &str) -> Result<Url> {
        Url::parse_with_params(
            OAUTH_ENTRY_URI,
            &[
                ("client_id", &*self.client_id),
                ("scope", "read_inbox,no_expiry"),
                ("redirect_uri", redirect_uri),
                ("state", state),
            ],
        )
        .context(UnableToBuildOauthEntryUrl)
    }

    pub fn into_unauth_client(self) -> UnauthClient {
        UnauthClient {
            client: super::reqwest_client(),
            config: self,
        }
    }
}

const SITE_STACKOVERFLOW: &str = "stackoverflow";
const FILTER_DEFAULT: &str = "default";
const FILTER_WITH_BODY: &str = "withbody";

pub struct UnauthClient {
    client: reqwest::Client,
    config: Config,
}

impl UnauthClient {
    pub fn into_auth_client(self, access_token: AccessToken) -> AuthClient {
        let Self { client, config } = self;
        AuthClient {
            client,
            auth_config: AuthConfig {
                access_token,
                config,
            },
        }
    }

    pub async fn get_access_token(
        &self,
        oauth_code: &str,
        redirect_uri: &str,
    ) -> Result<AccessToken> {
        let Self { client, config } = self;

        #[derive(Debug, Serialize)]
        struct AccessTokenParams<'a> {
            client_id: &'a str,
            client_secret: &'a str,
            code: &'a str,
            redirect_uri: &'a str,
        }

        #[derive(Debug, Deserialize)]
        struct AccessTokenResponse {
            access_token: AccessToken,
            expires: Option<Duration>,
        }

        let params = AccessTokenParams {
            client_id: &config.client_id,
            client_secret: &config.client_secret,
            code: oauth_code,
            redirect_uri,
        };

        let res = client
            .post(OAUTH_ACCESS_TOKEN_URI)
            .form(&params)
            .send()
            .await
            .context(UnableToExecuteAccessTokenRequest)?
            .ensure_success()
            .await
            .context(AccessTokenRequestRejected)?
            .json::<AccessTokenResponse>()
            .await
            .context(UnableToDeserializeAccessTokenRequest)?;

        Ok(res.access_token)
    }
}

#[derive(Debug, Serialize)]
struct AuthParams<'a, T> {
    key: &'a str,
    site: &'a str,
    access_token: &'a AccessToken,
    #[serde(flatten)]
    request_params: T,
}

#[derive(Debug, Clone)]
pub struct AuthClient {
    client: reqwest::Client,
    auth_config: AuthConfig,
}

#[derive(Debug, Clone)]
struct AuthConfig {
    access_token: AccessToken,
    config: Config,
}

impl AuthConfig {
    fn auth_params<T>(&self, request_params: T) -> AuthParams<'_, T> {
        let Self {
            config,
            access_token,
            ..
        } = self;

        AuthParams {
            key: &config.client_key,
            access_token,
            site: SITE_STACKOVERFLOW,
            request_params,
        }
    }
}

impl AuthClient {
    pub fn new(config: Config, access_token: AccessToken) -> Self {
        Self {
            client: super::reqwest_client(),
            auth_config: AuthConfig {
                access_token,
                config,
            },
        }
    }

    pub fn access_token(&self) -> &AccessToken {
        &self.auth_config.access_token
    }

    pub async fn current_user(&self) -> Result<User, CurrentUserError> {
        let s = trace_span!("current_user");

        async {
            let Self {
                client,
                auth_config,
            } = self;

            #[derive(Debug, Serialize)]
            struct CurrentUserParams<'a> {
                filter: &'a str,
            }

            let params = auth_config.auth_params(CurrentUserParams {
                filter: FILTER_DEFAULT,
            });

            client
                .get(auth_config.config.current_user.clone())
                .query(&params)
                .send()
                .await
                .context(UnableToExecuteRequest)?
                .ensure_success()
                .await
                .context(RequestRejected)?
                .json::<Wrapper<User>>()
                .await
                .context(UnableToDeserializeRequest)?
                .into_result()
                .context(RequestFailed)?
                .trace_quota()
                .into_singleton()
                .context(RequestDidNotHaveOneResult)
        }
        .instrument(s)
        .await
    }

    pub async fn unread_notifications(
        &self,
    ) -> Result<Vec<Notification>, UnreadNotificationsError> {
        let s = trace_span!("unread_notifications");

        async {
            let Self {
                client,
                auth_config,
            } = self;

            #[derive(Debug, Serialize)]
            struct UnreadNotificationsParams<'a> {
                filter: &'a str,
            }

            let params = auth_config.auth_params(UnreadNotificationsParams {
                filter: FILTER_DEFAULT,
            });

            let r = client
                .get(auth_config.config.unread_notifications.clone())
                .query(&params)
                .send()
                .await
                .context(UnableToExecuteRequest)?
                .ensure_success()
                .await
                .context(RequestRejected)?
                .json::<Wrapper<Notification>>()
                .await
                .context(UnableToDeserializeRequest)?
                .into_result()
                .context(RequestFailed)?
                .trace_quota();

            Ok(r.items)
        }
        .instrument(s)
        .await
    }

    pub async fn unread_inbox(&self) -> Result<Vec<Inbox>, UnreadInboxError> {
        let s = trace_span!("unread_inbox");

        async {
            let Self {
                client,
                auth_config,
            } = self;

            #[derive(Debug, Serialize)]
            struct UnreadInboxParams<'a> {
                filter: &'a str,
            }

            let params = auth_config.auth_params(UnreadInboxParams {
                filter: FILTER_WITH_BODY,
            });

            let r = client
                .get(auth_config.config.unread_inbox.clone())
                .query(&params)
                .send()
                .await
                .context(UnableToExecuteRequest)?
                .ensure_success()
                .await
                .context(RequestRejected)?
                .json::<Wrapper<Inbox>>()
                .await
                .context(UnableToDeserializeRequest)?
                .into_result()
                .context(RequestFailed)?
                .trace_quota();

            Ok(r.items)
        }
        .instrument(s)
        .await
    }
}

#[derive(Debug, Snafu)]
#[snafu(display(
    "NotSuccess: {:?} {} {} {:?} {}",
    status,
    res,
    headers,
    body,
    String::from_utf8_lossy(body)
))]
pub struct NotSuccess {
    status: reqwest::StatusCode,
    res: String,
    headers: String,
    body: Vec<u8>,
}

trait EnsureSuccess: Sized {
    fn ensure_success(self) -> BoxFuture<'static, Result<Self, NotSuccess>>;
}

impl EnsureSuccess for reqwest::Response {
    fn ensure_success(self) -> BoxFuture<'static, Result<Self, NotSuccess>> {
        async {
            let status = self.status();
            if !status.is_success() {
                let res = format!("{:?}", self);
                let headers = format!("{:?}", self.headers());
                let body = self.bytes().await.unwrap_or_default();

                NotSuccessContext {
                    status,
                    res,
                    headers,
                    body: &body[..],
                }
                .fail()
            } else {
                Ok(self)
            }
        }
        .boxed()
    }
}

trait TraceQuota {
    fn trace_quota(self) -> Self;
}

impl<T> TraceQuota for ApiSuccess<T> {
    fn trace_quota(self) -> Self {
        trace!("{:?}", self.quota);
        self
    }
}

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("STACK_OVERFLOW_CLIENT_ID must be set"))]
    UnknownClientId {
        source: env::VarError,
    },

    #[snafu(display("STACK_OVERFLOW_CLIENT_SECRET must be set"))]
    UnknownClientSecret {
        source: env::VarError,
    },

    #[snafu(display("STACK_OVERFLOW_CLIENT_KEY must be set"))]
    UnknownClientKey {
        source: env::VarError,
    },

    UnableToConfigureUnreadNotificationsUrl {
        source: url::ParseError,
    },

    UnableToConfigureUnreadInboxUrl {
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

    AccessTokenRequestRejected {
        source: NotSuccess,
    },

    UnableToDeserializeAccessTokenRequest {
        source: reqwest::Error,
    },
}

type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Snafu)]
pub enum CurrentUserError {
    #[snafu(context(false))]
    Common {
        source: CommonError,
    },

    RequestDidNotHaveOneResult,
}

#[derive(Debug, Snafu)]
pub struct UnreadNotificationsError(CommonError);

impl IsTransient for UnreadNotificationsError {
    fn is_transient(&self) -> bool {
        self.0.is_transient()
    }
}

#[derive(Debug, Snafu)]
pub struct UnreadInboxError(CommonError);

impl IsTransient for UnreadInboxError {
    fn is_transient(&self) -> bool {
        self.0.is_transient()
    }
}

#[derive(Debug, Snafu)]
pub enum CommonError {
    UnableToExecuteRequest { source: reqwest::Error },

    RequestRejected { source: NotSuccess },

    UnableToDeserializeRequest { source: reqwest::Error },

    RequestFailed { source: ApiError },
}

impl IsTransient for CommonError {
    fn is_transient(&self) -> bool {
        match self {
            Self::UnableToExecuteRequest { source } => source.is_transient(),
            _ => false,
        }
    }
}
