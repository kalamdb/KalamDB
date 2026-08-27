//! Migration entity for system.migrations table.

use kalamdb_commons::{datatypes::KalamDataType, models::MigrationId};
use kalamdb_macros::table;
use serde::{Deserialize, Serialize};

#[table(name = "migrations", comment = "Project migration lifecycle state")]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Migration {
    #[column(
        id = 1,
        ordinal = 1,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = true,
        default = "None",
        comment = "Namespace-qualified migration key"
    )]
    pub migration_key: MigrationId,
    #[column(
        id = 2,
        ordinal = 2,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "Migration identifier"
    )]
    pub migration_id:  String,
    #[column(
        id = 3,
        ordinal = 3,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "Target namespace"
    )]
    pub namespace:     String,
    #[column(
        id = 4,
        ordinal = 4,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "Human-readable migration name"
    )]
    pub name:          String,
    #[column(
        id = 5,
        ordinal = 5,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "SHA-256 checksum of the applied UP SQL"
    )]
    pub checksum:      String,
    #[column(
        id = 6,
        ordinal = 6,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "Migration lifecycle status"
    )]
    pub status:        String,
    #[column(
        id = 7,
        ordinal = 7,
        data_type(KalamDataType::Timestamp),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "Unix timestamp in milliseconds when migration started"
    )]
    pub started_at:    Option<i64>,
    #[column(
        id = 8,
        ordinal = 8,
        data_type(KalamDataType::Timestamp),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "Unix timestamp in milliseconds when migration finished"
    )]
    pub finished_at:   Option<i64>,
    #[column(
        id = 9,
        ordinal = 9,
        data_type(KalamDataType::Text),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "Failure details"
    )]
    pub error_message: Option<String>,
    #[column(
        id = 10,
        ordinal = 10,
        data_type(KalamDataType::Text),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "Source migration file"
    )]
    pub source:        Option<String>,
    #[column(
        id = 11,
        ordinal = 11,
        data_type(KalamDataType::Text),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "Kalam CLI version that wrote the record"
    )]
    pub kalam_version: Option<String>,
}
