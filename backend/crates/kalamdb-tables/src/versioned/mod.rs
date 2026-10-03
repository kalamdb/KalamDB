//! MVCC helpers shared by user and shared tables.
//!
//! Stream tables stay on the append log and do not use this path.

mod hot_pk;

pub(crate) use hot_pk::{first_live_pk, insert_notification_when_watched};
