use kalam_pg_common::{KalamPgError, USER_ID_GUC};
use kalamdb_commons::models::UserId;

/// Parsed extension session settings extracted from PostgreSQL GUCs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionSettings {
    session_user_id: Option<UserId>,
    current_schema: Option<String>,
}

impl SessionSettings {
    /// Parse and validate a raw `kalam.user_id` GUC value.
    pub fn parse_user_id_value(raw: &str) -> Result<UserId, KalamPgError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(KalamPgError::Validation(format!(
                "{USER_ID_GUC} cannot be empty"
            )));
        }

        UserId::try_new(trimmed.to_string()).map_err(|error| {
            KalamPgError::Validation(format!("invalid {USER_ID_GUC}: {error}"))
        })
    }

    /// Parse an optional raw GUC value into a validated user id.
    pub fn parse_optional_user_id_value(raw: Option<&str>) -> Result<Option<UserId>, KalamPgError> {
        match raw.map(str::trim).filter(|value| !value.is_empty()) {
            None => Ok(None),
            Some(value) => Self::parse_user_id_value(value).map(Some),
        }
    }

    /// Parse the supported PostgreSQL session values used by the extension.
    pub fn from_guc_values(
        user_id_value: Option<&str>,
        current_schema_value: Option<&str>,
    ) -> Result<Self, KalamPgError> {
        let session_user_id = Self::parse_optional_user_id_value(user_id_value)?;
        let current_schema = current_schema_value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        Ok(Self {
            session_user_id,
            current_schema,
        })
    }

    /// Return the optional session-scoped user id.
    pub fn session_user_id(&self) -> Option<&UserId> {
        self.session_user_id.as_ref()
    }

    /// Return the optional PostgreSQL current schema/default namespace.
    pub fn current_schema(&self) -> Option<&str> {
        self.current_schema.as_deref()
    }

    /// Return the exact GUC name used by the extension.
    pub fn guc_name() -> &'static str {
        USER_ID_GUC
    }
}
