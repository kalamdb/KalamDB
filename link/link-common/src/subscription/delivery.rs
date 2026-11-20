use crate::{models::ChangeEvent, Result};

/// Ordered messages from the socket actor to a subscription consumer.
pub(crate) enum SubscriptionDelivery {
    Event(Result<ChangeEvent>),
    ResetSnapshot { epoch: u64 },
}
