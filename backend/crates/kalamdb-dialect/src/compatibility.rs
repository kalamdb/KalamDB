//! SQL dialect type-mapping helpers.
//!
//! Centralises conversion from `sqlparser` data types into [`KalamDataType`]
//! so CREATE TABLE / ALTER TABLE parsers stay in sync.

use std::string::String;

use kalamdb_commons::models::datatypes::KalamDataType;
use sqlparser::ast::{DataType as SQLDataType, DataType::*, ObjectName};

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
        Decimal(info) => map_decimal_kalam_type(info),
        Custom(name, modifiers) => map_custom_kalam_type(name, modifiers),
        Array(_) | Enum(_, _) | Set(_) | Struct(_, _) => Ok(KalamDataType::Text),
        other => Err(format!("Unsupported data type: {other:?}")),
    }
}

fn map_custom_kalam_type(name: &ObjectName, modifiers: &[String]) -> Result<KalamDataType, String> {
    let ident = custom_type_identifier(name);
    match ident.as_str() {
        "file" => Ok(KalamDataType::File),
        "embedding" => {
            let dim = parse_embedding_dimension(modifiers)?;
            Ok(KalamDataType::Embedding(dim))
        },
        "serial" | "serial4" | "signed" => Ok(KalamDataType::Int),
        "bigserial" | "serial8" => Ok(KalamDataType::BigInt),
        "smallserial" | "serial2" | "int1" | "int2" => Ok(KalamDataType::SmallInt),
        "int4" => Ok(KalamDataType::Int),
        "int8" => Ok(KalamDataType::BigInt),
        other if other.ends_with("text") || other.ends_with("string") => Ok(KalamDataType::Text),
        other => KalamDataType::from_sql_name(other)
            .ok_or_else(|| format!("Unsupported custom data type '{other}'")),
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
        assert_eq!(map_sql_type_to_kalam(&custom("serial8")).unwrap(), KalamDataType::BigInt);
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
}
