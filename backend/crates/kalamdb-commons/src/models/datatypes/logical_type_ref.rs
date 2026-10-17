//! Validated logical type references for columns, type fields, and signatures.

use std::{borrow::Cow, fmt};

use serde::{Deserialize, Serialize};

use super::KalamDataType;
use crate::models::TypeId;

/// One logical type: a builtin, `JSONB`, `TIMESTAMPTZ`, a named catalog type,
/// an array, or a reserved map.
///
/// Builtin variants keep [`KalamDataType`] directly with zero per-type modifier
/// overhead. `Jsonb` and `TimestampTz` are first-class variants on this enum so
/// invalid combinations (such as `jsonb` + `with_time_zone` or `jsonb` on `INT`)
/// are unrepresentable at compile time.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LogicalTypeRef {
    Builtin(KalamDataType),
    Jsonb,
    TimestampTz,
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
    #[inline]
    pub const fn from_builtin(data_type: KalamDataType) -> Self {
        Self::Builtin(data_type)
    }

    #[inline]
    pub const fn jsonb() -> Self {
        Self::Jsonb
    }

    #[inline]
    pub const fn timestamptz() -> Self {
        Self::TimestampTz
    }

    #[inline]
    pub const fn named(type_id: TypeId) -> Self {
        Self::Named { type_id }
    }

    /// Named catalog type used as a composite. The catalog row records the kind.
    #[inline]
    pub const fn composite(type_id: TypeId) -> Self {
        Self::named(type_id)
    }

    #[inline]
    pub fn list(element: LogicalTypeRef, element_nullable: bool) -> Self {
        Self::List {
            element: Box::new(element),
            element_nullable,
        }
    }

    /// Array of `element`. One element type covers every dimension.
    #[inline]
    pub fn array(element: LogicalTypeRef, element_nullable: bool) -> Self {
        Self::list(element, element_nullable)
    }

    #[inline]
    pub const fn peel_list(&self) -> &LogicalTypeRef {
        match self {
            Self::List { element, .. } => element.peel_list(),
            other => other,
        }
    }

    #[inline]
    pub const fn named_type_id(&self) -> Option<&TypeId> {
        match self {
            Self::Named { type_id } => Some(type_id),
            Self::List { element, .. } => element.named_type_id(),
            Self::Map { value, .. } => value.named_type_id(),
            Self::Builtin(_) | Self::Jsonb | Self::TimestampTz => None,
        }
    }

    #[inline]
    pub const fn element(&self) -> Option<&LogicalTypeRef> {
        match self {
            Self::List { element, .. } => Some(element),
            _ => None,
        }
    }

    #[inline]
    pub const fn element_nullable(&self) -> bool {
        match self {
            Self::List {
                element_nullable, ..
            } => *element_nullable,
            _ => true,
        }
    }

    /// Walk named TypeIds reachable from this reference (lists unwrap).
    pub fn referenced_type_ids(&self) -> Vec<TypeId> {
        let mut out = Vec::new();
        collect_type_ids(self, &mut out);
        out
    }

    #[inline]
    pub fn is_map(&self) -> bool {
        matches!(self, Self::Map { .. })
            || matches!(self, Self::List { element, .. } if element.is_map())
    }

    #[inline]
    pub const fn is_array(&self) -> bool {
        matches!(self, Self::List { .. })
    }

    #[inline]
    pub const fn is_jsonb(&self) -> bool {
        matches!(self.peel_list(), Self::Jsonb)
    }

    #[inline]
    pub const fn is_timestamptz(&self) -> bool {
        matches!(self.peel_list(), Self::TimestampTz)
    }

    /// Static modifier label (`"jsonb"` or `"timestamptz"`) when applicable.
    #[inline]
    pub const fn modifier_label(&self) -> Option<&'static str> {
        match self.peel_list() {
            Self::Jsonb => Some("jsonb"),
            Self::TimestampTz => Some("timestamptz"),
            _ => None,
        }
    }

    /// Payload tag. `Jsonb` maps to [`KalamDataType::Json`] and `TimestampTz`
    /// maps to [`KalamDataType::Timestamp`].
    #[inline]
    pub const fn builtin(&self) -> Option<KalamDataType> {
        match self {
            Self::Builtin(data_type) => Some(*data_type),
            Self::Jsonb => Some(KalamDataType::Json),
            Self::TimestampTz => Some(KalamDataType::Timestamp),
            Self::Named { .. } | Self::List { .. } | Self::Map { .. } => None,
        }
    }

    /// Borrowed SQL spelling when static, or owned when parameterized/composite.
    pub fn sql_name_cow(&self) -> Cow<'static, str> {
        match self {
            Self::Builtin(data_type) => match data_type.sql_name_static() {
                Some(static_name) => Cow::Borrowed(static_name),
                None => Cow::Owned(data_type.sql_name()),
            },
            Self::Jsonb => Cow::Borrowed("JSONB"),
            Self::TimestampTz => Cow::Borrowed("TIMESTAMPTZ"),
            Self::Named { type_id } => Cow::Owned(type_id.to_string()),
            Self::List {
                element,
                element_nullable,
            } => {
                if *element_nullable {
                    Cow::Owned(format!("{}[]", element.sql_name_cow()))
                } else {
                    Cow::Owned(format!("{}[] (element not null)", element.sql_name_cow()))
                }
            },
            Self::Map { key, value, .. } => {
                Cow::Owned(format!("MAP({}, {})", key.sql_name_cow(), value.sql_name_cow()))
            },
        }
    }

    /// SQL spelling, including `JSONB` and `TIMESTAMPTZ`.
    pub fn sql_name(&self) -> String {
        self.sql_name_cow().into_owned()
    }

    /// Serde default for list-element nullability on columns and type fields.
    #[inline]
    pub const fn default_element_nullable() -> bool {
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
                Some(data_type) => Self::from_builtin(data_type),
                None => return Err(format!("{owner} has no type")),
            },
        };
        if inner.is_map() {
            return Err(format!("{owner} cannot store MAP types"));
        }
        if is_array {
            Ok(Self::array(inner, element_nullable))
        } else {
            Ok(inner)
        }
    }
}

fn collect_type_ids(type_ref: &LogicalTypeRef, out: &mut Vec<TypeId>) {
    match type_ref {
        LogicalTypeRef::Builtin(_) | LogicalTypeRef::Jsonb | LogicalTypeRef::TimestampTz => {},
        LogicalTypeRef::Named { type_id } => out.push(type_id.clone()),
        LogicalTypeRef::List { element, .. } => collect_type_ids(element, out),
        LogicalTypeRef::Map { key, value, .. } => {
            collect_type_ids(key, out);
            collect_type_ids(value, out);
        },
    }
}

impl From<KalamDataType> for LogicalTypeRef {
    #[inline]
    fn from(value: KalamDataType) -> Self {
        Self::from_builtin(value)
    }
}

impl fmt::Display for LogicalTypeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Builtin(data_type) => write!(f, "{data_type}"),
            Self::Jsonb => f.write_str("JSONB"),
            Self::TimestampTz => f.write_str("TIMESTAMPTZ"),
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
        let id = TypeId::from_parts(Some(&crate::models::SchemaId::new("chat")), "address");
        let type_ref = LogicalTypeRef::list(LogicalTypeRef::named(id.clone()), true);
        assert_eq!(type_ref.referenced_type_ids(), vec![id]);
        assert!(type_ref.is_array());
        assert!(!type_ref.is_map());
    }

    #[test]
    fn stored_named_array_wraps_type_id() {
        let id = TypeId::from_parts(Some(&crate::models::SchemaId::new("chat")), "address");
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

    #[test]
    fn jsonb_and_timestamptz_keep_payloads_and_distinct_names() {
        let json = LogicalTypeRef::from_builtin(KalamDataType::Json);
        let jsonb = LogicalTypeRef::jsonb();
        assert_ne!(json, jsonb);
        assert_eq!(json.builtin(), jsonb.builtin());
        assert_eq!(json.sql_name(), "JSON");
        assert_eq!(jsonb.sql_name(), "JSONB");
        assert!(jsonb.is_jsonb());

        let timestamp = LogicalTypeRef::from_builtin(KalamDataType::Timestamp);
        let timestamptz = LogicalTypeRef::timestamptz();
        assert_ne!(timestamp, timestamptz);
        assert_eq!(timestamp.builtin(), timestamptz.builtin());
        assert_eq!(timestamp.sql_name(), "TIMESTAMP");
        assert_eq!(timestamptz.sql_name(), "TIMESTAMPTZ");
        assert!(timestamptz.is_timestamptz());
    }

    #[test]
    fn numeric_modifier_stays_on_the_decimal_payload() {
        let numeric = LogicalTypeRef::from_builtin(KalamDataType::Decimal {
            precision: 12,
            scale:     2,
        });
        assert_eq!(numeric.sql_name(), "DECIMAL(12, 2)");
        assert!(!numeric.is_jsonb());
        assert!(!numeric.is_timestamptz());
    }

    #[test]
    fn composite_array_is_one_named_list() {
        let id = TypeId::from_parts(Some(&crate::models::SchemaId::new("app")), "payment");
        let type_ref = LogicalTypeRef::array(LogicalTypeRef::composite(id.clone()), true);
        assert!(type_ref.is_array());
        assert_eq!(type_ref.named_type_id(), Some(&id));
        assert_eq!(type_ref.element().and_then(|element| element.named_type_id()), Some(&id));
        assert_eq!(type_ref.referenced_type_ids(), vec![id]);
    }
}
