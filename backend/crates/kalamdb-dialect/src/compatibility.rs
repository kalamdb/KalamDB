//! SQL dialect type-mapping helpers.
//!
//! Centralises conversion from `sqlparser` data types into [`KalamDataType`]
//! so CREATE TABLE / ALTER TABLE parsers stay in sync.

use std::string::String;

use kalamdb_commons::models::{datatypes::KalamDataType, SchemaId};
use sqlparser::ast::{
    ArrayElemTypeDef, DataType as SQLDataType, DataType::*, ObjectName, ObjectNamePart,
};

use crate::ddl::create_type::TypeReference;

fn map_decimal_kalam_type(info: &sqlparser::ast::ExactNumberInfo) -> Result<KalamDataType, String> {
    let (precision, scale) = match info {
        sqlparser::ast::ExactNumberInfo::PrecisionAndScale(precision, scale) => {
            (*precision as u8, *scale as u8)
        },
        sqlparser::ast::ExactNumberInfo::Precision(precision) => (*precision as u8, 0),
        sqlparser::ast::ExactNumberInfo::None => (38, 10),
    };

    KalamDataType::validate_decimal_params(precision, scale).map_err(|error| error.to_string())?;
    Ok(KalamDataType::Decimal { precision, scale })
}

fn custom_type_identifier(name: &ObjectName) -> String {
    name.0
        .iter()
        .map(|id| id.to_string().to_lowercase())
        .collect::<Vec<_>>()
        .join(".")
}

fn parse_embedding_dimension(modifiers: &[String]) -> Result<u16, String> {
    if modifiers.len() != 1 {
        return Err("EMBEDDING type requires exactly one dimension parameter, e.g., \
                    EMBEDDING(384)"
            .to_string());
    }

    let dim_str = &modifiers[0];
    let dim = dim_str
        .parse::<u16>()
        .map_err(|_| format!("EMBEDDING dimension must be a positive integer, got '{dim_str}'"))?;
    KalamDataType::validate_embedding_dimension(dim).map_err(|error| error.to_string())?;
    Ok(dim)
}

/// Map a parsed `sqlparser` data type into a [`KalamDataType`].
pub fn map_sql_type_to_kalam(sql_type: &SQLDataType) -> Result<KalamDataType, String> {
    match sql_type {
        SmallInt(_) | Int2(_) | TinyInt(_) => Ok(KalamDataType::SmallInt),
        Int(_) | Integer(_) | Int4(_) | MediumInt(_) => Ok(KalamDataType::Int),
        BigInt(_) | Int8(_) | Int64 => Ok(KalamDataType::BigInt),
        Float(_) | Real | Float4 => Ok(KalamDataType::Float),
        SQLDataType::Double(_) | DoublePrecision | Float8 | Float64 => Ok(KalamDataType::Double),
        Boolean | Bool => Ok(KalamDataType::Boolean),
        SQLDataType::JSON | SQLDataType::JSONB => Ok(KalamDataType::Json),
        Character(_)
        | Char(_)
        | CharacterVarying(_)
        | CharVarying(_)
        | Varchar(_)
        | Nvarchar(_)
        | CharacterLargeObject(_)
        | CharLargeObject(_)
        | Clob(_)
        | Text
        | String(_) => Ok(KalamDataType::Text),
        Binary(_) | Varbinary(_) | Blob(_) | Bytes(_) | Bytea => Ok(KalamDataType::Bytes),
        Date => Ok(KalamDataType::Date),
        Timestamp(_, _) => Ok(KalamDataType::Timestamp),
        Datetime(_) => Ok(KalamDataType::DateTime),
        Time(_, _) => Ok(KalamDataType::Time),
        SQLDataType::Uuid => Ok(KalamDataType::Uuid),
        Decimal(info) | Numeric(info) => map_decimal_kalam_type(info),
        Custom(name, modifiers) => map_custom_kalam_type(name, modifiers),
        Array(_) | Enum(_, _) | Set(_) | Struct(_, _) => Ok(KalamDataType::Text),
        other => Err(format!("Unsupported data type: {other:?}")),
    }
}

/// Map a parsed SQL type to a logical [`TypeReference`] (builtin, named, or list).
pub fn sql_type_to_type_reference(sql_type: &SQLDataType) -> Result<TypeReference, String> {
    match sql_type {
        Array(elem) => {
            let inner = match elem {
                ArrayElemTypeDef::None => {
                    return Err("ARRAY type requires an element type".to_string());
                },
                ArrayElemTypeDef::SquareBracket(inner, _)
                | ArrayElemTypeDef::AngleBracket(inner)
                | ArrayElemTypeDef::Parenthesis(inner) => inner.as_ref(),
            };
            let mut type_ref = sql_type_to_type_reference(inner)?;
            type_ref.is_array = true;
            Ok(type_ref)
        },
        Custom(name, modifiers) => {
            let ident = custom_type_identifier(name);
            if matches!(ident.as_str(), "embedding" | "pg_catalog.embedding") {
                return Ok(builtin_type_reference(map_custom_kalam_type(name, modifiers)?));
            }
            match map_custom_kalam_type(name, modifiers) {
                Ok(data_type) => {
                    let mut tr = builtin_type_reference(data_type);
                    if ident.eq_ignore_ascii_case("jsonb") {
                        tr.name = "JSONB".to_string();
                    } else if ident.eq_ignore_ascii_case("timestamptz") {
                        tr.name = "TIMESTAMPTZ".to_string();
                    } else if ident.eq_ignore_ascii_case("geometry") {
                        tr.name = "GEOMETRY".to_string();
                    }
                    Ok(tr)
                },
                Err(_) => Ok(named_type_reference(name)?),
            }
        },
        SQLDataType::JSONB => {
            let mut tr = builtin_type_reference(KalamDataType::Json);
            tr.name = "JSONB".to_string();
            Ok(tr)
        },
        Timestamp(_, tz)
            if matches!(
                tz,
                sqlparser::ast::TimezoneInfo::WithTimeZone | sqlparser::ast::TimezoneInfo::Tz
            ) =>
        {
            let mut tr = builtin_type_reference(KalamDataType::Timestamp);
            tr.name = "TIMESTAMPTZ".to_string();
            Ok(tr)
        },
        other => Ok(builtin_type_reference(map_sql_type_to_kalam(other)?)),
    }
}

fn builtin_type_reference(data_type: KalamDataType) -> TypeReference {
    TypeReference {
        namespace_id:       None,
        name:               data_type.sql_name().to_string(),
        data_type:          Some(data_type),
        syntactic_modifier: None,
        is_array:           false,
        not_null:           false,
        nonempty:           false,
    }
}

fn named_type_reference(name: &ObjectName) -> Result<TypeReference, String> {
    let mut parts = Vec::new();
    for part in &name.0 {
        match part {
            ObjectNamePart::Identifier(ident) => {
                if ident.quote_style.is_some() {
                    parts.push(ident.value.clone());
                } else {
                    parts.push(ident.value.to_ascii_lowercase());
                }
            },
            other => parts.push(other.to_string().to_ascii_lowercase()),
        }
    }
    let (namespace_id, type_name) = match parts.as_slice() {
        [type_name] => (None, type_name.clone()),
        [schema, type_name] => (Some(SchemaId::new(schema.clone())), type_name.clone()),
        _ => {
            return Err(format!(
                "named type '{}' must be unqualified or schema-qualified",
                parts.join(".")
            ));
        },
    };
    Ok(TypeReference {
        namespace_id,
        name: type_name,
        data_type: None,
        syntactic_modifier: None,
        is_array: false,
        not_null: false,
        nonempty: false,
    })
}

fn map_custom_kalam_type(name: &ObjectName, modifiers: &[String]) -> Result<KalamDataType, String> {
    if let Some(ObjectNamePart::Identifier(ident)) = name.0.last() {
        if ident.quote_style.is_some() && ident.value.chars().any(|c| c.is_ascii_uppercase()) {
            return Err(format!("Quoted custom type '{}' is not a builtin data type", ident.value));
        }
    }
    let ident = custom_type_identifier(name);
    match ident.as_str() {
        "file" | "pg_catalog.file" => Ok(KalamDataType::File),
        "embedding" | "pg_catalog.embedding" => {
            let dim = parse_embedding_dimension(modifiers)?;
            Ok(KalamDataType::Embedding(dim))
        },
        // serial/serial4 are int4 and default to serial_id(), so clients receive
        // a 32-bit integer. bigserial/serial8 stay bigint and use SNOWFLAKE_ID().
        "serial" | "serial4" => Ok(KalamDataType::Int),
        "bigserial" | "serial8" => Ok(KalamDataType::BigInt),
        "signed" => Ok(KalamDataType::Int),
        "smallserial" | "serial2" | "int1" | "int2" => Ok(KalamDataType::SmallInt),
        "int4" => Ok(KalamDataType::Int),
        "int8" => Ok(KalamDataType::BigInt),
        "tinytext" | "mediumtext" | "longtext" | "ntext" | "citext" | "geometry" => {
            Ok(KalamDataType::Text)
        },
        other => KalamDataType::from_sql_name(other)
            .ok_or_else(|| format!("Unsupported custom data type '{other}'")),
    }
}

/// PostgreSQL character maximum length. `character` with no length is 1.
/// `varchar` and `text` with no length stay unlimited.
pub fn character_maximum_length(data_type: &SQLDataType) -> Option<u32> {
    let (length, bare_character) = match data_type {
        Char(length) | Character(length) => (length.as_ref(), true),
        Varchar(length) | CharacterVarying(length) | CharVarying(length) | Nvarchar(length) => {
            (length.as_ref(), false)
        },
        _ => return None,
    };
    match length {
        Some(sqlparser::ast::CharacterLength::IntegerLength { length, .. }) => {
            u32::try_from(*length).ok()
        },
        None if bare_character => Some(1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use sqlparser::ast::Ident;

    use super::*;

    fn custom(name: &str) -> SQLDataType {
        SQLDataType::Custom(
            ObjectName(vec![sqlparser::ast::ObjectNamePart::Identifier(Ident::new(name))]),
            vec![],
        )
    }

    fn custom_with_size(name: &str, size: i32) -> SQLDataType {
        SQLDataType::Custom(
            ObjectName(vec![sqlparser::ast::ObjectNamePart::Identifier(Ident::new(name))]),
            vec![size.to_string()],
        )
    }

    #[test]
    fn maps_postgres_serial_types() {
        assert_eq!(map_sql_type_to_kalam(&custom("serial")).unwrap(), KalamDataType::Int);
        assert_eq!(map_sql_type_to_kalam(&custom("serial4")).unwrap(), KalamDataType::Int);
        assert_eq!(map_sql_type_to_kalam(&custom("serial8")).unwrap(), KalamDataType::BigInt);
        assert_eq!(map_sql_type_to_kalam(&custom("bigserial")).unwrap(), KalamDataType::BigInt);
        assert_eq!(map_sql_type_to_kalam(&custom("smallserial")).unwrap(), KalamDataType::SmallInt);
    }

    #[test]
    fn rejects_unknown_custom_types() {
        let err = map_sql_type_to_kalam(&custom("geography")).unwrap_err();
        assert!(err.contains("Unsupported custom data type"));
    }

    #[test]
    fn maps_embedding_type() {
        for dim in [384_u16, 768, 1536, 3072] {
            assert_eq!(
                map_sql_type_to_kalam(&custom_with_size("EMBEDDING", dim.into())).unwrap(),
                KalamDataType::Embedding(dim)
            );
        }
    }

    #[test]
    fn maps_sql_type_to_kalam() {
        let dtype = map_sql_type_to_kalam(&SQLDataType::Text).unwrap();
        assert_eq!(dtype, KalamDataType::Text);
    }

    #[test]
    fn maps_numeric_precision_and_scale_onto_decimal128() {
        let dtype = map_sql_type_to_kalam(&SQLDataType::Numeric(
            sqlparser::ast::ExactNumberInfo::PrecisionAndScale(12, 2),
        ))
        .unwrap();
        assert_eq!(
            dtype,
            KalamDataType::Decimal {
                precision: 12,
                scale:     2,
            }
        );
    }

    #[test]
    fn maps_file_custom_type_to_kalam() {
        let dtype = map_sql_type_to_kalam(&custom("file")).unwrap();
        assert_eq!(dtype, KalamDataType::File);
    }

    #[test]
    fn rejects_embedding_without_dimension() {
        let err = map_sql_type_to_kalam(&custom("EMBEDDING")).unwrap_err();
        assert!(err.contains("requires exactly one dimension parameter"));
    }

    #[test]
    fn rejects_embedding_dimension_zero() {
        let err = map_sql_type_to_kalam(&custom_with_size("EMBEDDING", 0)).unwrap_err();
        assert!(err.contains("between 1 and 8192"));
    }

    #[test]
    fn rejects_embedding_dimension_too_large() {
        let err = map_sql_type_to_kalam(&custom_with_size("EMBEDDING", 9000)).unwrap_err();
        assert!(err.contains("between 1 and 8192"));
    }

    #[test]
    fn geometry_subtype_stays_a_builtin_text_column() {
        let dtype = SQLDataType::Custom(
            ObjectName(vec![sqlparser::ast::ObjectNamePart::Identifier(Ident::new(
                "geometry",
            ))]),
            vec!["geometry".to_string(), "4326".to_string()],
        );
        let type_ref = sql_type_to_type_reference(&dtype).unwrap();
        assert_eq!(type_ref.name, "GEOMETRY");
        assert_eq!(type_ref.builtin_data_type(), Some(KalamDataType::Text));
        assert!(type_ref.namespace_id.is_none());
    }

    #[test]
    fn character_limits_follow_postgres() {
        use sqlparser::ast::CharacterLength;
        let varchar_255 = SQLDataType::Varchar(Some(CharacterLength::IntegerLength {
            length: 255,
            unit:   None,
        }));
        assert_eq!(character_maximum_length(&varchar_255), Some(255));
        assert_eq!(character_maximum_length(&SQLDataType::Character(None)), Some(1));
        assert_eq!(character_maximum_length(&SQLDataType::CharacterVarying(None)), None);
        assert_eq!(character_maximum_length(&SQLDataType::Text), None);
    }
}
