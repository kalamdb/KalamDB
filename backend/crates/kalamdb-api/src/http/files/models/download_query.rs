//! Download query parameters model

use kalamdb_commons::models::UserId;
use serde::Deserialize;

/// Query parameters for file download
#[derive(Debug, Deserialize)]
pub struct DownloadQuery {
    /// Optional user_id for user-table downloads.
    /// Cross-user raw byte downloads are limited to dba/system roles.
    #[serde(default, deserialize_with = "deserialize_optional_user_id")]
    pub user_id: Option<UserId>,
}

fn deserialize_optional_user_id<'de, D>(deserializer: D) -> Result<Option<UserId>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    Ok(opt.map(|s| UserId::new(&s)))
}
