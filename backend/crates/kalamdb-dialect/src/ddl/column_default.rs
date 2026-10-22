//! Parse DEFAULT expressions into the shared `CALL` models.

use kalamdb_commons::{
    models::{
        is_builtin_default_name, normalize_builtin_name, BuiltinDefault, CallArgument, RoutineCall,
    },
    schemas::ColumnDefault,
    KalamDataType, RoutineId, SchemaId,
};
use serde_json::{Number, Value as JsonValue};
use sqlparser::ast::{
    Expr, Function, FunctionArg, FunctionArgExpr, FunctionArguments, ObjectName, ObjectNamePart,
    TypedString, UnaryOperator, Value,
};

use crate::{compatibility::map_sql_type_to_kalam, ddl::DdlResult, parser::KalamParser};

pub fn expr_to_column_default(
    expr: &Expr,
    default_namespace: &SchemaId,
) -> DdlResult<ColumnDefault> {
    match expr {
        Expr::Function(func) => {
            let call = function_to_routine_call(func, default_namespace)?;
            Ok(ColumnDefault::FunctionCall(call))
        },
        Expr::Value(value) => Ok(value_to_literal_default(&value.value)),
        Expr::Identifier(identifier) => {
            if identifier.value.eq_ignore_ascii_case("null") {
                return Ok(ColumnDefault::literal(JsonValue::Null));
            }
            if let Some(builtin) = BuiltinDefault::parse(&identifier.value) {
                return Ok(ColumnDefault::function(builtin.canonical_name(), vec![]));
            }
            Ok(ColumnDefault::literal(JsonValue::String(identifier.value.clone())))
        },
        _ => {
            let literal = expr.to_string();
            if literal.eq_ignore_ascii_case("null") {
                Ok(ColumnDefault::literal(JsonValue::Null))
            } else if let Some(builtin) = builtin_default_text(&literal) {
                Ok(ColumnDefault::function(builtin.canonical_name(), vec![]))
            } else {
                let val = literal.trim_matches('\'').to_string();
                Ok(ColumnDefault::literal(JsonValue::String(val)))
            }
        },
    }
}

pub fn expr_to_call_argument(expr: &Expr) -> DdlResult<CallArgument> {
    match expr {
        Expr::Nested(inner) => expr_to_call_argument(inner),
        Expr::Value(value) => value_to_call_argument(&value.value),
        Expr::Tuple(items) => {
            let mut tuple_args = Vec::with_capacity(items.len());
            for item in items {
                tuple_args.push(expr_to_call_argument(item)?);
            }
            Ok(CallArgument::Tuple(tuple_args))
        },
        Expr::Function(func)
            if func.name.0.len() == 1
                && matches!(
                    &func.name.0[0],
                    ObjectNamePart::Identifier(ident)
                        if ident.quote_style.is_none() && ident.value.eq_ignore_ascii_case("row")
                ) =>
        {
            match &func.args {
                FunctionArguments::None => Ok(CallArgument::Tuple(Vec::new())),
                FunctionArguments::List(list) => {
                    let mut tuple_args = Vec::with_capacity(list.args.len());
                    for arg in &list.args {
                        match arg {
                            FunctionArg::Unnamed(FunctionArgExpr::Expr(inner)) => {
                                tuple_args.push(expr_to_call_argument(inner)?);
                            },
                            _ => {
                                return Err(
                                    "ROW(...) elements must be unnamed expressions".to_string()
                                );
                            },
                        }
                    }
                    Ok(CallArgument::Tuple(tuple_args))
                },
                FunctionArguments::Subquery(_) => {
                    Err("ROW(...) cannot contain a subquery".to_string())
                },
            }
        },
        Expr::Cast {
            expr, data_type, ..
        } => {
            if let Ok(arg @ CallArgument::Tuple(_)) = expr_to_call_argument(expr) {
                return Ok(arg);
            }
            match map_sql_type_to_kalam(data_type) {
                Ok(kalam_type) => {
                    if matches!(expr.as_ref(), Expr::Value(value) if matches!(value.value, Value::Null))
                    {
                        return Ok(CallArgument::typed(kalam_type, JsonValue::Null));
                    }
                    let value = expr_to_json_literal(expr)?;
                    let value = coerce_json_for_type(&kalam_type, value)?;
                    Ok(CallArgument::typed(kalam_type, value))
                },
                Err(_) if matches!(data_type, sqlparser::ast::DataType::Custom(_, _)) => {
                    expr_to_call_argument(expr)
                },
                Err(err) => Err(err),
            }
        },
        Expr::TypedString(typed) => typed_string_to_call_argument(typed),
        Expr::UnaryOp {
            op: UnaryOperator::Minus,
            expr,
        } => negate_numeric_argument(expr),
        Expr::Array(array) => {
            let mut items = Vec::with_capacity(array.elem.len());
            for item in &array.elem {
                items.push(expr_to_json_literal(item)?);
            }
            Ok(CallArgument::json(JsonValue::Array(items)))
        },
        _ => Err(format!(
            "unsupported function argument '{expr}'; use a literal or CAST(literal AS type)"
        )),
    }
}

pub fn function_to_routine_call_with_placeholders(
    func: &Function,
    default_namespace: &SchemaId,
) -> DdlResult<RoutineCall> {
    let (arguments, named_arguments) = function_args_to_call_arguments(&func.args)?;
    let routine_id = routine_id_from_function_name(&func.name, default_namespace)?;
    Ok(RoutineCall::with_named_arguments(routine_id, arguments, named_arguments))
}

fn builtin_default_text(text: &str) -> Option<BuiltinDefault> {
    let trimmed = text.trim();
    let bare = trimmed.strip_suffix("()").unwrap_or(trimmed).trim();
    let call = format!("{bare}()");
    if !trimmed.eq_ignore_ascii_case(bare) && !trimmed.eq_ignore_ascii_case(&call) {
        return None;
    }
    BuiltinDefault::parse(bare)
}

fn function_to_routine_call(
    func: &Function,
    default_namespace: &SchemaId,
) -> DdlResult<RoutineCall> {
    let call = function_to_routine_call_with_placeholders(func, default_namespace)?;
    if call.arguments.iter().any(CallArgument::is_placeholder) {
        return Err("DEFAULT procedure arguments cannot use placeholders".to_string());
    }
    Ok(call)
}

fn routine_id_from_function_name(
    name: &ObjectName,
    default_namespace: &SchemaId,
) -> DdlResult<RoutineId> {
    let parts: Vec<(String, bool)> = name
        .0
        .iter()
        .map(|part| match part {
            ObjectNamePart::Identifier(ident) => {
                let is_quoted = ident.quote_style.is_some();
                let val = if is_quoted {
                    ident.value.clone()
                } else {
                    ident.value.to_ascii_lowercase()
                };
                (val, is_quoted)
            },
            other => (other.to_string().trim_matches('"').to_ascii_lowercase(), false),
        })
        .collect();
    match parts.as_slice() {
        [(unqualified, false)] if is_builtin_default_name(unqualified) => {
            Ok(RoutineId::new(normalize_builtin_name(unqualified)))
        },
        [(unqualified, _)] => Ok(RoutineId::from_parts(Some(default_namespace), unqualified)),
        [(schema, _), (unqualified, _)] => {
            Ok(RoutineId::from_parts(Some(&SchemaId::new(schema)), unqualified))
        },
        _ => Err(format!("invalid DEFAULT function name '{name}'")),
    }
}

fn function_args_to_call_arguments(
    args: &FunctionArguments,
) -> DdlResult<(Vec<CallArgument>, Vec<Option<String>>)> {
    match args {
        FunctionArguments::None => Ok((Vec::new(), Vec::new())),
        FunctionArguments::List(list) => {
            let mut arguments = Vec::with_capacity(list.args.len());
            let mut named_arguments = Vec::with_capacity(list.args.len());
            let mut saw_named = false;
            let mut seen_names = std::collections::HashSet::new();
            for arg in &list.args {
                match arg {
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::BinaryOp {
                        left,
                        op: sqlparser::ast::BinaryOperator::Assignment,
                        right,
                    })) if matches!(left.as_ref(), Expr::Identifier(_)) => {
                        let Expr::Identifier(name) = left.as_ref() else {
                            unreachable!();
                        };
                        saw_named = true;
                        let param_name = if name.quote_style.is_some() {
                            name.value.clone()
                        } else {
                            name.value.to_ascii_lowercase()
                        };
                        if !seen_names.insert(param_name.clone()) {
                            return Err(format!(
                                "argument name \"{param_name}\" used more than once"
                            ));
                        }
                        arguments.push(expr_to_call_argument(right)?);
                        named_arguments.push(Some(param_name));
                    },
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => {
                        if saw_named {
                            return Err(
                                "positional argument cannot follow named argument".to_string()
                            );
                        }
                        arguments.push(expr_to_call_argument(expr)?);
                        named_arguments.push(None);
                    },
                    FunctionArg::Named {
                        name,
                        arg: FunctionArgExpr::Expr(expr),
                        ..
                    } => {
                        saw_named = true;
                        let param_name = if name.quote_style.is_some() {
                            name.value.clone()
                        } else {
                            name.value.to_ascii_lowercase()
                        };
                        if !seen_names.insert(param_name.clone()) {
                            return Err(format!(
                                "argument name \"{param_name}\" used more than once"
                            ));
                        }
                        arguments.push(expr_to_call_argument(expr)?);
                        named_arguments.push(Some(param_name));
                    },
                    FunctionArg::ExprNamed {
                        name: Expr::Identifier(name),
                        arg: FunctionArgExpr::Expr(expr),
                        ..
                    } => {
                        saw_named = true;
                        let param_name = if name.quote_style.is_some() {
                            name.value.clone()
                        } else {
                            name.value.to_ascii_lowercase()
                        };
                        if !seen_names.insert(param_name.clone()) {
                            return Err(format!(
                                "argument name \"{param_name}\" used more than once"
                            ));
                        }
                        arguments.push(expr_to_call_argument(expr)?);
                        named_arguments.push(Some(param_name));
                    },
                    _ => {
                        return Err(
                            "DEFAULT function arguments must be literal expressions".to_string()
                        );
                    },
                }
            }
            Ok((arguments, named_arguments))
        },
        FunctionArguments::Subquery(_) => {
            Err("DEFAULT function arguments cannot be subqueries".to_string())
        },
    }
}

fn typed_string_to_call_argument(typed: &TypedString) -> DdlResult<CallArgument> {
    let kalam_type = map_sql_type_to_kalam(&typed.data_type)?;
    let value = value_to_json(&typed.value.value)?;
    let value = coerce_json_for_type(&kalam_type, value)?;
    Ok(CallArgument::typed(kalam_type, value))
}

fn negate_numeric_argument(expr: &Expr) -> DdlResult<CallArgument> {
    match expr_to_call_argument(expr)? {
        CallArgument::Typed {
            data_type: KalamDataType::BigInt,
            value,
        } => {
            let number = value
                .as_i64()
                .ok_or_else(|| "negated argument must be an integer".to_string())?;
            Ok(CallArgument::bigint(-number))
        },
        CallArgument::Typed {
            data_type: KalamDataType::Int,
            value,
        } => {
            let number = value
                .as_i64()
                .ok_or_else(|| "negated argument must be an integer".to_string())?;
            Ok(CallArgument::int(-i32::try_from(number).map_err(|_| "INT overflow")?))
        },
        CallArgument::Typed {
            data_type: KalamDataType::Double,
            value,
        } => {
            let number =
                value.as_f64().ok_or_else(|| "negated argument must be a float".to_string())?;
            CallArgument::double(-number)
        },
        other => Err(format!("cannot negate argument {}", other.to_sql())),
    }
}

fn value_to_call_argument(value: &Value) -> DdlResult<CallArgument> {
    match value {
        Value::Null => Ok(CallArgument::Null),
        Value::Boolean(flag) => Ok(CallArgument::boolean(*flag)),
        Value::Number(number, _) => parse_number_argument(number),
        Value::SingleQuotedString(text)
        | Value::DoubleQuotedString(text)
        | Value::NationalStringLiteral(text)
        | Value::EscapedStringLiteral(text)
        | Value::DollarQuotedString(sqlparser::ast::DollarQuotedString { value: text, .. }) => {
            Ok(CallArgument::text(text.clone()))
        },
        Value::HexStringLiteral(hex) => {
            Ok(CallArgument::typed(KalamDataType::Bytes, hex_to_json_bytes(hex)?))
        },
        Value::Placeholder(placeholder) => parse_placeholder(placeholder),
        other => Err(format!("unsupported function argument '{other}'")),
    }
}

fn parse_number_argument(number: &str) -> DdlResult<CallArgument> {
    if let Ok(int_value) = number.parse::<i64>() {
        Ok(CallArgument::bigint(int_value))
    } else if let Ok(float_value) = number.parse::<f64>() {
        CallArgument::double(float_value)
    } else {
        Err(format!("invalid numeric argument '{number}'"))
    }
}

fn parse_placeholder(placeholder: &str) -> DdlResult<CallArgument> {
    let index = placeholder
        .trim_start_matches('$')
        .parse::<usize>()
        .map_err(|_| format!("invalid CALL placeholder '{placeholder}'"))?;
    if index == 0 {
        return Err("CALL placeholders are 1-based".to_string());
    }
    Ok(CallArgument::Placeholder(index))
}

fn expr_to_json_literal(expr: &Expr) -> DdlResult<JsonValue> {
    match expr_to_call_argument(expr)? {
        CallArgument::Null => Ok(JsonValue::Null),
        CallArgument::Typed { value, .. } => Ok(value),
        CallArgument::Tuple(items) => {
            let mut arr = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    CallArgument::Null => arr.push(JsonValue::Null),
                    CallArgument::Typed { value, .. } => arr.push(value),
                    _ => return Err("unsupported nested tuple element".to_string()),
                }
            }
            Ok(JsonValue::Array(arr))
        },
        CallArgument::Placeholder(_) => {
            Err("placeholders cannot be nested inside CAST".to_string())
        },
    }
}

fn value_to_json(value: &Value) -> DdlResult<JsonValue> {
    match value_to_call_argument(value)? {
        CallArgument::Null => Ok(JsonValue::Null),
        CallArgument::Typed { value, .. } => Ok(value),
        CallArgument::Tuple(_) => Err("tuples cannot be nested inside a typed literal".to_string()),
        CallArgument::Placeholder(_) => {
            Err("placeholders cannot be nested inside a typed literal".to_string())
        },
    }
}

fn coerce_json_for_type(data_type: &KalamDataType, value: JsonValue) -> DdlResult<JsonValue> {
    if value.is_null() {
        return Ok(JsonValue::Null);
    }
    match data_type {
        KalamDataType::Json | KalamDataType::File => match value {
            JsonValue::String(text) => serde_json::from_str(&text)
                .map_err(|error| format!("invalid {data_type} argument '{text}': {error}")),
            other => Ok(other),
        },
        KalamDataType::Bytes => match value {
            JsonValue::String(text) if text.chars().all(|ch| ch.is_ascii_hexdigit()) => {
                hex_to_json_bytes(&text)
            },
            other => Ok(other),
        },
        _ => Ok(value),
    }
}

fn hex_to_json_bytes(hex: &str) -> DdlResult<JsonValue> {
    if hex.len() % 2 != 0 {
        return Err(format!("invalid hex literal '{hex}'"));
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let chars: Vec<char> = hex.chars().collect();
    for chunk in chars.chunks(2) {
        let text: String = chunk.iter().collect();
        let byte =
            u8::from_str_radix(&text, 16).map_err(|_| format!("invalid hex literal '{hex}'"))?;
        bytes.push(JsonValue::Number(Number::from(byte)));
    }
    Ok(JsonValue::Array(bytes))
}

fn value_to_literal_default(value: &Value) -> ColumnDefault {
    match value {
        Value::Number(number, _) => {
            if let Ok(int_value) = number.parse::<i64>() {
                ColumnDefault::literal(JsonValue::Number(int_value.into()))
            } else if let Ok(float_value) = number.parse::<f64>() {
                ColumnDefault::literal(serde_json::json!(float_value))
            } else {
                ColumnDefault::literal(JsonValue::String(number.clone()))
            }
        },
        Value::SingleQuotedString(string_value)
        | Value::DoubleQuotedString(string_value)
        | Value::NationalStringLiteral(string_value) => {
            ColumnDefault::literal(JsonValue::String(string_value.clone()))
        },
        Value::Boolean(boolean_value) => ColumnDefault::literal(JsonValue::Bool(*boolean_value)),
        Value::Null => ColumnDefault::literal(JsonValue::Null),
        other => ColumnDefault::literal(JsonValue::String(other.to_string())),
    }
}

pub fn parse_call_argument_sql(input: &str) -> DdlResult<CallArgument> {
    let expr = KalamParser::parse_expr(input).map_err(|error| error.to_string())?;
    expr_to_call_argument(&expr)
}
