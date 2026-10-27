//! Contract argument and return validation for procedure CALL.

use std::sync::Arc;

use arrow::datatypes::{DataType, Field};
use dashmap::DashMap;
use datafusion::scalar::ScalarValue;
use kalamdb_commons::{
    datatypes::ToArrowType,
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
    pub epoch: u16,
    pub routine_id: RoutineId,
    pub revision_hash: [u8; 32],
    pub revision_hex: String,
    /// [`CatalogStores::contract_generation`] observed when this descriptor was built.
    pub catalog_generation: u64,
    pub routine: CatalogRoutine,
    pub parameters: Arc<[CatalogRoutineParameter]>,
    pub input_parameters: Arc<[CatalogRoutineParameter]>,
    pub out_parameters: Arc<[CatalogRoutineParameter]>,
}

static PINNED_DESCRIPTOR_CACHE: Lazy<DashMap<RoutineId, Arc<PinnedRoutineDescriptor>>> =
    Lazy::new(DashMap::new);

/// Resolve and cache an immutable [`PinnedRoutineDescriptor`] from authoritative catalog metadata
/// (`T052`).
///
/// The cached value is shared. A hit does not clone the catalog row. The routine row, including
/// its SQL body, stays on the descriptor so a pinned call does not reload the catalog.
pub fn resolve_pinned_descriptor(
    stores: &CatalogStores,
    routine_id: &RoutineId,
) -> Result<Arc<PinnedRoutineDescriptor>, KalamDbError> {
    let generation = stores.contract_generation();
    if let Some(cached) = PINNED_DESCRIPTOR_CACHE.get(routine_id) {
        if cached.catalog_generation == generation {
            return Ok(Arc::clone(cached.value()));
        }
    }
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
    let rev_hash = compute_descriptor_revision(&routine, &parameters);
    let (input_params, out_params) = split_parameter_directions(&parameters);
    let rev_hex = digest_to_hex(&rev_hash);
    let descriptor = Arc::new(PinnedRoutineDescriptor {
        epoch: DOMAIN_EPOCH,
        routine_id: routine_id.clone(),
        revision_hash: rev_hash,
        revision_hex: rev_hex,
        catalog_generation: generation,
        routine,
        parameters: Arc::from(parameters),
        input_parameters: Arc::from(input_params),
        out_parameters: Arc::from(out_params),
    });
    if PINNED_DESCRIPTOR_CACHE.len() >= MAX_PINNED_DESCRIPTOR_CACHE {
        PINNED_DESCRIPTOR_CACHE.clear();
    }
    PINNED_DESCRIPTOR_CACHE.insert(routine_id.clone(), Arc::clone(&descriptor));
    Ok(descriptor)
}

fn split_parameter_directions(
    parameters: &[CatalogRoutineParameter],
) -> (Vec<CatalogRoutineParameter>, Vec<CatalogRoutineParameter>) {
    let mut input_params = Vec::new();
    let mut out_params = Vec::new();
    for parameter in parameters {
        let is_input = matches!(
            parameter.mode,
            RoutineParameterMode::In | RoutineParameterMode::InOut | RoutineParameterMode::Variadic
        );
        let is_output =
            matches!(parameter.mode, RoutineParameterMode::Out | RoutineParameterMode::InOut);
        if is_input && is_output {
            input_params.push(parameter.clone());
            out_params.push(parameter.clone());
        } else if is_input {
            input_params.push(parameter.clone());
        } else if is_output {
            out_params.push(parameter.clone());
        }
    }
    (input_params, out_params)
}

fn compute_descriptor_revision(
    routine: &CatalogRoutine,
    parameters: &[CatalogRoutineParameter],
) -> [u8; 32] {
    let mut buf = Vec::with_capacity(256);
    buf.extend_from_slice(b"KALAM_ROUTINE_DESCRIPTOR_V2");
    buf.extend_from_slice(&DOMAIN_EPOCH.to_le_bytes());
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
    bind_domain_arguments(payload, descriptor, None, None)
}

/// Same as [`bind_domain_call_arguments`], resolving composite field names from the type catalog.
pub fn bind_domain_call_arguments_typed(
    payload: &DomainValue,
    descriptor: &PinnedRoutineDescriptor,
    type_registry: &TypeRegistry,
    stores: &CatalogStores,
) -> Result<Vec<RoutineValue>, KalamDbError> {
    bind_domain_arguments(payload, descriptor, Some(type_registry), Some(stores))
}

fn bind_domain_arguments(
    payload: &DomainValue,
    descriptor: &PinnedRoutineDescriptor,
    type_registry: Option<&TypeRegistry>,
    stores: Option<&CatalogStores>,
) -> Result<Vec<RoutineValue>, KalamDbError> {
    let input_params = &descriptor.input_parameters;
    if input_params.is_empty() {
        return match payload {
            DomainValue::Void | DomainValue::Omitted => Ok(Vec::new()),
            DomainValue::Composite(fields) if fields.is_empty() => Ok(Vec::new()),
            DomainValue::Object(fields) if fields.is_empty() => Ok(Vec::new()),
            DomainValue::Array(items) if items.is_empty() => Ok(Vec::new()),
            other => Err(invalid_args(format!(
                "procedure {} expected 0 arguments, got {other:?}",
                descriptor.routine_id
            ))),
        };
    }

    match payload {
        DomainValue::Composite(fields) => {
            reject_unknown_or_duplicate_slots(fields, input_params.len(), &descriptor.routine_id)?;
            let mut out = Vec::with_capacity(input_params.len());
            for (idx, param) in input_params.iter().enumerate() {
                let slot = (idx + 1) as u16;
                let found = fields.iter().find(|(s, _)| *s == slot).map(|(_, v)| v);
                out.push(bind_parameter_slot(
                    found,
                    param,
                    &descriptor.routine_id,
                    Some(slot),
                    type_registry,
                    stores,
                )?);
            }
            Ok(out)
        },
        DomainValue::Object(fields) => {
            for (name, _) in fields {
                if !input_params.iter().any(|param| param.name == *name) {
                    return Err(invalid_args(format!(
                        "unknown argument '{name}' for {}",
                        descriptor.routine_id
                    )));
                }
            }
            let mut out = Vec::with_capacity(input_params.len());
            for param in input_params.iter() {
                let found = fields.iter().find(|(name, _)| name == &param.name).map(|(_, v)| v);
                out.push(bind_parameter_slot(
                    found,
                    param,
                    &descriptor.routine_id,
                    None,
                    type_registry,
                    stores,
                )?);
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
                out.push(bind_parameter_slot(
                    items.get(idx),
                    param,
                    &descriptor.routine_id,
                    None,
                    type_registry,
                    stores,
                )?);
            }
            Ok(out)
        },
        other => Err(invalid_args(format!(
            "expected composite, object, or array domain arguments for {}, got {other:?}",
            descriptor.routine_id
        ))),
    }
}

fn bind_parameter_slot(
    found: Option<&DomainValue>,
    param: &CatalogRoutineParameter,
    routine_id: &RoutineId,
    slot: Option<u16>,
    type_registry: Option<&TypeRegistry>,
    stores: Option<&CatalogStores>,
) -> Result<RoutineValue, KalamDbError> {
    match found {
        None | Some(DomainValue::Omitted) => {
            if let Some(default_expr) = param.default_expr.as_deref() {
                Ok(evaluate_parameter_default(param, default_expr)?)
            } else if !param.not_null {
                Ok(RoutineValue::new(ScalarValue::Null))
            } else {
                let slot_note = slot.map(|slot| format!(" (slot {slot})")).unwrap_or_default();
                Err(invalid_args(format!(
                    "missing required argument '{}'{} for {routine_id}",
                    param.name, slot_note
                )))
            }
        },
        Some(DomainValue::Void) => Err(invalid_args(format!(
            "void is not a valid argument for '{}' of {routine_id}",
            param.name
        ))),
        Some(value) => {
            let bound = bind_domain_argument(value, param, type_registry, stores)?;
            coerce_bound_argument(bound, param)
        },
    }
}

fn bind_domain_argument(
    value: &DomainValue,
    param: &CatalogRoutineParameter,
    type_registry: Option<&TypeRegistry>,
    stores: Option<&CatalogStores>,
) -> Result<RoutineValue, KalamDbError> {
    let DomainValue::Composite(fields) = value else {
        return domain_value_to_routine_value(value, param.builtin_data_type().as_ref());
    };
    let Some(names) = composite_field_names(param, type_registry, stores) else {
        return domain_value_to_routine_value(value, param.builtin_data_type().as_ref());
    };
    let mut named = Vec::with_capacity(fields.len());
    for (slot, field_val) in fields {
        let index = usize::from(slot.saturating_sub(1));
        let name = names.get(index).cloned().unwrap_or_else(|| slot.to_string());
        named.push((name, domain_value_to_routine_value(field_val, None)?));
    }
    Ok(RoutineValue::named_object(named))
}

fn composite_field_names(
    param: &CatalogRoutineParameter,
    type_registry: Option<&TypeRegistry>,
    stores: Option<&CatalogStores>,
) -> Option<Vec<String>> {
    let type_registry = type_registry?;
    let stores = stores?;
    let type_id = param.type_id.as_ref()?;
    let resolved = type_registry.get_or_load(stores, type_id).ok()?;
    if !matches!(
        resolved.kind,
        CatalogTypeKind::Composite | CatalogTypeKind::ImplicitTableRow | CatalogTypeKind::RowAlias
    ) {
        return None;
    }
    Some(
        resolved
            .fields
            .iter()
            .filter(|field| !field.dropped)
            .map(|field| field.name.clone())
            .collect(),
    )
}

fn coerce_bound_argument(
    value: RoutineValue,
    param: &CatalogRoutineParameter,
) -> Result<RoutineValue, KalamDbError> {
    if value.named_fields.is_some() || value.json_sql {
        return Ok(value);
    }
    let Some(data_type) = param.builtin_data_type() else {
        return Ok(value);
    };
    if data_type == KalamDataType::Json {
        return Ok(value);
    }
    let Ok(arrow) = data_type.to_arrow_type() else {
        return Ok(value);
    };
    let target = if param.is_array {
        DataType::List(Arc::new(Field::new("item", arrow, !param.not_null)))
    } else {
        arrow
    };
    if value.value.data_type() == target {
        return Ok(value);
    }
    let casted = if value.value.is_null() {
        ScalarValue::try_from(&target).map_err(|error| invalid_args(error.to_string()))?
    } else {
        match value.value.cast_to(&target) {
            Ok(casted) => casted,
            Err(error) => coerce_text_argument(&value.value, data_type).map_err(|_| {
                invalid_args(format!(
                    "cannot coerce argument '{}' to {}: {error}",
                    param.name, param.type_name
                ))
            })?,
        }
    };
    Ok(RoutineValue::new(casted))
}

fn coerce_text_argument(value: &ScalarValue, data_type: KalamDataType) -> Result<ScalarValue, ()> {
    let text = match value {
        ScalarValue::Utf8(Some(text))
        | ScalarValue::LargeUtf8(Some(text))
        | ScalarValue::Utf8View(Some(text)) => text,
        _ => return Err(()),
    };
    kalamdb_commons::json_value_to_scalar_for_column(&JsonValue::String(text.clone()), &data_type)
        .map_err(|_| ())
}

struct TypeSpec {
    display: String,
    type_id: Option<TypeId>,
    data_type: Option<KalamDataType>,
    is_array: bool,
    not_null: bool,
    nonempty: bool,
}

impl TypeSpec {
    fn from_parameter(parameter: &CatalogRoutineParameter) -> Self {
        Self {
            display: format!("argument '{}'", parameter.name),
            type_id: parameter.type_id.clone(),
            data_type: parameter.builtin_data_type(),
            is_array: parameter.is_array,
            not_null: parameter.not_null,
            nonempty: parameter.nonempty,
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
            display: "return value".to_string(),
            type_id: routine.return_type_id.clone(),
            data_type: routine.return_data_type.clone(),
            is_array: routine.return_is_array,
            not_null: routine.return_not_null,
            nonempty: false,
        })
    }

    fn element(&self) -> Self {
        Self {
            display: format!("{} element", self.display),
            type_id: self.type_id.clone(),
            data_type: self.data_type.clone(),
            is_array: false,
            not_null: false,
            nonempty: false,
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

pub(super) fn resolve_and_validate_parameters(
    type_registry: &TypeRegistry,
    stores: &CatalogStores,
    routine_id: &RoutineId,
    parameters: &[CatalogRoutineParameter],
    args: &[RoutineValue],
) -> Result<Vec<(CatalogRoutineParameter, RoutineValue)>, KalamDbError> {
    let input_params: Vec<&CatalogRoutineParameter> = parameters
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
    let resolved: Vec<_> = input_params.into_iter().cloned().zip(positional).collect();
    for (parameter, value) in &resolved {
        validate_spec(type_registry, stores, &TypeSpec::from_parameter(parameter), value)?;
    }
    Ok(resolved)
}

pub(super) fn validate_call_return(
    type_registry: &TypeRegistry,
    stores: &CatalogStores,
    routine: &CatalogRoutine,
    value: &RoutineValue,
    parameters: &[CatalogRoutineParameter],
) -> Result<(), KalamDbError> {
    if let Some(spec) = TypeSpec::from_return(routine) {
        return validate_spec(type_registry, stores, &spec, value);
    }
    if routine.kind == RoutineKind::Procedure {
        let out_params: Vec<&CatalogRoutineParameter> = parameters
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
    if spec.type_id.is_none() && !spec.is_array {
        return validate_builtin(spec, value);
    }
    let json = structured_json(value).unwrap_or(JsonValue::Null);
    validate_json(type_registry, stores, spec, &json)
}

fn validate_builtin(spec: &TypeSpec, value: &RoutineValue) -> Result<(), KalamDbError> {
    if is_null_argument(value) {
        if spec.not_null {
            return Err(invalid_args(format!("{} cannot be null", spec.display)));
        }
        return Ok(());
    }
    if spec.nonempty {
        let empty = match &value.value {
            ScalarValue::Utf8(Some(text))
            | ScalarValue::LargeUtf8(Some(text))
            | ScalarValue::Utf8View(Some(text)) => text.is_empty(),
            ScalarValue::Binary(Some(bytes))
            | ScalarValue::LargeBinary(Some(bytes))
            | ScalarValue::BinaryView(Some(bytes)) => bytes.is_empty(),
            _ => false,
        };
        if empty {
            return Err(invalid_args(format!("{} cannot be empty", spec.display)));
        }
    }
    Ok(())
}

fn is_null_argument(value: &RoutineValue) -> bool {
    value.value.is_null()
        || matches!(
            &value.value,
            ScalarValue::Null | ScalarValue::Utf8(None) | ScalarValue::LargeUtf8(None)
        )
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
                    display: format!("{} field '{}'", spec.display, field.name),
                    type_id: field.type_ref.named_type_id().cloned(),
                    data_type: field.type_ref.builtin().or_else(|| {
                        field.type_ref.element().and_then(kalamdb_commons::LogicalTypeRef::builtin)
                    }),
                    is_array: field.type_ref.is_array(),
                    not_null: field.not_null,
                    nonempty: false,
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
                    display: spec.display.clone(),
                    type_id: base_field.type_ref.named_type_id().cloned(),
                    data_type: base_field.type_ref.builtin().or_else(|| {
                        base_field
                            .type_ref
                            .element()
                            .and_then(kalamdb_commons::LogicalTypeRef::builtin)
                    }),
                    is_array: base_field.type_ref.is_array(),
                    not_null: base_field.not_null,
                    nonempty: false,
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
    routine_value_as_json(value)
}

fn is_empty_object(value: &RoutineValue) -> bool {
    matches!(structured_json(value), Some(JsonValue::Object(map)) if map.is_empty())
        || matches!(routine_value_as_json(value), Some(JsonValue::Null))
}

fn invalid_args(message: String) -> KalamDbError {
    FunctionsError::InvalidArguments(message).into()
}

/// Positional composite slots are 1-based and unique. Slot 0 and slots past the
/// declared parameter count are unknown arguments, not omitted defaults.
fn reject_unknown_or_duplicate_slots(
    fields: &[(u16, DomainValue)],
    param_count: usize,
    routine_id: &kalamdb_commons::models::RoutineId,
) -> Result<(), KalamDbError> {
    let mut seen = vec![false; param_count];
    for (slot, _) in fields {
        if *slot == 0 || (*slot as usize) > param_count {
            return Err(invalid_args(format!(
                "unknown argument slot {slot} for {routine_id}; valid slots are 1..={param_count}"
            )));
        }
        let index = (*slot as usize) - 1;
        if seen[index] {
            return Err(invalid_args(format!("duplicate argument slot {slot} for {routine_id}")));
        }
        seen[index] = true;
    }
    Ok(())
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
    use std::sync::Arc;

    use datafusion::scalar::ScalarValue;
    use kalamdb_commons::{
        models::{
            RoutineId, RoutineKind, RoutineNullSafety, RoutineParallelSafety, RoutineParameterId,
            RoutineParameterMode, RoutineSecurityMode, RoutineVolatility, SchemaId, UserId,
        },
        KalamDataType,
    };
    use kalamdb_functions::RoutineValue;
    use kalamdb_serialization::DomainValue;
    use kalamdb_system::{CatalogRoutine, CatalogRoutineParameter};
    use serde_json::json;

    use super::{
        bind_domain_call_arguments, format_supported_enum_labels, is_empty_object, structured_json,
        PinnedRoutineDescriptor,
    };
    use crate::functions::convert::{json_to_routine_value, routine_value_as_json};

    #[test]
    fn objects_bind_as_json_sql() {
        let value = json_to_routine_value(&json!({"city": "Paris"}), None).unwrap();
        assert!(value.json_sql);
        let parsed = routine_value_as_json(&value).unwrap();
        assert_eq!(parsed["city"], "Paris");
    }

    #[test]
    fn structured_json_keeps_plain_text() {
        let value = RoutineValue::new(ScalarValue::Utf8(Some(r#"{"city":"Paris"}"#.into())));
        let parsed = structured_json(&value).unwrap();
        assert_eq!(parsed, json!(r#"{"city":"Paris"}"#));
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

    fn text_param(name: &str, ordinal: i32, not_null: bool) -> CatalogRoutineParameter {
        let routine_id = RoutineId::new("api.echo");
        CatalogRoutineParameter {
            parameter_id: RoutineParameterId::new(&routine_id, ordinal).expect("parameter id"),
            routine_id,
            name: name.to_string(),
            ordinal,
            type_id: None,
            type_name: "text".to_string(),
            is_array: false,
            not_null,
            nonempty: false,
            data_type: Some(KalamDataType::Text),
            mode: RoutineParameterMode::In,
            default_expr: None,
        }
    }

    fn echo_descriptor(params: Vec<CatalogRoutineParameter>) -> PinnedRoutineDescriptor {
        let namespace = SchemaId::new("api");
        PinnedRoutineDescriptor {
            epoch: 1,
            routine_id: RoutineId::new("api.echo"),
            revision_hash: [0; 32],
            revision_hex: "00".repeat(32),
            catalog_generation: 0,
            routine: CatalogRoutine {
                routine_id: RoutineId::from_parts(Some(&namespace), "echo"),
                namespace_id: namespace,
                name: "echo".to_string(),
                owner: UserId::new("root"),
                security: RoutineSecurityMode::Invoker,
                language: None,
                body: None,
                return_type_id: None,
                return_type_name: None,
                return_is_array: false,
                return_not_null: false,
                comment: None,
                return_data_type: None,
                inline_source_hash: None,
                inline_artifact_id: None,
                kind: RoutineKind::default(),
                volatility: RoutineVolatility::default(),
                null_safety: RoutineNullSafety::default(),
                parallel_safety: RoutineParallelSafety::default(),
                cost: None,
                rows: None,
                config_params: None,
            },
            parameters: Arc::from(params.clone()),
            input_parameters: Arc::from(params),
            out_parameters: Arc::from([]),
        }
    }

    #[test]
    fn composite_rejects_duplicate_and_unknown_slots() {
        let descriptor =
            echo_descriptor(vec![text_param("left", 1, true), text_param("right", 2, false)]);
        let duplicate = DomainValue::Composite(vec![
            (1, DomainValue::Utf8("a".into())),
            (1, DomainValue::Utf8("b".into())),
        ]);
        let duplicate_err = bind_domain_call_arguments(&duplicate, &descriptor).unwrap_err();
        assert!(duplicate_err.to_string().contains("duplicate argument slot 1"));

        let zero = DomainValue::Composite(vec![(0, DomainValue::Utf8("a".into()))]);
        let zero_err = bind_domain_call_arguments(&zero, &descriptor).unwrap_err();
        assert!(zero_err.to_string().contains("unknown argument slot 0"));

        let past_end = DomainValue::Composite(vec![(3, DomainValue::Utf8("a".into()))]);
        let past_err = bind_domain_call_arguments(&past_end, &descriptor).unwrap_err();
        assert!(past_err.to_string().contains("unknown argument slot 3"));
    }

    #[test]
    fn array_omits_nullable_parameters_as_sql_null() {
        let descriptor =
            echo_descriptor(vec![text_param("left", 1, true), text_param("right", 2, false)]);
        let payload =
            DomainValue::Array(vec![DomainValue::Utf8("kept".into()), DomainValue::Omitted]);
        let bound = bind_domain_call_arguments(&payload, &descriptor).unwrap();
        assert_eq!(bound[0].value, ScalarValue::Utf8(Some("kept".into())));
        assert_eq!(bound[1].value, ScalarValue::Null);

        let missing_required = DomainValue::Array(vec![DomainValue::Omitted]);
        let err = bind_domain_call_arguments(&missing_required, &descriptor).unwrap_err();
        assert!(err.to_string().contains("missing required argument 'left'"));
    }
}
