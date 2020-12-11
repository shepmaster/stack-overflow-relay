use crate::{
    database::DbHandle, poll_spawner::PollSpawnerHandle, GlobalConfig, GlobalStackOverflowConfig,
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

        db.register(account_id, access_token.clone()).await;
        poll_spawner.start_polling(account_id, access_token).await;

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
}

type Result<T, E = Error> = std::result::Result<T, E>;
