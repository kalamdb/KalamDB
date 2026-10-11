// IDs module
#[cfg(feature = "storage")]
pub mod row_id;
pub mod seq_id;
#[cfg(feature = "storage")]
pub mod snowflake;
pub mod version_id;

#[cfg(feature = "storage")]
pub use row_id::{SharedTableRowId, StreamTableRowId, UserTableRowId};
pub use seq_id::SeqId;
#[cfg(feature = "storage")]
pub use snowflake::SnowflakeGenerator;
pub use version_id::{
    check_entry_slot_count, same_domain_cmp, EntryVersionBound, MaterializedFrontier,
    RaftVersionId, VersionCheckpoint, VersionError, VersionId, MAX_LOG_INDEX, MAX_ORDINAL,
    MAX_ROW_SLOTS_PER_ENTRY, ORDINAL_BITS,
};

mod version_domain;
pub use version_domain::VersionDomain;
