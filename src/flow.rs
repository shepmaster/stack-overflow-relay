use crate::{
    database::DbHandle, domain::AccountId, poll_spawner::PollSpawnerHandle,
    GlobalStackOverflowConfig,
};
use snafu::{ResultExt, Snafu};

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
            .context(UnableToGetOauthAccessToken)?;

        let so_client = so_client.into_auth_client(resp);

        let resp = so_client
            .current_user()
            .await
            .context(UnableToGetCurrentUser)?;

        let account_id = resp.account_id;
        let access_token = so_client.access_token().clone();

        db.register(account_id, access_token.clone())
            .await
            .context(UnableToPersistRegistration)?;
        poll_spawner.start_polling(account_id, access_token).await;

        Ok(account_id)
    }
}

#[derive(Debug, Clone)]
pub struct NotifyFlow {
    db: DbHandle,
}

impl NotifyFlow {
    pub fn new(db: DbHandle) -> Self {
        Self { db }
    }

    pub async fn notify(&mut self, notifications: Vec<(AccountId, String)>) -> Result<()> {
        let Self { db } = self;

        if notifications.is_empty() {
            return Ok(());
        };
        let new_notifications = db
            .add_new_notifications(notifications)
            .await
            .context(UnableToPersistNotifications)?;

        if new_notifications.is_empty() {
            return Ok(());
        };
        crate::ios::send_notifications(new_notifications).await; // Need the unique ios id

        Ok(())
    }
}

#[derive(Debug, Snafu)]
pub enum Error {
    UnableToGetOauthAccessToken {
        source: crate::stack_overflow::Error,
    },

    UnableToGetCurrentUser {
        source: crate::stack_overflow::Error,
    },

    UnableToPersistRegistration {
        source: crate::database::Error,
    },

    UnableToPersistNotifications {
        source: crate::database::Error,
    },
}

type Result<T, E = Error> = std::result::Result<T, E>;
