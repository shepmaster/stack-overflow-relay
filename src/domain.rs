pub use crate::pushover::UserKey;
pub use crate::stack_overflow::AccountId;

#[derive(Debug)]
pub struct IncomingNotification {
    pub account_id: AccountId,
    pub text: String,
}

#[derive(Debug)]
pub struct OutgoingNotification {
    pub user: UserKey,
    pub text: String,
}
