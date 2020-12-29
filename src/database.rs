use crate::{
    domain::{IncomingNotification, OutgoingNotification, UserKey},
    stack_overflow::{AccessToken, AccountId},
};
use diesel::{connection::TransactionManager, dsl::any, pg::upsert::excluded, prelude::*};
use snafu::{ResultExt, Snafu};
use tracing::{trace, trace_span};

mod models;
mod schema;

pub struct Db {
    conn: diesel::PgConnection,
}

impl Db {
    pub fn new(conn: diesel::PgConnection) -> Self {
        Self { conn }
    }
}

#[alictor::alictor(kind = blocking)]
impl Db {
    fn registrations(&self) -> Result<Vec<(AccountId, AccessToken)>> {
        use schema::registrations;

        let Self { conn } = self;

        let r = registrations::table
            .select((registrations::account_id, registrations::access_token))
            .load(conn)
            .context(UnableToQueryRegistrations)?;

        Ok(r.into_iter()
            .map(|(id, token)| (AccountId(id), AccessToken(token)))
            .collect())
    }

    fn register(&self, account_id: AccountId, access_token: AccessToken) -> Result<()> {
        use models::Registration;
        use schema::registrations::dsl;

        let Self { conn } = self;

        let registration = Registration {
            account_id: account_id.0,
            access_token: access_token.0,
        };

        diesel::insert_into(dsl::registrations)
            .values(&registration)
            .on_conflict(dsl::account_id)
            .do_update()
            .set(dsl::access_token.eq(dsl::access_token)) // should this be `excluded`?
            .execute(conn)
            .context(UnableToInsertRegistration)?;

        Ok(())
    }

    fn set_pushover_user(&self, account_id: AccountId, user_key: UserKey) -> Result<()> {
        use models::PushoverUser;
        use schema::pushover_users::dsl;

        let Self { conn } = self;

        let user = PushoverUser {
            key: user_key.0,
            account_id: account_id.0,
        };

        diesel::insert_into(dsl::pushover_users)
            .values(&user)
            .on_conflict(dsl::account_id)
            .do_update()
            .set(dsl::key.eq(excluded(dsl::key)))
            .execute(conn)
            .context(UnableToInsertPushoverUser)?;

        Ok(())
    }

    fn add_new_notifications(
        &self,
        notifications: Vec<IncomingNotification>,
    ) -> Result<Vec<OutgoingNotification>> {
        use models::NewNotification;
        use schema::notifications as n;
        use schema::pushover_users as p;

        let s = trace_span!("add_new_notifications");
        let _s = s.enter();
        let Self { conn } = self;

        trace!("Checking {} notifications", notifications.len());

        let notifications: Vec<_> = notifications
            .into_iter()
            .map(|n| NewNotification {
                account_id: n.account_id.0,
                text: n.text,
            })
            .collect();

        let raw_notifications: Vec<(String, String)> = transaction(conn, |conn| {
            let ids = diesel::insert_into(n::table)
                .values(notifications)
                .on_conflict((n::account_id, n::text))
                .do_nothing()
                .returning(n::id)
                .log_query()
                .get_results::<i32>(conn)
                .context(UnableToInsertNotifications)?;

            trace!("Inserted {} new notifications", ids.len());

            p::table
                .inner_join(n::table.on(n::account_id.eq(p::account_id)))
                .select((p::key, n::text))
                .filter(n::id.eq(any(ids)))
                .log_query()
                .load(conn)
                .context(UnableToQueryNotifications)
        })?;

        Ok(raw_notifications
            .into_iter()
            .map(|(key, text)| OutgoingNotification {
                user: UserKey(key),
                text,
            })
            .collect())
    }
}

trait LogQuery {
    fn log_query(self) -> Self;
}

impl<T> LogQuery for T
where
    for<'a> diesel::query_builder::DebugQuery<'a, T, diesel::pg::Pg>: std::fmt::Display,
{
    fn log_query(self) -> Self {
        trace!("Query: {}", diesel::debug_query::<diesel::pg::Pg, _>(&self));
        self
    }
}

fn transaction<'a, T, F>(conn: &'a PgConnection, f: F) -> Result<T>
where
    F: FnOnce(&'a PgConnection) -> Result<T>,
{
    let transaction_manager = conn.transaction_manager();
    transaction_manager
        .begin_transaction(conn)
        .context(TransactionFailed)?;
    match f(conn) {
        Ok(value) => {
            transaction_manager
                .commit_transaction(conn)
                .context(TransactionFailed)?;
            Ok(value)
        }
        Err(e) => {
            transaction_manager
                .rollback_transaction(conn)
                .context(TransactionFailed)?;
            Err(e)
        }
    }
}

#[derive(Debug, Snafu)]
pub enum Error {
    UnableToQueryRegistrations { source: diesel::result::Error },

    UnableToInsertRegistration { source: diesel::result::Error },

    UnableToInsertPushoverUser { source: diesel::result::Error },

    UnableToInsertNotifications { source: diesel::result::Error },

    UnableToQueryNotifications { source: diesel::result::Error },

    TransactionFailed { source: diesel::result::Error },
}

type Result<T, E = Error> = std::result::Result<T, E>;
