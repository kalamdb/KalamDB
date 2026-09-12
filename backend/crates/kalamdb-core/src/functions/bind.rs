//! Contract argument and return validation for procedure CALL.

use kalamdb_commons::{
    models::{CatalogTypeKind, RoutineId, TypeId},
    KalamDataType,
};
use kalamdb_functions::{FunctionsError, RoutineValue};
use kalamdb_system::{CatalogRoutine, CatalogRoutineParameter, CatalogStores, CatalogTypeField};
use serde_json::Value as JsonValue;

use super::convert::{json_to_routine_value, routine_value_as_json};
use crate::error::KalamDbError;

struct TypeSpec {
    display:   String,
    type_id:   Option<TypeId>,
    data_type: Option<KalamDataType>,
    is_array:  bool,
    not_null:  bool,
    nonempty:  bool,
}

impl TypeSpec {
    fn from_parameter(parameter: &CatalogRoutineParameter) -> Self {
        Self {
            display:   format!("argument '{}'", parameter.name),
            type_id:   parameter.type_id.clone(),
            data_type: parameter.builtin_data_type(),
            is_array:  parameter.is_array,
            not_null:  parameter.not_null,
            nonempty:  parameter.nonempty,
        }
    }

    fn from_return(routine: &CatalogRoutine) -> Option<Self> {
        if routine.return_type_id.is_none()
            && routine.return_data_type.is_none()
            && !routine.return_not_null
            && !routine.return_is_array
        {
            return None;
        }
        Some(Self {
            display:   "return value".to_string(),
            type_id:   routine.return_type_id.clone(),
            data_type: routine.return_data_type.clone(),
            is_array:  routine.return_is_array,
            not_null:  routine.return_not_null,
            nonempty:  false,
        })
    }

    fn element(&self) -> Self {
        Self {
            display:   format!("{} element", self.display),
            type_id:   self.type_id.clone(),
            data_type: self.data_type.clone(),
            is_array:  false,
            not_null:  false,
            nonempty:  false,
        }
    }
}

pub(super) fn validate_call_arguments(
    stores: &CatalogStores,
    routine_id: &RoutineId,
    args: &[RoutineValue],
) -> Result<(), KalamDbError> {
    let params = stores.list_parameters(routine_id).map_err(|error| {
        KalamDbError::CatalogError(format!(
            "failed to load parameters for procedure {routine_id}: {error}"
        ))
    })?;
    let positional = positional_args(routine_id, &params, args)?;
    for (parameter, value) in params.iter().zip(positional.iter()) {
        validate_spec(stores, &TypeSpec::from_parameter(parameter), value)?;
    }
    Ok(())
}

pub(super) fn validate_call_return(
    stores: &CatalogStores,
    routine: &CatalogRoutine,
    value: &RoutineValue,
) -> Result<(), KalamDbError> {
    let Some(spec) = TypeSpec::from_return(routine) else {
        return Ok(());
    };
    validate_spec(stores, &spec, value)
}

fn positional_args(
    routine_id: &RoutineId,
    params: &[CatalogRoutineParameter],
    args: &[RoutineValue],
) -> Result<Vec<RoutineValue>, KalamDbError> {
    if params.is_empty() {
        if args.is_empty() || (args.len() == 1 && is_empty_object(&args[0])) {
            return Ok(Vec::new());
        }
        return Err(invalid_args(format!(
            "procedure {routine_id} expected 0 arguments, got {}",
            args.len()
        )));
    }
    if is_named_record(args, params) {
        return extract_named(&args[0], params);
    }
    if args.len() != params.len() {
        return Err(invalid_args(format!(
            "procedure {routine_id} expected {} argument(s), got {}",
            params.len(),
            args.len()
        )));
    }
    Ok(args.to_vec())
}

fn is_named_record(args: &[RoutineValue], params: &[CatalogRoutineParameter]) -> bool {
    if args.len() != 1 {
        return false;
    }
    let Some(JsonValue::Object(map)) = structured_json(&args[0]) else {
        return false;
    };
    params.iter().all(|parameter| map.contains_key(&parameter.name))
}

fn extract_named(
    arg: &RoutineValue,
    params: &[CatalogRoutineParameter],
) -> Result<Vec<RoutineValue>, KalamDbError> {
    let Some(JsonValue::Object(map)) = structured_json(arg) else {
        return Err(invalid_args("expected a named argument object".to_string()));
    };
    let mut values = Vec::with_capacity(params.len());
    for parameter in params {
        let json = map.get(&parameter.name).unwrap_or(&JsonValue::Null);
        values.push(json_to_routine_value(json, parameter.builtin_data_type().as_ref())?);
    }
    Ok(values)
}

fn validate_spec(
    stores: &CatalogStores,
    spec: &TypeSpec,
    value: &RoutineValue,
) -> Result<(), KalamDbError> {
    let json = structured_json(value).unwrap_or(JsonValue::Null);
    validate_json(stores, spec, &json)
}

fn validate_json(
    stores: &CatalogStores,
    spec: &TypeSpec,
    json: &JsonValue,
) -> Result<(), KalamDbError> {
    if json.is_null() {
        if spec.not_null {
            return Err(invalid_args(format!("{} cannot be null", spec.display)));
        }
        return Ok(());
    }
    if spec.is_array {
        let JsonValue::Array(items) = json else {
            return Err(invalid_args(format!("{} must be an array", spec.display)));
        };
        if spec.nonempty && items.is_empty() {
            return Err(invalid_args(format!("{} cannot be empty", spec.display)));
        }
        let element = spec.element();
        for item in items {
            validate_json(stores, &element, item)?;
        }
        return Ok(());
    }
    if let Some(type_id) = &spec.type_id {
        return validate_named_type(stores, spec, type_id, json);
    }
    if spec.nonempty {
        match json {
            JsonValue::String(text) if text.is_empty() => {
                return Err(invalid_args(format!("{} cannot be empty", spec.display)));
            },
            JsonValue::Array(items) if items.is_empty() => {
                return Err(invalid_args(format!("{} cannot be empty", spec.display)));
            },
            _ => {},
        }
    }
    Ok(())
}

fn validate_named_type(
    stores: &CatalogStores,
    spec: &TypeSpec,
    type_id: &TypeId,
    json: &JsonValue,
) -> Result<(), KalamDbError> {
    let catalog_type = stores
        .get_type(type_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        .ok_or_else(|| KalamDbError::NotFound(format!("type {type_id} not found")))?;
    let fields = load_type_fields(stores, &catalog_type)?;
    match catalog_type.kind {
        CatalogTypeKind::Enum => {
            let JsonValue::String(label) = json else {
                return Err(invalid_args(format!(
                    "{} must be an enum label of {type_id}",
                    spec.display
                )));
            };
            if fields.iter().any(|field| field.name == *label) {
                Ok(())
            } else {
                Err(invalid_args(format!("invalid enum label '{label}' for {}", spec.display)))
            }
        },
        CatalogTypeKind::Composite
        | CatalogTypeKind::ImplicitTableRow
        | CatalogTypeKind::RowAlias => {
            let JsonValue::Object(map) = json else {
                return Err(invalid_args(format!("{} must be an object", spec.display)));
            };
            for field in fields {
                let child = TypeSpec {
                    display:   format!("{} field '{}'", spec.display, field.name),
                    type_id:   field.field_type_id.clone(),
                    data_type: field.builtin_data_type(),
                    is_array:  field.is_array,
                    not_null:  field.not_null,
                    nonempty:  field.nonempty,
                };
                let value = map.get(&field.name).unwrap_or(&JsonValue::Null);
                validate_json(stores, &child, value)?;
            }
            Ok(())
        },
        CatalogTypeKind::TopicPayload => {
            let JsonValue::Object(map) = json else {
                return Err(invalid_args(format!("{} must be an object", spec.display)));
            };
            match map.get("_table") {
                Some(JsonValue::String(table)) if !table.is_empty() => Ok(()),
                _ => Err(invalid_args(format!(
                    "{} must include a non-empty '_table' discriminator",
                    spec.display
                ))),
            }
        },
    }
}

fn load_type_fields(
    stores: &CatalogStores,
    catalog_type: &kalamdb_system::CatalogType,
) -> Result<Vec<CatalogTypeField>, KalamDbError> {
    if catalog_type.kind == CatalogTypeKind::RowAlias {
        if let Some(source) = &catalog_type.source_type_id {
            return stores
                .list_type_fields(source)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()));
        }
    }
    stores
        .list_type_fields(&catalog_type.type_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))
}

fn structured_json(value: &RoutineValue) -> Option<JsonValue> {
    let json = routine_value_as_json(value)?;
    if let JsonValue::String(text) = &json {
        if let Ok(parsed) = serde_json::from_str::<JsonValue>(text) {
            if parsed.is_object() || parsed.is_array() {
                return Some(parsed);
            }
        }
    }
    Some(json)
}

fn is_empty_object(value: &RoutineValue) -> bool {
    matches!(structured_json(value), Some(JsonValue::Object(map)) if map.is_empty())
        || matches!(routine_value_as_json(value), Some(JsonValue::Null))
}

fn invalid_args(message: String) -> KalamDbError {
    FunctionsError::InvalidArguments(message).into()
}

#[cfg(test)]
mod tests {
    use datafusion::scalar::ScalarValue;
    use kalamdb_functions::RoutineValue;
    use serde_json::json;

    use super::{is_empty_object, structured_json};
    use crate::functions::convert::{json_to_routine_value, routine_value_as_json};

    #[test]
    fn objects_bind_as_json_sql() {
        let value = json_to_routine_value(&json!({"city": "Paris"}), None).unwrap();
        assert!(value.json_sql);
        let parsed = routine_value_as_json(&value).unwrap();
        assert_eq!(parsed["city"], "Paris");
    }

    #[test]
    fn structured_json_parses_utf8_object_fallback() {
        let value = RoutineValue::new(ScalarValue::Utf8(Some(r#"{"city":"Paris"}"#.into())));
        let parsed = structured_json(&value).unwrap();
        assert_eq!(parsed["city"], "Paris");
    }

    #[test]
    fn empty_object_is_detected() {
        let value = json_to_routine_value(&json!({}), None).unwrap();
        assert!(is_empty_object(&value));
    }
}
