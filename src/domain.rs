pub use crate::stack_overflow::AccountId;

#[derive(Debug)]
pub struct Notification {
    pub account_id: AccountId,
    pub text: String,
}
