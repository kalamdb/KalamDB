/// Controls when a live subscription advances its reconnect cursor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SubscriptionAckMode {
    /// Reading an event advances progress, preserving existing SDK behavior.
    #[default]
    Automatic,
    /// Progress advances only after the consumer explicitly acknowledges it.
    Explicit,
}
