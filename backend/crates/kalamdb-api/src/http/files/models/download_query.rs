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
    opt.map(UserId::try_new)
        .transpose()
        .map_err(|e| serde::de::Error::custom(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::DownloadQuery;

    #[test]
    fn download_query_rejects_invalid_user_id() {
        serde_json::from_value::<DownloadQuery>(serde_json::json!({
            "user_id": "../root"
        }))
        .expect_err("invalid query user_id must be rejected without panicking");
    }

    #[test]
    fn download_query_accepts_canonical_user_id() {
        let query = serde_json::from_value::<DownloadQuery>(serde_json::json!({
            "user_id": "alice_1"
        }))
        .expect("canonical query user_id should deserialize");

        assert_eq!(query.user_id.as_ref().map(|id| id.as_str()), Some("alice_1"));
    }
}
