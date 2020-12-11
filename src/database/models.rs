use super::schema::*;

#[derive(Queryable, Insertable)]
pub struct Registration {
    pub account_id: i32,
    pub access_token: String,
}
