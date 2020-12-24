use crate::{
    domain::{IncomingNotification, OutgoingNotification, UserKey},
    stack_overflow::{AccessToken, AccountId},
};
use diesel::{connection::TransactionManager, pg::upsert::excluded, prelude::*};
use futures::{
    channel::{mpsc, oneshot},
    SinkExt, StreamExt,
};
use snafu::{ResultExt, Snafu};

mod models;
mod schema;

joinable!(schema::pushover_users -> schema::notifications (account_id));

pub struct Db {
    conn: diesel::PgConnection,
}

impl Db {
    pub fn new(conn: diesel::PgConnection) -> Self {
        Self { conn }
    }
}

impl Db {
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
        use schema::pushover_users as p; //::dsl; //::dsl;

        let Self { conn } = self;

        let notifications: Vec<_> = notifications
            .into_iter()
            .map(|n| NewNotification {
                account_id: n.account_id.0,
                text: n.text,
            })
            .collect();

        let raw_notifications: Vec<(String, String)> = transaction(conn, |conn| {
            diesel::insert_into(n::table)
                .values(notifications)
                .on_conflict((n::account_id, n::text))
                .do_nothing()
                .execute(conn)
                .context(UnableToInsertNotifications)?;

            p::table
                .inner_join(n::table)
                .select((p::key, n::text))
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
    UnableToInsertRegistration { source: diesel::result::Error },

    UnableToInsertPushoverUser { source: diesel::result::Error },

    UnableToInsertNotifications { source: diesel::result::Error },

    UnableToQueryNotifications { source: diesel::result::Error },

    TransactionFailed { source: diesel::result::Error },
}

type Result<T, E = Error> = std::result::Result<T, E>;

// Can this be auto-generated by a proc-macro?
// https://draft.ryhl.io/blog/actors-with-tokio/

pub fn spawn(this: Db) -> (DbHandle, tokio::task::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(10);
    let child = tokio::spawn(db_task(this, rx));
    (DbHandle(tx), child)
}

#[derive(Debug, Clone)]
pub struct DbHandle(mpsc::Sender<DbCommand>);

impl DbHandle {
    pub async fn register(&mut self, a: AccountId, b: AccessToken) -> Result<()> {
        let (tx, rx) = oneshot::channel();

        // Ignore send errors. If this send fails, so does the
        // rx.await below. There's no reason to check for the
        // same failure twice.
        let _ = self.0.send(DbCommand::Register(tx, a, b)).await;
        rx.await.expect("Actor error - task gone")
    }

    pub async fn set_pushover_user(&mut self, a: AccountId, b: UserKey) -> Result<()> {
        let (tx, rx) = oneshot::channel();

        // Ignore send errors. If this send fails, so does the
        // rx.await below. There's no reason to check for the
        // same failure twice.
        let _ = self.0.send(DbCommand::SetPushoverUser(tx, a, b)).await;
        rx.await.expect("Actor error - task gone")
    }

    pub async fn add_new_notifications(
        &mut self,
        a: Vec<IncomingNotification>,
    ) -> Result<Vec<OutgoingNotification>> {
        let (tx, rx) = oneshot::channel();

        // Ignore send errors. If this send fails, so does the
        // rx.await below. There's no reason to check for the
        // same failure twice.
        let _ = self.0.send(DbCommand::AddNewNotifications(tx, a)).await;
        rx.await.expect("Actor error - task gone")
    }
}

#[derive(Debug)]
enum DbCommand {
    Register(oneshot::Sender<Result<()>>, AccountId, AccessToken),

    SetPushoverUser(oneshot::Sender<Result<()>>, AccountId, UserKey),

    AddNewNotifications(
        oneshot::Sender<Result<Vec<OutgoingNotification>>>,
        Vec<IncomingNotification>,
    ),
}

async fn db_task(#[allow(unused_mut)] mut this: Db, mut rx: mpsc::Receiver<DbCommand>) {
    while let Some(cmd) = rx.next().await {
        match cmd {
            DbCommand::Register(__r, a, b) => {
                // Macro: block_in_place vs nothing vs spawn_blocking
                // TODO: This should be spawn-blocking
                let retval = tokio::task::block_in_place(|| this.register(a, b));

                // If we couldn't respond, that's OK
                let _ = __r.send(retval);
            }

            DbCommand::SetPushoverUser(__r, a, b) => {
                // Macro: block_in_place vs nothing vs spawn_blocking
                // TODO: This should be spawn-blocking
                let retval = tokio::task::block_in_place(|| this.set_pushover_user(a, b));

                // If we couldn't respond, that's OK
                let _ = __r.send(retval);
            }

            DbCommand::AddNewNotifications(__r, a) => {
                // Macro: block_in_place vs nothing vs spawn_blocking
                // TODO: This should be spawn-blocking
                let retval = tokio::task::block_in_place(|| this.add_new_notifications(a));

                // If we couldn't respond, that's OK
                let _ = __r.send(retval);
            }
        }
    }
}

//
