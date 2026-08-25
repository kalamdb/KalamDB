use sqlparser::ast::CreatePolicyCommand;

/// SQL commands to which a table policy applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyCommand {
    All,
    Select,
    Insert,
    Update,
    Delete,
}

impl From<Option<CreatePolicyCommand>> for PolicyCommand {
    fn from(command: Option<CreatePolicyCommand>) -> Self {
        match command {
            None | Some(CreatePolicyCommand::All) => Self::All,
            Some(CreatePolicyCommand::Select) => Self::Select,
            Some(CreatePolicyCommand::Insert) => Self::Insert,
            Some(CreatePolicyCommand::Update) => Self::Update,
            Some(CreatePolicyCommand::Delete) => Self::Delete,
        }
    }
}
