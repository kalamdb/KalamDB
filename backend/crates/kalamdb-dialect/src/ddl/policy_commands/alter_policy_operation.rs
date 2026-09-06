use super::PolicyTarget;

/// Mutation supported by `ALTER POLICY`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlterPolicyOperation {
    Rename {
        new_name: String,
    },
    Apply {
        targets:        Option<Vec<PolicyTarget>>,
        using_sql:      Option<String>,
        with_check_sql: Option<String>,
    },
}
