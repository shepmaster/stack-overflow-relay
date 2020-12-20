use crate::{
    database::DbHandle, domain::AccountId, poll_spawner::PollSpawnerHandle, GlobalConfig,
    GlobalStackOverflowConfig,
};
use snafu::{ResultExt, Snafu};

#[derive(Debug, Clone)]
pub struct RegisterFlow {
    config: GlobalConfig,
    so_config: GlobalStackOverflowConfig,
    db: DbHandle,
    poll_spawner: PollSpawnerHandle,
}

impl RegisterFlow {
    pub fn new(
        config: GlobalConfig,
        so_config: GlobalStackOverflowConfig,
        db: DbHandle,
        poll_spawner: PollSpawnerHandle,
    ) -> Self {
        Self {
            config,
            so_config,
            db,
            poll_spawner,
        }
    }

    pub async fn register(&mut self, code: &str, redirect_uri: &str) -> Result<()> {
        let Self {
            config,
            so_config,
            db,
            poll_spawner,
        } = self;

        let req = crate::stack_overflow::AccessTokenRequest {
            client_id: &config.stack_overflow_client_id,
            client_secret: &config.stack_overflow_client_secret,
            code,
            redirect_uri,
        };
        let resp = crate::stack_overflow::get_access_token(&req)
            .await
            .context(UnableToGetOauthAccessToken)?;

        dbg!(&resp);

        let access_token = resp.access_token;

        let req = crate::stack_overflow::CurrentUserParams {
            key: &config.stack_overflow_client_key,
            site: "stackoverflow",
            access_token: &access_token,
            filter: "default",
        };
        let resp = crate::stack_overflow::current_user(so_config, &req)
            .await
            .context(UnableToGetCurrentUser)?;

        dbg!(&resp);

        let account_id = resp.account_id;

        db.register(account_id, access_token.clone())
            .await
            .context(UnableToPersistRegistration)?;
        poll_spawner.start_polling(account_id, access_token).await;

        Ok(())
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
