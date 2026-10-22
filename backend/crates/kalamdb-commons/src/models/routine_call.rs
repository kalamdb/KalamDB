//! Shared `CALL` / column-default invocation: routine identity plus arguments.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{call_argument::CallArgument, ids::RoutineId};

/// A DEFAULT function KalamDB evaluates itself.
///
/// Aliases live only in [`BuiltinDefault::parse`]. PostgreSQL names we already
/// implement as session functions are included. `clock_timestamp`,
/// `current_date`, `current_time`, and `gen_random_uuid` are not: they are a
/// different type or a different clock, and there is no function for them yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinDefault {
    Now,
    CurrentUser,
    CurrentRole,
    CurrentSchema,
    CurrentDatabase,
    SnowflakeId,
    SerialId,
    UuidV7,
    Ulid,
}

impl BuiltinDefault {
    /// Resolve an unqualified function name, including PostgreSQL aliases.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "now" | "current_timestamp" | "transaction_timestamp" | "statement_timestamp" => {
                Self::Now
            },
            "current_user" | "session_user" | "current_user_id" | "user" => Self::CurrentUser,
            "current_role" => Self::CurrentRole,
            "current_schema" | "current_namespace" => Self::CurrentSchema,
            "current_database" | "current_catalog" => Self::CurrentDatabase,
            "snowflake_id" | "auto_increment" => Self::SnowflakeId,
            "serial_id" => Self::SerialId,
            "uuid_v7" => Self::UuidV7,
            "ulid" => Self::Ulid,
            _ => return None,
        })
    }

    /// Name stored on the column and printed in `DEFAULT` SQL.
    pub fn canonical_name(self) -> &'static str {
        match self {
            Self::Now => "now",
            Self::CurrentUser => "current_user",
            Self::CurrentRole => "current_role",
            Self::CurrentSchema => "current_schema",
            Self::CurrentDatabase => "current_database",
            Self::SnowflakeId => "snowflake_id",
            Self::SerialId => "serial_id",
            Self::UuidV7 => "uuid_v7",
            Self::Ulid => "ulid",
        }
    }

    /// DataFusion function name. Session functions use the `kdb_` implementation.
    pub fn scalar_udf_name(self) -> &'static str {
        match self {
            Self::Now => "now",
            Self::CurrentUser => "kdb_current_user",
            Self::CurrentRole => "kdb_current_role",
            Self::CurrentSchema => "kdb_current_schema",
            Self::CurrentDatabase => "kdb_current_database",
            Self::SnowflakeId => "snowflake_id",
            Self::SerialId => "serial_id",
            Self::UuidV7 => "uuid_v7",
            Self::Ulid => "ulid",
        }
    }

    /// Primary-key generators. A repeated call is a different id.
    pub fn is_id_generator(self) -> bool {
        matches!(self, Self::SnowflakeId | Self::SerialId)
    }

    /// Each row needs its own value. Session functions stay fixed for the statement.
    pub fn is_volatile(self) -> bool {
        matches!(self, Self::SnowflakeId | Self::SerialId | Self::UuidV7 | Self::Ulid)
    }
}

/// Same shape as SQL `CALL schema.name(args...)`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RoutineCall {
    pub routine_id:      RoutineId,
    pub arguments:       Vec<CallArgument>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub named_arguments: Vec<Option<String>>,
}

impl RoutineCall {
    pub fn new(routine_id: RoutineId, arguments: Vec<CallArgument>) -> Self {
        Self {
            routine_id,
            arguments,
            named_arguments: Vec::new(),
        }
    }

    pub fn with_named_arguments(
        routine_id: RoutineId,
        arguments: Vec<CallArgument>,
        named_arguments: Vec<Option<String>>,
    ) -> Self {
        let has_any_named = named_arguments.iter().any(Option::is_some);
        Self {
            routine_id,
            arguments,
            named_arguments: if has_any_named {
                named_arguments
            } else {
                Vec::new()
            },
        }
    }

    pub fn builtin(name: &str) -> Self {
        Self::new(RoutineId::new(normalize_builtin_name(name)), Vec::new())
    }

    pub fn unqualified_name(&self) -> &str {
        self.routine_id
            .as_str()
            .split_once('.')
            .map(|(_, name)| name)
            .unwrap_or(self.routine_id.as_str())
    }

    pub fn builtin_default(&self) -> Option<BuiltinDefault> {
        if self.routine_id.as_str().contains('.') {
            return None;
        }
        BuiltinDefault::parse(self.unqualified_name())
    }

    pub fn is_builtin_default(&self) -> bool {
        self.builtin_default().is_some()
    }

    pub fn is_id_generator_default(&self) -> bool {
        self.builtin_default().is_some_and(BuiltinDefault::is_id_generator)
    }

    pub fn has_placeholder(&self) -> bool {
        self.arguments.iter().any(CallArgument::is_placeholder)
    }

    pub fn scalar_udf_name(&self) -> Option<&'static str> {
        self.builtin_default().map(BuiltinDefault::scalar_udf_name)
    }

    pub fn to_sql(&self) -> String {
        let name = if self.is_builtin_default() {
            self.unqualified_name().to_ascii_uppercase()
        } else {
            self.routine_id.as_str().to_string()
        };
        if self.arguments.is_empty() {
            format!("{name}()")
        } else {
            let args =
                self.arguments.iter().map(CallArgument::to_sql).collect::<Vec<_>>().join(", ");
            format!("{name}({args})")
        }
    }
}

pub fn is_builtin_default_name(name: &str) -> bool {
    BuiltinDefault::parse(name).is_some()
}

pub fn normalize_builtin_name(name: &str) -> String {
    BuiltinDefault::parse(name)
        .map(|builtin| builtin.canonical_name().to_string())
        .unwrap_or_else(|| name.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_now_is_unqualified() {
        let call = RoutineCall::builtin("NOW");
        assert_eq!(call.routine_id.as_str(), "now");
        assert!(call.is_builtin_default());
        assert_eq!(call.to_sql(), "NOW()");
        assert_eq!(call.scalar_udf_name(), Some("now"));
    }

    #[test]
    fn qualified_procedure_is_not_builtin() {
        let call = RoutineCall::new(
            RoutineId::from_parts(Some(&crate::models::SchemaId::new("chat")), "next_id"),
            Vec::new(),
        );
        assert!(!call.is_builtin_default());
        assert_eq!(call.to_sql(), "chat.next_id()");
    }

    #[test]
    fn qualified_snowflake_id_is_user_procedure() {
        let call = RoutineCall::new(
            RoutineId::from_parts(Some(&crate::models::SchemaId::new("app")), "snowflake_id"),
            Vec::new(),
        );
        assert!(!call.is_builtin_default());
        assert!(!call.is_id_generator_default());
    }

    #[test]
    fn postgresql_aliases_share_one_builtin() {
        assert_eq!(BuiltinDefault::parse("CURRENT_TIMESTAMP"), Some(BuiltinDefault::Now));
        assert_eq!(BuiltinDefault::parse("transaction_timestamp"), Some(BuiltinDefault::Now));
        assert_eq!(BuiltinDefault::parse("session_user"), Some(BuiltinDefault::CurrentUser));
        assert_eq!(BuiltinDefault::parse("current_catalog"), Some(BuiltinDefault::CurrentDatabase));
        assert_eq!(normalize_builtin_name("SESSION_USER"), "current_user");
        assert_eq!(
            RoutineCall::builtin("current_schema").scalar_udf_name(),
            Some("kdb_current_schema")
        );
        assert!(BuiltinDefault::parse("clock_timestamp").is_none());
        assert!(BuiltinDefault::parse("gen_random_uuid").is_none());
        assert!(RoutineCall::builtin("serial_id").is_id_generator_default());
        assert!(BuiltinDefault::SerialId.is_volatile());
        assert!(!BuiltinDefault::Now.is_volatile());
    }
}
