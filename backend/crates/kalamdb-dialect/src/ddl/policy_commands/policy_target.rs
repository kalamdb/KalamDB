use kalamdb_commons::Role;
use sqlparser::ast::Owner;

use crate::ddl::DdlResult;

/// A role class targeted by a table policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyTarget {
    Public,
    Role(Role),
}

impl TryFrom<Owner> for PolicyTarget {
    type Error = String;

    fn try_from(owner: Owner) -> DdlResult<Self> {
        let Owner::Ident(identifier) = owner else {
            return Err(format!("unsupported policy target '{owner}'"));
        };
        if identifier.value.eq_ignore_ascii_case("PUBLIC") {
            return Ok(Self::Public);
        }
        Role::from_str_opt(&identifier.value)
            .map(Self::Role)
            .ok_or_else(|| format!("unsupported policy target '{}'", identifier.value))
    }
}
