//! SQL and PL/pgSQL statement execution for a routine body.

use datafusion::scalar::ScalarValue;
use kalamdb_functions::{FunctionsError, RoutineValue};
use kalamdb_system::CatalogRoutine;

use crate::{
    error::KalamDbError,
    sql::context::{ExecutionContext, ExecutionResult},
};

pub(super) fn extract_plpgsql_declare_and_body(
    body: &str,
) -> (Vec<(String, Option<String>)>, Option<&str>) {
    let trimmed = body.trim().trim_end_matches(';').trim();
    let upper = trimmed.to_ascii_uppercase();
    let Some(begin_pos) = find_top_level_keyword(&upper, "BEGIN") else {
        return (Vec::new(), None);
    };
    let mut decls = Vec::new();
    let before_begin = trimmed[..begin_pos].trim();
    if let Some(after_declare) = strip_keyword_prefix_ci(before_begin, "DECLARE") {
        for raw_decl in split_plpgsql_statements(after_declare) {
            let decl = raw_decl.trim();
            if decl.is_empty() {
                continue;
            }
            if let Some((lhs, rhs)) = decl.split_once(":=").or_else(|| decl.split_once('=')) {
                let var_name = lhs.split_whitespace().next().unwrap_or("").trim().to_string();
                if !var_name.is_empty() {
                    decls.push((var_name, Some(rhs.trim().to_string())));
                }
            } else {
                let var_name = decl.split_whitespace().next().unwrap_or("").trim().to_string();
                if !var_name.is_empty() {
                    decls.push((var_name, None));
                }
            }
        }
    }
    let after_begin = &trimmed[begin_pos + "BEGIN".len()..];
    if !after_begin.starts_with(|c: char| c.is_whitespace() || c == ';') {
        return (decls, None);
    }
    let after_begin = after_begin.trim();
    let after_upper = after_begin.to_ascii_uppercase();
    let stripped = if after_upper.ends_with("END") {
        &after_begin[..after_begin.len() - 3]
    } else {
        after_begin
    };
    (decls, Some(stripped.trim()))
}

fn find_top_level_keyword(upper: &str, keyword: &str) -> Option<usize> {
    let bytes = upper.as_bytes();
    let kw = keyword.as_bytes();
    if bytes.len() < kw.len() {
        return None;
    }
    for i in 0..=(bytes.len() - kw.len()) {
        if &bytes[i..i + kw.len()] == kw {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            let after_idx = i + kw.len();
            let after_ok = after_idx == bytes.len()
                || (!bytes[after_idx].is_ascii_alphanumeric() && bytes[after_idx] != b'_');
            if before_ok && after_ok {
                return Some(i);
            }
        }
    }
    None
}

fn split_plpgsql_statements(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0usize;
    let mut start = 0usize;
    let mut paren_depth = 0i32;
    let mut block_depth = 0i32;

    while i < bytes.len() {
        let ch = bytes[i];
        if ch == b'\'' || ch == b'"' {
            let quote = ch;
            i += 1;
            while i < bytes.len() {
                if bytes[i] == quote {
                    i += 1;
                    if i < bytes.len() && bytes[i] == quote {
                        i += 1;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            continue;
        }
        if ch == b'(' {
            paren_depth += 1;
            i += 1;
            continue;
        }
        if ch == b')' {
            paren_depth = (paren_depth - 1).max(0);
            i += 1;
            continue;
        }
        if paren_depth == 0 && (ch.is_ascii_alphabetic() || ch == b'_') {
            let word_start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &body[word_start..i];
            let preceded_by_end = body[start..word_start]
                .split_whitespace()
                .last()
                .is_some_and(|prev| prev.eq_ignore_ascii_case("END"));
            if (word.eq_ignore_ascii_case("IF") || word.eq_ignore_ascii_case("LOOP"))
                && !preceded_by_end
            {
                block_depth += 1;
            } else if (word.eq_ignore_ascii_case("IF") || word.eq_ignore_ascii_case("LOOP"))
                && preceded_by_end
            {
                block_depth = (block_depth - 1).max(0);
            }
            continue;
        }
        if ch == b';' && paren_depth == 0 && block_depth == 0 {
            let piece = body[start..i].trim();
            if !piece.is_empty() {
                out.push(piece.to_string());
            }
            i += 1;
            start = i;
            continue;
        }
        i += 1;
    }
    let tail = body[start..].trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

pub(super) async fn eval_plpgsql_expr(
    executor: &crate::sql::executor::SqlExecutor,
    ctx: &ExecutionContext,
    expr_sql: &str,
    env_args: &[(kalamdb_system::CatalogRoutineParameter, RoutineValue)],
) -> Result<RoutineValue, KalamDbError> {
    let select_sql = format!("SELECT {expr_sql}");
    let sql = substitute_sql_routine_parameters(&select_sql, env_args);
    let params: Vec<ScalarValue> = env_args.iter().map(|(_, val)| val.value.clone()).collect();
    let metadata = executor
        .prepare_statement_metadata(&sql, ctx)
        .map_err(|err| KalamDbError::InvalidSql(err.to_string()))?;
    let result = Box::pin(executor.execute_with_metadata(&metadata, ctx, params)).await?;
    super::convert::execution_result_to_routine(result)
}

async fn eval_plpgsql_bool(
    executor: &crate::sql::executor::SqlExecutor,
    ctx: &ExecutionContext,
    cond_sql: &str,
    env_args: &[(kalamdb_system::CatalogRoutineParameter, RoutineValue)],
) -> Result<bool, KalamDbError> {
    let val = eval_plpgsql_expr(executor, ctx, cond_sql, env_args).await?;
    Ok(matches!(val.value, ScalarValue::Boolean(Some(true))))
}

pub(super) fn execute_plpgsql_block<'a>(
    executor: &'a crate::sql::executor::SqlExecutor,
    ctx: &'a ExecutionContext,
    routine: &'a CatalogRoutine,
    block_sql: &'a str,
    env_args: &'a mut Vec<(kalamdb_system::CatalogRoutineParameter, RoutineValue)>,
) -> std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<(Option<RoutineValue>, Option<ExecutionResult>), KalamDbError>,
            > + Send
            + 'a,
    >,
> {
    Box::pin(async move {
        let statements = split_plpgsql_statements(block_sql);
        let mut last_result = None;
        for raw_sql in statements {
            let trimmed_stmt = raw_sql.trim().trim_end_matches(';').trim();
            if trimmed_stmt.is_empty()
                || trimmed_stmt.eq_ignore_ascii_case("BEGIN")
                || trimmed_stmt.eq_ignore_ascii_case("END")
                || trimmed_stmt.eq_ignore_ascii_case("NULL")
            {
                continue;
            }
            if let Some(rest) = strip_keyword_prefix_ci(trimmed_stmt, "RETURN") {
                let expr_sql = rest.trim();
                if expr_sql.is_empty() {
                    return Ok((Some(RoutineValue::new(ScalarValue::Null)), last_result));
                }
                let val = eval_plpgsql_expr(executor, ctx, expr_sql, env_args).await?;
                return Ok((Some(val), last_result));
            }
            if let Some((cond_sql, loop_body)) = parse_while_loop_stmt(trimmed_stmt) {
                let mut iterations = 0usize;
                while eval_plpgsql_bool(executor, ctx, cond_sql, env_args).await? {
                    iterations += 1;
                    if iterations > 10_000 {
                        return Err(KalamDbError::from(FunctionsError::ResourceLimit(
                            "PL/pgSQL loop iteration limit".into(),
                        )));
                    }
                    let (ret, res) =
                        execute_plpgsql_block(executor, ctx, routine, loop_body, env_args).await?;
                    if res.is_some() {
                        last_result = res;
                    }
                    if ret.is_some() {
                        return Ok((ret, last_result));
                    }
                }
                continue;
            }
            if let Some((cond_sql, then_body, else_body)) = parse_if_stmt(trimmed_stmt) {
                if eval_plpgsql_bool(executor, ctx, cond_sql, env_args).await? {
                    let (ret, res) =
                        execute_plpgsql_block(executor, ctx, routine, then_body, env_args).await?;
                    if res.is_some() {
                        last_result = res;
                    }
                    if ret.is_some() {
                        return Ok((ret, last_result));
                    }
                } else if let Some(else_sql) = else_body {
                    let (ret, res) =
                        execute_plpgsql_block(executor, ctx, routine, else_sql, env_args).await?;
                    if res.is_some() {
                        last_result = res;
                    }
                    if ret.is_some() {
                        return Ok((ret, last_result));
                    }
                }
                continue;
            }
            if let Some((lhs, rhs)) = split_plpgsql_assignment(trimmed_stmt) {
                let assigned_val = eval_plpgsql_expr(executor, ctx, rhs, env_args).await?;
                if let Some((_, slot)) =
                    env_args.iter_mut().find(|(p, _)| p.name.eq_ignore_ascii_case(lhs))
                {
                    *slot = assigned_val;
                }
                continue;
            }
            let sql = substitute_sql_routine_parameters(trimmed_stmt, env_args);
            let params: Vec<ScalarValue> =
                env_args.iter().map(|(_, val)| val.value.clone()).collect();
            let metadata = executor
                .prepare_statement_metadata(&sql, ctx)
                .map_err(|err| KalamDbError::InvalidSql(err.to_string()))?;
            if metadata.classified_statement.as_ref().is_some_and(|statement| {
                matches!(
                    statement.kind(),
                    kalamdb_sql::SqlStatementKind::BeginTransaction
                        | kalamdb_sql::SqlStatementKind::CommitTransaction
                        | kalamdb_sql::SqlStatementKind::RollbackTransaction
                )
            }) {
                return Err(KalamDbError::from(FunctionsError::Invalid(
                    "transaction control is not allowed inside a routine".into(),
                )));
            }
            if matches!(
                routine.volatility,
                kalamdb_commons::models::RoutineVolatility::Immutable
                    | kalamdb_commons::models::RoutineVolatility::Stable
            ) && metadata.classified_statement.as_ref().is_some_and(|statement| {
                !matches!(
                    statement.kind(),
                    kalamdb_sql::SqlStatementKind::Select
                        | kalamdb_sql::SqlStatementKind::DataFusionMetaCommand
                        | kalamdb_sql::SqlStatementKind::SetSearchPath(_)
                )
            }) {
                return Err(KalamDbError::from(FunctionsError::Invalid(format!(
                    "{} routine {} cannot execute read-write or DDL statements",
                    routine.volatility.as_str(),
                    routine.routine_id
                ))));
            }
            let result = Box::pin(executor.execute_with_metadata(&metadata, ctx, params)).await?;
            last_result = Some(result);
        }
        Ok((None, last_result))
    })
}

fn parse_while_loop_stmt(stmt: &str) -> Option<(&str, &str)> {
    let rest = strip_keyword_prefix_ci(stmt.trim(), "WHILE")?.trim();
    let upper = rest.to_ascii_uppercase();
    if !upper.ends_with("END LOOP") {
        return None;
    }
    let without_end = rest[..rest.len() - "END LOOP".len()].trim();
    let upper_inner = without_end.to_ascii_uppercase();
    let loop_pos = find_top_level_keyword(&upper_inner, "LOOP")?;
    let cond = without_end[..loop_pos].trim();
    let body = without_end[loop_pos + "LOOP".len()..].trim();
    Some((cond, body))
}

fn parse_if_stmt(stmt: &str) -> Option<(&str, &str, Option<&str>)> {
    let rest = strip_keyword_prefix_ci(stmt.trim(), "IF")?.trim();
    let upper = rest.to_ascii_uppercase();
    if !upper.ends_with("END IF") {
        return None;
    }
    let without_end = rest[..rest.len() - "END IF".len()].trim();
    let upper_inner = without_end.to_ascii_uppercase();
    let then_pos = find_top_level_keyword(&upper_inner, "THEN")?;
    let cond = without_end[..then_pos].trim();
    let after_then = without_end[then_pos + "THEN".len()..].trim();
    let upper_after_then = after_then.to_ascii_uppercase();
    if let Some(else_pos) = find_top_level_keyword(&upper_after_then, "ELSE") {
        let then_body = after_then[..else_pos].trim();
        let else_body = after_then[else_pos + "ELSE".len()..].trim();
        Some((cond, then_body, Some(else_body)))
    } else {
        Some((cond, after_then, None))
    }
}

fn strip_keyword_prefix_ci<'a>(input: &'a str, keyword: &str) -> Option<&'a str> {
    if input.len() < keyword.len() {
        return None;
    }
    let (prefix, rest) = input.split_at(keyword.len());
    if !prefix.eq_ignore_ascii_case(keyword) {
        return None;
    }
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        Some(rest)
    } else {
        None
    }
}

fn split_plpgsql_assignment(stmt: &str) -> Option<(&str, &str)> {
    let (lhs, rhs) = stmt.split_once(":=")?;
    let lhs = lhs.trim();
    let rhs = rhs.trim();
    if lhs.is_empty()
        || rhs.is_empty()
        || !lhs.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return None;
    }
    Some((lhs, rhs))
}

fn substitute_sql_routine_parameters(
    sql: &str,
    input_args: &[(kalamdb_system::CatalogRoutineParameter, RoutineValue)],
) -> String {
    if input_args.is_empty() {
        return sql.to_string();
    }
    let mut named: Vec<(&str, usize)> = input_args
        .iter()
        .enumerate()
        .filter_map(|(idx, (param, _))| {
            let trimmed = param.name.trim();
            if trimmed.is_empty() || trimmed.starts_with('$') {
                None
            } else {
                Some((trimmed, idx + 1))
            }
        })
        .collect();
    if named.is_empty() {
        return sql.to_string();
    }
    named.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

    let mut out = String::with_capacity(sql.len() + 16);
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        if ch == b'\'' || ch == b'"' {
            let quote = ch;
            out.push(quote as char);
            i += 1;
            while i < bytes.len() {
                let c = bytes[i];
                out.push(c as char);
                i += 1;
                if c == quote {
                    if i < bytes.len() && bytes[i] == quote {
                        out.push(bytes[i] as char);
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
            continue;
        }
        if ch.is_ascii_alphabetic() || ch == b'_' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let ident = &sql[start..i];
            let preceded_by_dot = start > 0 && bytes[start - 1] == b'.';
            let followed_by_paren = sql[i..].trim_start().starts_with('(');
            if !preceded_by_dot && !followed_by_paren {
                if let Some((_, pos)) =
                    named.iter().find(|(name, _)| name.eq_ignore_ascii_case(ident))
                {
                    out.push('$');
                    out.push_str(&pos.to_string());
                    continue;
                }
            }
            out.push_str(ident);
            continue;
        }
        out.push(ch as char);
        i += 1;
    }
    out
}
