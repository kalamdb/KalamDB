#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// SQL commands to which a policy applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "lowercase")
)]
pub enum PolicyCommand {
    All,
    Select,
    Insert,
    Update,
    Delete,
}

impl PolicyCommand {
    pub fn applies_to(self, command: Self) -> bool {
        self == Self::All || self == command
    }

    pub fn as_sql(self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Select => "SELECT",
            Self::Insert => "INSERT",
            Self::Update => "UPDATE",
            Self::Delete => "DELETE",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PolicyCommand;

    #[test]
    fn as_sql_matches_create_policy_command_keywords() {
        assert_eq!(PolicyCommand::All.as_sql(), "ALL");
        assert_eq!(PolicyCommand::Select.as_sql(), "SELECT");
        assert_eq!(PolicyCommand::Insert.as_sql(), "INSERT");
        assert_eq!(PolicyCommand::Update.as_sql(), "UPDATE");
        assert_eq!(PolicyCommand::Delete.as_sql(), "DELETE");
    }
}
