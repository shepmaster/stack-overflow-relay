use super::schema::*;
use crate::domain;

#[derive(Debug, Queryable, Insertable)]
pub struct Registration {
    pub account_id: i32,
    pub access_token: String,
}

#[derive(Debug, Insertable)]
#[table_name = "notifications"]
pub struct NewNotification {
    pub account_id: i32,
    pub text: String,
}

#[derive(Debug, Queryable)]
pub struct Notification {
    pub id: i32,
    pub account_id: i32,
    pub text: String,
}

impl From<Notification> for domain::Notification {
    fn from(other: Notification) -> Self {
        let Notification {
            account_id, text, ..
        } = other;
        let account_id = domain::AccountId(account_id);
        domain::Notification { account_id, text }
    }
}
