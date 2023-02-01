use crate::{
    database::DbHandle,
    domain::{AccountId, IncomingNotification, UserKey},
    error::IsTransient,
    poll_spawner::PollSpawnerHandle,
    pushover, GlobalStackOverflowConfig,
};
use snafu::{ResultExt, Snafu};
use tracing::{trace, trace_span, Instrument};

#[derive(Debug, Clone)]
pub struct BootFlow {
    db: DbHandle,
    poll_spawner: PollSpawnerHandle,
}

impl BootFlow {
    pub fn new(db: DbHandle, poll_spawner: PollSpawnerHandle) -> Self {
        Self { db, poll_spawner }
    }

    pub async fn boot(&mut self) -> Result<()> {
        let Self { db, poll_spawner } = self;

        let registrations = db
            .registrations()
            .await
            .context(UnableToLoadRegistrationsSnafu)?;
        poll_spawner.start_many(registrations).await;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct RegisterFlow {
    so_config: GlobalStackOverflowConfig,
    db: DbHandle,
    poll_spawner: PollSpawnerHandle,
}

impl RegisterFlow {
    pub fn new(
        so_config: GlobalStackOverflowConfig,
        db: DbHandle,
        poll_spawner: PollSpawnerHandle,
    ) -> Self {
        Self {
            so_config,
            db,
            poll_spawner,
        }
    }

    pub async fn register(&mut self, code: &str, redirect_uri: &str) -> Result<AccountId> {
        let Self {
            so_config,
            db,
            poll_spawner,
        } = self;

        let so_client = so_config.clone().into_unauth_client();
        let resp = so_client
            .get_access_token(code, redirect_uri)
            .await
            .context(UnableToGetOauthAccessTokenSnafu)?;

        let so_client = so_client.into_auth_client(resp);

        let resp = so_client.current_user().await?;

        let account_id = resp.account_id;
        let access_token = so_client.access_token().clone();

        db.register(account_id, access_token.clone())
            .await
            .context(UnableToPersistRegistrationSnafu)?;
        poll_spawner.start_polling(account_id, access_token).await;

        Ok(account_id)
    }
}

#[derive(Debug, Clone)]
pub struct SetPushoverUserFlow {
    db: DbHandle,
}

impl SetPushoverUserFlow {
    pub fn new(db: DbHandle) -> Self {
        Self { db }
    }

    pub async fn set_pushover_user(&mut self, account_id: AccountId, user: UserKey) -> Result<()> {
        let Self { db } = self;

        db.set_pushover_user(account_id, user)
            .await
            .context(UnableToPersistPushoverUserSnafu)?;

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ProxyNotificationsFlow {
    so_config: GlobalStackOverflowConfig,
    db: DbHandle,
    pushover: pushover::Client,
}

impl ProxyNotificationsFlow {
    pub fn new(
        so_config: GlobalStackOverflowConfig,
        db: DbHandle,
        pushover: pushover::Client,
    ) -> Self {
        Self {
            so_config,
            db,
            pushover,
        }
    }

    pub fn auth(
        self,
        account_id: AccountId,
        access_token: crate::stack_overflow::AccessToken,
    ) -> ProxyNotificationsAuthFlow {
        let Self {
            so_config,
            db,
            pushover,
        } = self;

        let so_client = crate::stack_overflow::AuthClient::new(so_config.clone(), access_token);

        ProxyNotificationsAuthFlow {
            so_client,
            db,
            pushover,
            account_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProxyNotificationsAuthFlow {
    so_client: crate::stack_overflow::AuthClient,
    db: DbHandle,
    pushover: pushover::Client,
    account_id: AccountId,
}

impl ProxyNotificationsAuthFlow {
    pub async fn proxy(&mut self) -> Result<()> {
        let s = trace_span!("notify");
        let Self {
            so_client,
            db,
            pushover,
            account_id,
        } = self;
        let account_id = *account_id;

        async {
            let (a, b) = futures::join!(so_client.unread_notifications(), so_client.unread_inbox());

            let a = a?.into_iter().map(|n| IncomingNotification {
                account_id,
                text: n.body,
            });

            let b = b?.into_iter().map(|i| IncomingNotification {
                account_id,
                text: i.body,
            });

            let notifications: Vec<_> = a.chain(b).collect();

            if notifications.is_empty() {
                trace!("No notifications present");
                return Ok(());
            };

            let new_notifications = db
                .add_new_notifications(notifications)
                .await
                .context(UnableToPersistNotificationsSnafu)?;
            if new_notifications.is_empty() {
                trace!("All notifications have been seen");
                return Ok(());
            }

            pushover
                .notify(new_notifications)
                .await
                .context(UnableToDeliverNotificationsSnafu)?;

            Ok(())
        }
        .instrument(s)
        .await
    }
}

#[derive(Debug, Snafu)]
#[allow(clippy::enum_variant_names)]
pub enum Error {
    UnableToLoadRegistrations {
        source: crate::database::Error,
    },

    UnableToGetOauthAccessToken {
        source: crate::stack_overflow::Error,
    },

    #[snafu(context(false))]
    UnableToGetCurrentUser {
        source: crate::stack_overflow::CurrentUserError,
    },

    UnableToPersistRegistration {
        source: crate::database::Error,
    },

    UnableToPersistPushoverUser {
        source: crate::database::Error,
    },

    #[snafu(context(false))]
    UnableToGetUnreadNotifications {
        source: crate::stack_overflow::UnreadNotificationsError,
    },

    #[snafu(context(false))]
    UnableToGetUnreadInbox {
        source: crate::stack_overflow::UnreadInboxError,
    },

    UnableToPersistNotifications {
        source: crate::database::Error,
    },

    UnableToDeliverNotifications {
        source: crate::pushover::Error,
    },
}

impl IsTransient for Error {
    fn is_transient(&self) -> bool {
        match self {
            Self::UnableToGetUnreadNotifications { source } => source.is_transient(),
            Self::UnableToGetUnreadInbox { source } => source.is_transient(),
            Self::UnableToDeliverNotifications { source } => source.is_transient(),
            _ => false,
        }
    }
}

type Result<T, E = Error> = std::result::Result<T, E>;
