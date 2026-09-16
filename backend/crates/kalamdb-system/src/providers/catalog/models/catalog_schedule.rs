use kalamdb_commons::{
    datatypes::KalamDataType,
    models::{NamespaceId, RoutineId, ScheduleId, UserId},
};
use kalamdb_macros::table;
use serde::{Deserialize, Serialize};

/// Durable schedule definition and latest execution state. Times are Unix milliseconds.
#[table(
    name = "schedules",
    comment = "Procedure schedules; overlap and misfire policy are skip"
)]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CatalogSchedule {
    #[column(
        id = 1,
        ordinal = 1,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = true,
        default = "None",
        comment = "schedule id"
    )]
    pub schedule_id:       ScheduleId,
    #[column(
        id = 2,
        ordinal = 2,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "namespace id"
    )]
    pub namespace_id:      NamespaceId,
    #[column(
        id = 3,
        ordinal = 3,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "name"
    )]
    pub name:              String,
    #[column(
        id = 4,
        ordinal = 4,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "routine id"
    )]
    pub routine_id:        RoutineId,
    #[column(
        id = 5,
        ordinal = 5,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "principal user id"
    )]
    pub principal_user_id: UserId,
    #[column(
        id = 6,
        ordinal = 6,
        data_type(KalamDataType::Text),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "cron"
    )]
    pub cron:              Option<String>,
    #[column(
        id = 7,
        ordinal = 7,
        data_type(KalamDataType::BigInt),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "interval ms"
    )]
    pub interval_ms:       Option<i64>,
    #[column(
        id = 8,
        ordinal = 8,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "timezone"
    )]
    pub timezone:          String,
    #[column(
        id = 9,
        ordinal = 9,
        data_type(KalamDataType::Boolean),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "enabled"
    )]
    pub enabled:           bool,
    #[column(
        id = 10,
        ordinal = 10,
        data_type(KalamDataType::BigInt),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "next run at"
    )]
    pub next_run_at:       i64,
    #[column(
        id = 11,
        ordinal = 11,
        data_type(KalamDataType::Text),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "run id"
    )]
    pub run_id:            Option<String>,
    #[column(
        id = 12,
        ordinal = 12,
        data_type(KalamDataType::Text),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "owner"
    )]
    pub owner:             Option<String>,
    #[column(
        id = 13,
        ordinal = 13,
        data_type(KalamDataType::BigInt),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "running until"
    )]
    pub running_until:     Option<i64>,
    #[column(
        id = 14,
        ordinal = 14,
        data_type(KalamDataType::BigInt),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "last started at"
    )]
    pub last_started_at:   Option<i64>,
    #[column(
        id = 15,
        ordinal = 15,
        data_type(KalamDataType::BigInt),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "last finished at"
    )]
    pub last_finished_at:  Option<i64>,
    #[column(
        id = 16,
        ordinal = 16,
        data_type(KalamDataType::Text),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "last error"
    )]
    pub last_error:        Option<String>,
    #[column(
        id = 17,
        ordinal = 17,
        data_type(KalamDataType::BigInt),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "run count"
    )]
    pub run_count:         i64,
    #[column(
        id = 18,
        ordinal = 18,
        data_type(KalamDataType::BigInt),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "skip count"
    )]
    pub skip_count:        i64,
    #[column(
        id = 19,
        ordinal = 19,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "version"
    )]
    pub version:           String,
}
impl kalamdb_commons::KSerializable for CatalogSchedule {}
