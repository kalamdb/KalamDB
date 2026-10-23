//! Contract argument and return validation for procedure CALL.

use std::sync::Arc;

use dashmap::DashMap;
use kalamdb_commons::{
    models::{CatalogTypeKind, RoutineCall, RoutineId, RoutineKind, RoutineParameterMode, TypeId},
    KalamDataType,
};
use kalamdb_functions::{FunctionsError, RoutineValue};
use kalamdb_serialization::{digest_to_hex, sha256_digest, DomainValue, DOMAIN_EPOCH};
use kalamdb_sql::ddl::column_default::parse_call_argument_sql;
use kalamdb_system::{CatalogRoutine, CatalogRoutineParameter, CatalogStores};
use once_cell::sync::Lazy;
use serde_json::Value as JsonValue;

use super::convert::{
    bind_call_arguments, domain_value_to_routine_value, json_to_routine_value,
    routine_value_as_json,
};
use crate::{error::KalamDbError, schema_registry::TypeRegistry};

const MAX_PINNED_DESCRIPTOR_CACHE: usize = 1_024;

/// Immutable revision-pinned routine descriptor resolved from authoritative metadata (`T052`,
/// `FR-042`).
///
/// Caches signature, parameter modes, and revision digest, but never caches evaluated runtime
/// parameter default values.
#[derive(Debug, Clone)]
pub struct PinnedRoutineDescriptor {
    pub epoch:            u16,
    pub routine_id:       RoutineId,
    pub revision_hash:    [u8; 32],
    pub revision_hex:     String,
    pub routine:          CatalogRoutine,
    pub parameters:       Arc<[CatalogRoutineParameter]>,
    pub input_parameters: Arc<[CatalogRoutineParameter]>,
    pub out_parameters:   Arc<[CatalogRoutineParameter]>,
}

static PINNED_DESCRIPTOR_CACHE: Lazy<DashMap<RoutineId, PinnedRoutineDescriptor>> =
    Lazy::new(DashMap::new);

/// Resolve and cache an immutable [`PinnedRoutineDescriptor`] from authoritative catalog metadata
/// (`T052`).
pub fn resolve_pinned_descriptor(
    stores: &CatalogStores,
    routine_id: &RoutineId,
    module_generation: u64,
) -> Result<PinnedRoutineDescriptor, KalamDbError> {
    let Some(routine) = stores.get_routine(routine_id).map_err(|error| {
        KalamDbError::CatalogError(format!("failed to load routine {routine_id}: {error}"))
    })?
    else {
        PINNED_DESCRIPTOR_CACHE.remove(routine_id);
        return Err(FunctionsError::UnknownProcedure(routine_id.to_string()).into());
    };
    let parameters = stores.list_parameters(routine_id).map_err(|error| {
        KalamDbError::CatalogError(format!(
            "failed to load parameters for routine {routine_id}: {error}"
        ))
    })?;
    let rev_hash = compute_descriptor_revision(&routine, &parameters, module_generation);
    if let Some(cached) = PINNED_DESCRIPTOR_CACHE.get(routine_id) {
        if cached.revision_hash == rev_hash {
            return Ok(cached.clone());
        }
    }
    let input_params: Vec<CatalogRoutineParameter> = parameters
        .iter()
        .filter(|param| {
            matches!(
                param.mode,
                RoutineParameterMode::In
                    | RoutineParameterMode::InOut
                    | RoutineParameterMode::Variadic
            )
        })
        .cloned()
        .collect();
    let out_params: Vec<CatalogRoutineParameter> = parameters
        .iter()
        .filter(|param| {
            matches!(param.mode, RoutineParameterMode::Out | RoutineParameterMode::InOut)
        })
        .cloned()
        .collect();
    let rev_hex = digest_to_hex(&rev_hash);
    let descriptor = PinnedRoutineDescriptor {
        epoch: DOMAIN_EPOCH,
        routine_id: routine_id.clone(),
        revision_hash: rev_hash,
        revision_hex: rev_hex,
        routine,
        parameters: Arc::from(parameters),
        input_parameters: Arc::from(input_params),
        out_parameters: Arc::from(out_params),
    };
    if PINNED_DESCRIPTOR_CACHE.len() >= MAX_PINNED_DESCRIPTOR_CACHE {
        PINNED_DESCRIPTOR_CACHE.clear();
    }
    PINNED_DESCRIPTOR_CACHE.insert(routine_id.clone(), descriptor.clone());
    Ok(descriptor)
}

fn compute_descriptor_revision(
    routine: &CatalogRoutine,
    parameters: &[CatalogRoutineParameter],
    module_generation: u64,
) -> [u8; 32] {
    let mut buf = Vec::with_capacity(256);
    buf.extend_from_slice(b"KALAM_ROUTINE_DESCRIPTOR_V1");
    buf.extend_from_slice(&DOMAIN_EPOCH.to_le_bytes());
    buf.extend_from_slice(&module_generation.to_le_bytes());
    buf.extend_from_slice(routine.routine_id.as_str().as_bytes());
    buf.extend_from_slice(routine.kind.as_str().as_bytes());
    buf.extend_from_slice(routine.security.as_str().as_bytes());
    buf.extend_from_slice(routine.volatility.as_str().as_bytes());
    buf.extend_from_slice(routine.null_safety.as_str().as_bytes());
    if let Some(lang) = &routine.language {
        buf.extend_from_slice(lang.as_bytes());
    }
    if let Some(src_hash) = &routine.inline_source_hash {
        buf.extend_from_slice(src_hash.as_bytes());
    }
    if let Some(ret_ty) = &routine.return_type_name {
        buf.extend_from_slice(ret_ty.as_bytes());
    }
    for param in parameters {
        buf.extend_from_slice(&param.ordinal.to_le_bytes());
        buf.extend_from_slice(param.name.as_bytes());
        buf.extend_from_slice(param.type_name.as_bytes());
        buf.extend_from_slice(&[u8::from(param.is_array), u8::from(param.not_null)]);
        buf.extend_from_slice(param.mode.as_str().as_bytes());
        if let Some(def) = &param.default_expr {
            buf.extend_from_slice(def.as_bytes());
        }
    }
    sha256_digest(&buf)
}

/// Bind a binary [`DomainValue`] request payload directly against a [`PinnedRoutineDescriptor`],
/// evaluating default expressions freshly on omitted slots (`T052`, `T056`).
pub fn bind_domain_call_arguments(
    payload: &DomainValue,
    descriptor: &PinnedRoutineDescriptor,
) -> Result<Vec<RoutineValue>, KalamDbError> {
    let input_params = &descriptor.input_parameters;
    if input_params.is_empty() {
        return match payload {
            DomainValue::Void | DomainValue::Omitted => Ok(Vec::new()),
            DomainValue::Composite(fields) if fields.is_empty() => Ok(Vec::new()),
            DomainValue::Array(items) if items.is_empty() => Ok(Vec::new()),
            other => Err(invalid_args(format!(
                "procedure {} expected 0 arguments, got {other:?}",
                descriptor.routine_id
            ))),
        };
    }

    match payload {
        DomainValue::Composite(fields) => {
            let mut out = Vec::with_capacity(input_params.len());
            for (idx, param) in input_params.iter().enumerate() {
                let slot = (idx + 1) as u16;
                let found = fields.iter().find(|(s, _)| *s == slot).map(|(_, v)| v);
                match found {
                    None | Some(DomainValue::Omitted) => {
                        if let Some(default_expr) = param.default_expr.as_deref() {
                            out.push(evaluate_parameter_default(param, default_expr)?);
                        } else if !param.not_null {
                            out.push(RoutineValue::new(datafusion::scalar::ScalarValue::Null));
                        } else {
                            return Err(invalid_args(format!(
                                "missing required argument '{}' (slot {slot}) for {}",
                                param.name, descriptor.routine_id
                            )));
                        }
                    },
                    Some(val) => {
                        out.push(domain_value_to_routine_value(
                            val,
                            param.builtin_data_type().as_ref(),
                        )?);
                    },
                }
            }
            Ok(out)
        },
        DomainValue::Array(items) => {
            if items.len() > input_params.len() {
                return Err(invalid_args(format!(
                    "procedure {} expected {} argument(s), got {}",
                    descriptor.routine_id,
                    input_params.len(),
                    items.len()
                )));
            }
            let mut out = Vec::with_capacity(input_params.len());
            for (idx, param) in input_params.iter().enumerate() {
                match items.get(idx) {
                    None | Some(DomainValue::Omitted) => {
                        if let Some(default_expr) = param.default_expr.as_deref() {
                            out.push(evaluate_parameter_default(param, default_expr)?);
                        } else {
                            return Err(invalid_args(format!(
                                "missing required argument '{}' for {}",
                                param.name, descriptor.routine_id
                            )));
                        }
                    },
                    Some(val) => {
                        out.push(domain_value_to_routine_value(
                            val,
                            param.builtin_data_type().as_ref(),
                        )?);
                    },
                }
            }
            Ok(out)
        },
        other => Err(invalid_args(format!(
            "expected composite or array domain arguments for {}, got {other:?}",
            descriptor.routine_id
        ))),
    }
}

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

pub(super) fn bind_sql_routine_call(
    stores: &CatalogStores,
    call: &RoutineCall,
    params: &[datafusion::scalar::ScalarValue],
) -> Result<Vec<RoutineValue>, KalamDbError> {
    let raw_args = bind_call_arguments(&call.arguments, params)?;
    if call.named_arguments.is_empty() || call.named_arguments.iter().all(|name| name.is_none()) {
        return Ok(raw_args);
    }

    let routine_params = stores.list_parameters(&call.routine_id).map_err(|error| {
        KalamDbError::CatalogError(format!(
            "failed to load parameters for routine {}: {error}",
            call.routine_id
        ))
    })?;
    let input_params: Vec<&CatalogRoutineParameter> = routine_params
        .iter()
        .filter(|param| {
            matches!(
                param.mode,
                RoutineParameterMode::In
                    | RoutineParameterMode::InOut
                    | RoutineParameterMode::Variadic
            )
        })
        .collect();

    if raw_args.len() > input_params.len()
        && !input_params
            .last()
            .is_some_and(|param| param.mode == RoutineParameterMode::Variadic)
    {
        return Err(invalid_args(format!(
            "procedure {} expected {} argument(s), got {}",
            call.routine_id,
            input_params.len(),
            raw_args.len()
        )));
    }

    let mut assigned: Vec<Option<RoutineValue>> = vec![None; input_params.len()];
    for (idx, arg) in raw_args.into_iter().enumerate() {
        let maybe_name = call.named_arguments.get(idx).and_then(|name| name.as_deref());
        if let Some(arg_name) = maybe_name {
            let Some(param_idx) =
                input_params.iter().position(|param| param.name.eq_ignore_ascii_case(arg_name))
            else {
                return Err(invalid_args(format!(
                    "routine {} has no input parameter named '{}'",
                    call.routine_id, arg_name
                )));
            };
            if assigned[param_idx].is_some() {
                return Err(invalid_args(format!(
                    "argument for parameter '{}' of routine {} specified more than once",
                    input_params[param_idx].name, call.routine_id
                )));
            }
            assigned[param_idx] = Some(arg);
        } else {
            if idx >= assigned.len() {
                return Err(invalid_args(format!(
                    "procedure {} expected {} argument(s)",
                    call.routine_id,
                    input_params.len()
                )));
            }
            if assigned[idx].is_some() {
                return Err(invalid_args(format!(
                    "argument for parameter '{}' of routine {} specified more than once",
                    input_params[idx].name, call.routine_id
                )));
            }
            assigned[idx] = Some(arg);
        }
    }

    let mut resolved = Vec::with_capacity(input_params.len());
    for (param, slot) in input_params.iter().zip(assigned.into_iter()) {
        if let Some(val) = slot {
            resolved.push(val);
        } else if let Some(default_expr) = param.default_expr.as_deref() {
            resolved.push(evaluate_parameter_default(param, default_expr)?);
        } else {
            return Err(invalid_args(format!(
                "missing required argument for parameter '{}' of routine {}",
                param.name, call.routine_id
            )));
        }
    }
    Ok(resolved)
}

pub(super) fn validate_call_arguments(
    type_registry: &TypeRegistry,
    stores: &CatalogStores,
    routine_id: &RoutineId,
    args: &[RoutineValue],
) -> Result<(), KalamDbError> {
    let params = stores.list_parameters(routine_id).map_err(|error| {
        KalamDbError::CatalogError(format!(
            "failed to load parameters for procedure {routine_id}: {error}"
        ))
    })?;
    let input_params: Vec<&CatalogRoutineParameter> = params
        .iter()
        .filter(|param| {
            matches!(
                param.mode,
                RoutineParameterMode::In
                    | RoutineParameterMode::InOut
                    | RoutineParameterMode::Variadic
            )
        })
        .collect();
    let positional = positional_args(type_registry, stores, routine_id, &input_params, args)?;
    for (parameter, value) in input_params.iter().zip(positional.iter()) {
        validate_spec(type_registry, stores, &TypeSpec::from_parameter(parameter), value)?;
    }
    Ok(())
}

pub(super) fn resolve_input_arguments(
    type_registry: &TypeRegistry,
    stores: &CatalogStores,
    routine_id: &RoutineId,
    args: &[RoutineValue],
) -> Result<Vec<(CatalogRoutineParameter, RoutineValue)>, KalamDbError> {
    let params = stores.list_parameters(routine_id).map_err(|error| {
        KalamDbError::CatalogError(format!(
            "failed to load parameters for procedure {routine_id}: {error}"
        ))
    })?;
    let input_params: Vec<&CatalogRoutineParameter> = params
        .iter()
        .filter(|param| {
            matches!(
                param.mode,
                RoutineParameterMode::In
                    | RoutineParameterMode::InOut
                    | RoutineParameterMode::Variadic
            )
        })
        .collect();
    let positional = positional_args(type_registry, stores, routine_id, &input_params, args)?;
    Ok(input_params.into_iter().cloned().zip(positional).collect())
}

pub(super) fn validate_call_return(
    type_registry: &TypeRegistry,
    stores: &CatalogStores,
    routine: &CatalogRoutine,
    value: &RoutineValue,
) -> Result<(), KalamDbError> {
    if let Some(spec) = TypeSpec::from_return(routine) {
        return validate_spec(type_registry, stores, &spec, value);
    }
    if routine.kind == RoutineKind::Procedure {
        let params = stores.list_parameters(&routine.routine_id).map_err(|error| {
            KalamDbError::CatalogError(format!(
                "failed to load parameters for procedure {}: {error}",
                routine.routine_id
            ))
        })?;
        let out_params: Vec<&CatalogRoutineParameter> = params
            .iter()
            .filter(|param| {
                matches!(param.mode, RoutineParameterMode::Out | RoutineParameterMode::InOut)
            })
            .collect();
        if !out_params.is_empty() {
            let json = structured_json(value).unwrap_or(JsonValue::Null);
            if out_params.len() == 1 {
                let param = out_params[0];
                let field_val = match &json {
                    JsonValue::Object(map) => map.get(&param.name).unwrap_or(&json),
                    _ => &json,
                };
                let mut spec = TypeSpec::from_parameter(param);
                spec.display = format!("OUT parameter '{}'", param.name);
                return validate_json(type_registry, stores, &spec, field_val);
            }
            let JsonValue::Object(map) = &json else {
                return Err(invalid_args(format!(
                    "procedure {} with OUT/INOUT parameters must return an object",
                    routine.routine_id
                )));
            };
            for param in out_params {
                let field_val = map.get(&param.name).unwrap_or(&JsonValue::Null);
                let mut spec = TypeSpec::from_parameter(param);
                spec.display = format!("OUT parameter '{}'", param.name);
                validate_json(type_registry, stores, &spec, field_val)?;
            }
        }
    }
    Ok(())
}

fn positional_args(
    type_registry: &TypeRegistry,
    stores: &CatalogStores,
    routine_id: &RoutineId,
    params: &[&CatalogRoutineParameter],
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
        let extracted = extract_named(&args[0], params)?;
        return coerce_positional_parameters(type_registry, stores, params, extracted);
    }

    if params.last().is_some_and(|param| param.mode == RoutineParameterMode::Variadic) {
        let fixed_count = params.len() - 1;
        if args.len() < fixed_count {
            return Err(invalid_args(format!(
                "procedure {routine_id} expected at least {fixed_count} argument(s), got {}",
                args.len()
            )));
        }
        let mut resolved = args[..fixed_count].to_vec();
        let packed = if args.len() == fixed_count + 1 && is_array_routine_value(&args[fixed_count])
        {
            args[fixed_count].clone()
        } else {
            let mut items = Vec::with_capacity(args.len() - fixed_count);
            for arg in &args[fixed_count..] {
                items.push(structured_json(arg).unwrap_or(JsonValue::Null));
            }
            json_to_routine_value(&JsonValue::Array(items), None)?
        };
        resolved.push(packed);
        return coerce_positional_parameters(type_registry, stores, params, resolved);
    }

    if args.len() > params.len() {
        return Err(invalid_args(format!(
            "procedure {routine_id} expected {} argument(s), got {}",
            params.len(),
            args.len()
        )));
    }
    if args.len() < params.len() {
        let mut resolved = args.to_vec();
        for param in &params[args.len()..] {
            let Some(default_expr) = param.default_expr.as_deref() else {
                return Err(invalid_args(format!(
                    "procedure {routine_id} expected {} argument(s), got {}",
                    params.len(),
                    args.len()
                )));
            };
            resolved.push(evaluate_parameter_default(param, default_expr)?);
        }
        return coerce_positional_parameters(type_registry, stores, params, resolved);
    }
    coerce_positional_parameters(type_registry, stores, params, args.to_vec())
}

fn coerce_positional_parameters(
    type_registry: &TypeRegistry,
    stores: &CatalogStores,
    params: &[&CatalogRoutineParameter],
    values: Vec<RoutineValue>,
) -> Result<Vec<RoutineValue>, KalamDbError> {
    let mut out = Vec::with_capacity(values.len());
    for (param, value) in params.iter().zip(values.into_iter()) {
        out.push(coerce_parameter_value(type_registry, stores, param, value)?);
    }
    Ok(out)
}

fn coerce_parameter_value(
    type_registry: &TypeRegistry,
    stores: &CatalogStores,
    param: &CatalogRoutineParameter,
    value: RoutineValue,
) -> Result<RoutineValue, KalamDbError> {
    if !param.is_array {
        if let Some(type_id) = &param.type_id {
            let resolved = type_registry.get_or_load(stores, type_id)?;
            if matches!(
                resolved.kind,
                CatalogTypeKind::Composite
                    | CatalogTypeKind::ImplicitTableRow
                    | CatalogTypeKind::RowAlias
            ) {
                if let Some(JsonValue::Array(items)) = structured_json(&value) {
                    let active_fields: Vec<_> =
                        resolved.fields.iter().filter(|f| !f.dropped).collect();
                    if items.len() != active_fields.len() {
                        return Err(invalid_args(format!(
                            "argument '{}' expected {} composite field(s) for {}, got {}",
                            param.name,
                            active_fields.len(),
                            type_id,
                            items.len()
                        )));
                    }
                    let mut map = serde_json::Map::with_capacity(active_fields.len());
                    for (field, item) in active_fields.iter().zip(items.into_iter()) {
                        map.insert(field.name.clone(), item);
                    }
                    return json_to_routine_value(&JsonValue::Object(map), None);
                }
            }
        }
    }
    Ok(value)
}

fn is_array_routine_value(value: &RoutineValue) -> bool {
    matches!(
        &value.value,
        datafusion::scalar::ScalarValue::List(_)
            | datafusion::scalar::ScalarValue::LargeList(_)
            | datafusion::scalar::ScalarValue::FixedSizeList(_)
    ) || matches!(structured_json(value), Some(JsonValue::Array(_)))
}

fn evaluate_parameter_default(
    param: &CatalogRoutineParameter,
    default_expr: &str,
) -> Result<RoutineValue, KalamDbError> {
    let call_arg = parse_call_argument_sql(default_expr).map_err(|error| {
        invalid_args(format!(
            "invalid default expression '{}' for parameter '{}': {error}",
            default_expr, param.name
        ))
    })?;
    let mut values = bind_call_arguments(&[call_arg], &[])?;
    let value = values
        .pop()
        .unwrap_or_else(|| RoutineValue::new(datafusion::scalar::ScalarValue::Null));
    if let Some(json) = structured_json(&value) {
        return json_to_routine_value(&json, param.builtin_data_type().as_ref());
    }
    Ok(value)
}

fn is_named_record(args: &[RoutineValue], params: &[&CatalogRoutineParameter]) -> bool {
    if args.len() != 1 || !args[0].host_named_input {
        return false;
    }
    let Some(JsonValue::Object(map)) = structured_json(&args[0]) else {
        return false;
    };
    params
        .iter()
        .all(|parameter| map.contains_key(&parameter.name) || parameter.default_expr.is_some())
        && params.iter().any(|parameter| map.contains_key(&parameter.name))
}

fn extract_named(
    arg: &RoutineValue,
    params: &[&CatalogRoutineParameter],
) -> Result<Vec<RoutineValue>, KalamDbError> {
    let Some(JsonValue::Object(map)) = structured_json(arg) else {
        return Err(invalid_args("expected a named argument object".to_string()));
    };
    let mut values = Vec::with_capacity(params.len());
    for parameter in params {
        if let Some(json) = map.get(&parameter.name) {
            values.push(json_to_routine_value(json, parameter.builtin_data_type().as_ref())?);
        } else if let Some(default_expr) = parameter.default_expr.as_deref() {
            values.push(evaluate_parameter_default(parameter, default_expr)?);
        } else {
            values.push(json_to_routine_value(
                &JsonValue::Null,
                parameter.builtin_data_type().as_ref(),
            )?);
        }
    }
    Ok(values)
}

fn validate_spec(
    type_registry: &TypeRegistry,
    stores: &CatalogStores,
    spec: &TypeSpec,
    value: &RoutineValue,
) -> Result<(), KalamDbError> {
    let json = structured_json(value).unwrap_or(JsonValue::Null);
    validate_json(type_registry, stores, spec, &json)
}

fn validate_json(
    type_registry: &TypeRegistry,
    stores: &CatalogStores,
    spec: &TypeSpec,
    json: &JsonValue,
) -> Result<(), KalamDbError> {
    if json.is_null() {
        if spec.not_null {
            return Err(invalid_args(format!("{} cannot be null", spec.display)));
        }
        if !spec.is_array {
            if let Some(type_id) = &spec.type_id {
                let resolved = type_registry.get_or_load(stores, type_id)?;
                if resolved.kind == CatalogTypeKind::Domain {
                    kalamdb_commons::conversions::arrow_json_conversion::validate_domain_specs(
                        &resolved.domain_constraints(),
                        &datafusion::scalar::ScalarValue::Null,
                    )
                    .map_err(invalid_args)?;
                }
            }
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
            validate_json(type_registry, stores, &element, item)?;
        }
        return Ok(());
    }
    if let Some(type_id) = &spec.type_id {
        return validate_named_type(type_registry, stores, spec, type_id, json);
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
    type_registry: &TypeRegistry,
    stores: &CatalogStores,
    spec: &TypeSpec,
    type_id: &TypeId,
    json: &JsonValue,
) -> Result<(), KalamDbError> {
    let resolved = type_registry.get_or_load(stores, type_id)?;
    match resolved.kind {
        CatalogTypeKind::Enum => {
            let supported =
                format_supported_enum_labels(resolved.fields.iter().map(|f| f.name.as_str()));
            let JsonValue::String(label) = json else {
                return Err(invalid_args(format!(
                    "{} must be an enum label of {type_id}; {supported}",
                    spec.display
                )));
            };
            if resolved.fields.iter().any(|field| field.name == *label) {
                Ok(())
            } else {
                Err(invalid_args(format!(
                    "invalid enum label '{label}' for {}; {supported}",
                    spec.display
                )))
            }
        },
        CatalogTypeKind::Composite
        | CatalogTypeKind::ImplicitTableRow
        | CatalogTypeKind::RowAlias => {
            let JsonValue::Object(map) = json else {
                return Err(invalid_args(format!("{} must be an object", spec.display)));
            };
            for field in resolved.fields.iter() {
                let child = TypeSpec {
                    display:   format!("{} field '{}'", spec.display, field.name),
                    type_id:   field.type_ref.named_type_id().cloned(),
                    data_type: field.type_ref.builtin().or_else(|| {
                        field.type_ref.element().and_then(kalamdb_commons::LogicalTypeRef::builtin)
                    }),
                    is_array:  field.type_ref.is_array(),
                    not_null:  field.not_null,
                    nonempty:  false,
                };
                let value = map.get(&field.name).unwrap_or(&JsonValue::Null);
                validate_json(type_registry, stores, &child, value)?;
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
        CatalogTypeKind::Domain => {
            if let Some(base_field) =
                resolved.fields.iter().find(|field| field.name == "__base" && !field.dropped)
            {
                let base_spec = TypeSpec {
                    display:   spec.display.clone(),
                    type_id:   base_field.type_ref.named_type_id().cloned(),
                    data_type: base_field.type_ref.builtin().or_else(|| {
                        base_field
                            .type_ref
                            .element()
                            .and_then(kalamdb_commons::LogicalTypeRef::builtin)
                    }),
                    is_array:  base_field.type_ref.is_array(),
                    not_null:  base_field.not_null,
                    nonempty:  false,
                };
                validate_json(type_registry, stores, &base_spec, json)?;
            }
            let scalar = kalamdb_commons::conversions::json_value_to_scalar_strict(json)
                .map_err(invalid_args)?;
            kalamdb_commons::conversions::arrow_json_conversion::validate_domain_specs(
                &resolved.domain_constraints(),
                &scalar,
            )
            .map_err(invalid_args)?;
            Ok(())
        },
    }
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

fn format_supported_enum_labels<'a, I>(labels: I) -> String
where
    I: IntoIterator<Item = &'a str>,
{
    let labels: Vec<&str> = labels.into_iter().collect();
    if labels.is_empty() {
        return "supported values: none".to_string();
    }
    const LIMIT: usize = 32;
    let mut out = String::from("supported values: ");
    for (index, label) in labels.iter().copied().take(LIMIT).enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push('\'');
        out.push_str(label);
        out.push('\'');
    }
    if labels.len() > LIMIT {
        out.push_str(" (and ");
        out.push_str(&(labels.len() - LIMIT).to_string());
        out.push_str(" more)");
    }
    out
}

#[cfg(test)]
mod tests {
    use datafusion::scalar::ScalarValue;
    use kalamdb_functions::RoutineValue;
    use serde_json::json;

    use super::{format_supported_enum_labels, is_empty_object, structured_json};
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

    #[test]
    fn supported_enum_labels_are_quoted_in_error_text() {
        assert_eq!(
            format_supported_enum_labels(["active", "blocked"]),
            "supported values: 'active', 'blocked'"
        );
        assert_eq!(format_supported_enum_labels(std::iter::empty()), "supported values: none");
    }

    #[test]
    fn supported_enum_labels_truncate_long_lists() {
        let labels: Vec<String> = (0..34).map(|index| format!("v{index}")).collect();
        let text = format_supported_enum_labels(labels.iter().map(String::as_str));
        assert!(text.starts_with("supported values: 'v0'"));
        assert!(text.contains("'v31'"));
        assert!(!text.contains("'v32'"));
        assert!(text.ends_with("(and 2 more)"));
    }
}
