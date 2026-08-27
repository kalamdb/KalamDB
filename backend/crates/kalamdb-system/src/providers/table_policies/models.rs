use kalamdb_commons::{
    datatypes::KalamDataType, PolicyCommand, PolicyId, PolicyProgram, PolicyTarget, TableId,
    TablePolicy,
};
use kalamdb_macros::table;
use serde::{Deserialize, Serialize};

/// Storage/catalog representation of a compiled table policy.
#[table(
    name = "table_policies",
    comment = "Shared-table row-level security policies"
)]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TablePolicyRecord {
    #[column(
        id = 1,
        ordinal = 1,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = true,
        default = "None",
        comment = "Namespace-qualified table and policy identifier"
    )]
    pub policy_id:         PolicyId,
    #[column(
        id = 2,
        ordinal = 2,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "Protected table identifier"
    )]
    pub table_id:          TableId,
    #[column(
        id = 3,
        ordinal = 3,
        data_type(KalamDataType::Text),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "Policy name"
    )]
    pub policy_name:       String,
    #[column(
        id = 4,
        ordinal = 4,
        data_type(KalamDataType::Json),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "Policy command"
    )]
    pub command:           PolicyCommand,
    #[column(
        id = 5,
        ordinal = 5,
        data_type(KalamDataType::Json),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "Policy role targets"
    )]
    pub targets:           Vec<PolicyTarget>,
    #[column(
        id = 6,
        ordinal = 6,
        data_type(KalamDataType::Text),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "Original USING expression"
    )]
    pub using_sql:         Option<String>,
    #[column(
        id = 7,
        ordinal = 7,
        data_type(KalamDataType::Text),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "Original WITH CHECK expression"
    )]
    pub with_check_sql:    Option<String>,
    #[column(
        id = 8,
        ordinal = 8,
        data_type(KalamDataType::Json),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "Compiled USING authorization IR"
    )]
    pub using_program:     Option<PolicyProgram>,
    #[column(
        id = 9,
        ordinal = 9,
        data_type(KalamDataType::Json),
        nullable = true,
        primary_key = false,
        default = "None",
        comment = "Compiled WITH CHECK authorization IR"
    )]
    pub check_program:     Option<PolicyProgram>,
    #[column(
        id = 10,
        ordinal = 10,
        data_type(KalamDataType::BigInt),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "Monotonic policy generation for the protected table"
    )]
    pub policy_generation: u64,
    #[column(
        id = 11,
        ordinal = 11,
        data_type(KalamDataType::BigInt),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "Protected-table schema generation used at compilation"
    )]
    pub schema_generation: u64,
    #[column(
        id = 12,
        ordinal = 12,
        data_type(KalamDataType::Json),
        nullable = false,
        primary_key = false,
        default = "None",
        comment = "Tables whose mutations invalidate authorization"
    )]
    pub dependencies:      Vec<TableId>,
}

impl From<TablePolicy> for TablePolicyRecord {
    fn from(policy: TablePolicy) -> Self {
        let dependencies = policy
            .using_program
            .iter()
            .chain(policy.check_program.iter())
            .filter_map(|program| match program {
                PolicyProgram::AuthorizationRelation(relation) => Some(&relation.dependencies),
                PolicyProgram::RowLocal { .. } => None,
            })
            .flatten()
            .cloned()
            .collect();
        Self {
            policy_id: policy.policy_id,
            table_id: policy.table_id,
            policy_name: policy.policy_name,
            command: policy.command,
            targets: policy.targets,
            using_sql: policy.using_sql,
            with_check_sql: policy.with_check_sql,
            using_program: policy.using_program,
            check_program: policy.check_program,
            policy_generation: policy.policy_generation,
            schema_generation: policy.schema_generation,
            dependencies,
        }
    }
}

impl From<TablePolicyRecord> for TablePolicy {
    fn from(record: TablePolicyRecord) -> Self {
        TablePolicy::new(
            record.policy_id,
            record.table_id,
            record.policy_name,
            record.command,
            record.targets,
            record.using_sql,
            record.with_check_sql,
            record.using_program,
            record.check_program,
            record.policy_generation,
            record.schema_generation,
        )
    }
}
