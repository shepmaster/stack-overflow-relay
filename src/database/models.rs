use super::schema::*;

#[derive(Debug, Queryable, Insertable)]
pub struct Registration {
    pub account_id: i32,
    pub access_token: String,
}

#[derive(Debug, Queryable, Insertable)]
pub struct PushoverUser {
    pub key: String,
    pub account_id: i32,
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
