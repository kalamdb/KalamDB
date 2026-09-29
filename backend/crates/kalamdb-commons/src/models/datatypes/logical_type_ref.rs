//! Validated logical type references for columns, type fields, and signatures.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::KalamDataType;
use crate::models::TypeId;

/// One logical type: a builtin, a named catalog type, a list, or a reserved map.
///
/// Named types are identified by opaque [`TypeId`], not SQL names. Maps are
/// reserved and rejected for stored columns in v1.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LogicalTypeRef {
    Builtin(KalamDataType),
    Named {
        type_id: TypeId,
    },
    List {
        element:          Box<LogicalTypeRef>,
        element_nullable: bool,
    },
    /// Reserved. Not a legal stored column type.
    Map {
        key:            Box<LogicalTypeRef>,
        value:          Box<LogicalTypeRef>,
        value_nullable: bool,
    },
}

impl LogicalTypeRef {
    pub fn named(type_id: TypeId) -> Self {
        Self::Named { type_id }
    }

    pub fn list(element: LogicalTypeRef, element_nullable: bool) -> Self {
        Self::List {
            element: Box::new(element),
            element_nullable,
        }
    }

    pub fn named_type_id(&self) -> Option<&TypeId> {
        match self {
            Self::Named { type_id } => Some(type_id),
            Self::List { element, .. } => element.named_type_id(),
            Self::Map { value, .. } => value.named_type_id(),
            Self::Builtin(_) => None,
        }
    }

    /// Walk named TypeIds reachable from this reference (lists unwrap).
    pub fn referenced_type_ids(&self) -> Vec<TypeId> {
        let mut out = Vec::new();
        collect_type_ids(self, &mut out);
        out
    }

    pub fn is_map(&self) -> bool {
        matches!(self, Self::Map { .. })
            || matches!(self, Self::List { element, .. } if element.is_map())
    }

    pub fn is_array(&self) -> bool {
        matches!(self, Self::List { .. })
    }

    pub fn builtin(&self) -> Option<KalamDataType> {
        match self {
            Self::Builtin(data_type) => Some(*data_type),
            _ => None,
        }
    }

    /// Serde default for list-element nullability on columns and type fields.
    pub fn default_element_nullable() -> bool {
        true
    }

    /// Shared constructor for table columns and `CREATE TYPE` fields.
    ///
    /// Named [`TypeId`] wins over a builtin. Maps are rejected for stored values.
    pub fn stored(
        owner: &str,
        named_type_id: Option<&TypeId>,
        builtin: Option<KalamDataType>,
        is_array: bool,
        element_nullable: bool,
    ) -> Result<Self, String> {
        let inner = match named_type_id {
            Some(type_id) => Self::named(type_id.clone()),
            None => match builtin {
                Some(data_type) => Self::Builtin(data_type),
                None => return Err(format!("{owner} has no type")),
            },
        };
        if inner.is_map() {
            return Err(format!("{owner} cannot store MAP types"));
        }
        if is_array {
            Ok(Self::list(inner, element_nullable))
        } else {
            Ok(inner)
        }
    }
}

fn collect_type_ids(type_ref: &LogicalTypeRef, out: &mut Vec<TypeId>) {
    match type_ref {
        LogicalTypeRef::Builtin(_) => {},
        LogicalTypeRef::Named { type_id } => out.push(type_id.clone()),
        LogicalTypeRef::List { element, .. } => collect_type_ids(element, out),
        LogicalTypeRef::Map { key, value, .. } => {
            collect_type_ids(key, out);
            collect_type_ids(value, out);
        },
    }
}

impl fmt::Display for LogicalTypeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Builtin(data_type) => f.write_str(&data_type.sql_name()),
            Self::Named { type_id } => write!(f, "{type_id}"),
            Self::List {
                element,
                element_nullable,
            } => {
                write!(f, "{element}[]")?;
                if !*element_nullable {
                    f.write_str(" (element not null)")?;
                }
                Ok(())
            },
            Self::Map { key, value, .. } => write!(f, "MAP({key}, {value})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::datatypes::KalamDataType;

    #[test]
    fn list_of_named_reports_type_id() {
        let id = TypeId::from_parts(Some(&crate::models::NamespaceId::new("chat")), "address");
        let type_ref = LogicalTypeRef::list(LogicalTypeRef::named(id.clone()), true);
        assert_eq!(type_ref.referenced_type_ids(), vec![id]);
        assert!(type_ref.is_array());
        assert!(!type_ref.is_map());
    }

    #[test]
    fn stored_named_array_wraps_type_id() {
        let id = TypeId::from_parts(Some(&crate::models::NamespaceId::new("chat")), "address");
        let type_ref = LogicalTypeRef::stored(
            "column 'addr'",
            Some(&id),
            Some(KalamDataType::Text),
            true,
            false,
        )
        .expect("stored type");
        assert_eq!(type_ref.named_type_id(), Some(&id));
        assert!(type_ref.is_array());
        assert_eq!(type_ref.to_string(), format!("{id}[] (element not null)"));
    }

    #[test]
    fn stored_requires_a_type() {
        let error = LogicalTypeRef::stored("field 'x'", None, None, false, true).unwrap_err();
        assert!(error.contains("has no type"));
    }
}
