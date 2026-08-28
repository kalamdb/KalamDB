#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{PolicyCommand, PolicyId, PolicyProgram, PolicyTarget};
use crate::TableId;

/// Persisted table-policy definition and compiled authorization metadata.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TablePolicy {
    pub policy_id:         PolicyId,
    pub table_id:          TableId,
    pub policy_name:       String,
    pub command:           PolicyCommand,
    pub targets:           Vec<PolicyTarget>,
    pub using_sql:         Option<String>,
    pub with_check_sql:    Option<String>,
    pub using_program:     Option<PolicyProgram>,
    pub check_program:     Option<PolicyProgram>,
    pub policy_generation: u64,
    pub schema_generation: u64,
}

impl TablePolicy {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        policy_id: PolicyId,
        table_id: TableId,
        policy_name: impl Into<String>,
        command: PolicyCommand,
        targets: Vec<PolicyTarget>,
        using_sql: Option<String>,
        with_check_sql: Option<String>,
        using_program: Option<PolicyProgram>,
        check_program: Option<PolicyProgram>,
        policy_generation: u64,
        schema_generation: u64,
    ) -> Self {
        Self {
            policy_id,
            table_id,
            policy_name: policy_name.into(),
            command,
            targets,
            using_sql,
            with_check_sql,
            using_program,
            check_program,
            policy_generation,
            schema_generation,
        }
    }

    pub fn using_program_for(&self, command: PolicyCommand) -> Option<&PolicyProgram> {
        if self.command.applies_to(command)
            && matches!(
                command,
                PolicyCommand::Select | PolicyCommand::Update | PolicyCommand::Delete
            )
        {
            self.using_program.as_ref()
        } else {
            None
        }
    }

    pub fn check_program_for(&self, command: PolicyCommand) -> Option<&PolicyProgram> {
        if self.command.applies_to(command)
            && matches!(command, PolicyCommand::Insert | PolicyCommand::Update)
        {
            self.check_program.as_ref()
        } else {
            None
        }
    }
}
