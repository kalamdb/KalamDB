mod ack;
mod add_source;
mod cleanup;
mod clear;
mod consume;
mod create;
mod drop;
mod name_resolution;
mod reset_consumer_group;
mod retention;

pub use ack::AckHandler;
pub use add_source::AddTopicSourceHandler;
pub use clear::ClearTopicHandler;
pub use consume::ConsumeHandler;
pub use create::CreateTopicHandler;
pub use drop::DropTopicHandler;
use kalamdb_core::error::KalamDbError;
use kalamdb_system::SystemError;
pub use reset_consumer_group::ResetConsumerGroupHandler;
pub use retention::{AlterTopicRetentionHandler, ClearTopicRetentionHandler};

pub(super) fn catalog_error(error: SystemError) -> KalamDbError {
    match error {
        SystemError::AlreadyExists(message) => KalamDbError::AlreadyExists(message),
        SystemError::InvalidOperation(message) => KalamDbError::InvalidSql(message),
        other => KalamDbError::ExecutionError(other.to_string()),
    }
}
