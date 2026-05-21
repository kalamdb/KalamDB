use kalamdb_commons::models::rows::Row;
use regex::{Regex, RegexBuilder};
use sqlparser::ast::{Expr, Value};

use crate::{
    value::{scalar_as_str, ValueExpr},
    RowFilterError, RowFilterResult,
};

#[derive(Clone, Debug)]
pub(crate) struct LikePredicate {
    value: ValueExpr,
    pattern: ValueExpr,
    literal_regex: Option<Regex>,
    negated: bool,
    escape_char: Option<char>,
    case_insensitive: bool,
}

impl LikePredicate {
    pub(crate) fn compile(
        value: &Expr,
        pattern: &Expr,
        negated: bool,
        any: bool,
        escape_char: Option<&Value>,
        case_insensitive: bool,
    ) -> RowFilterResult<Self> {
        if any {
            return Err(RowFilterError::UnsupportedExpression(
                "LIKE ANY expressions are not supported by row-local filters".to_string(),
            ));
        }

        let value = ValueExpr::compile(value)?;
        let pattern = ValueExpr::compile(pattern)?;
        let escape_char = parse_like_escape_char(escape_char)?;
        let literal_regex = pattern
            .literal_string()
            .map(|literal| compile_like_regex(literal, escape_char, case_insensitive))
            .transpose()?;

        Ok(Self {
            value,
            pattern,
            literal_regex,
            negated,
            escape_char,
            case_insensitive,
        })
    }

    pub(crate) fn evaluate(&self, row_data: &Row) -> RowFilterResult<bool> {
        let value = self.value.evaluate(row_data)?;
        if value.is_null() {
            return Ok(false);
        }
        let value_str = scalar_as_str(value.as_ref()).ok_or_else(|| {
            RowFilterError::InvalidOperation(format!(
                "cannot evaluate LIKE against non-string value: {:?}",
                value
            ))
        })?;

        let matched = if let Some(regex) = &self.literal_regex {
            regex.is_match(value_str)
        } else {
            let pattern = self.pattern.evaluate(row_data)?;
            if pattern.is_null() {
                return Ok(false);
            }
            let pattern_str = scalar_as_str(pattern.as_ref()).ok_or_else(|| {
                RowFilterError::InvalidOperation(format!(
                    "LIKE pattern must be a string literal or column value, got: {:?}",
                    pattern
                ))
            })?;
            compile_like_regex(pattern_str, self.escape_char, self.case_insensitive)?
                .is_match(value_str)
        };

        Ok(if self.negated { !matched } else { matched })
    }
}

fn compile_like_regex(
    pattern: &str,
    escape_char: Option<char>,
    case_insensitive: bool,
) -> RowFilterResult<Regex> {
    let regex_pattern = build_like_regex_pattern(pattern, escape_char)?;
    RegexBuilder::new(&regex_pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|error| {
            RowFilterError::InvalidOperation(format!(
                "invalid LIKE pattern {:?}: {}",
                pattern, error
            ))
        })
}

fn build_like_regex_pattern(pattern: &str, escape_char: Option<char>) -> RowFilterResult<String> {
    let mut regex_pattern = String::with_capacity(pattern.len() + 2);
    regex_pattern.push('^');

    let mut escaped = false;
    for ch in pattern.chars() {
        if escaped {
            regex_pattern.push_str(&regex::escape(&ch.to_string()));
            escaped = false;
            continue;
        }

        if Some(ch) == escape_char {
            escaped = true;
            continue;
        }

        match ch {
            '%' => regex_pattern.push_str(".*"),
            '_' => regex_pattern.push('.'),
            _ => regex_pattern.push_str(&regex::escape(&ch.to_string())),
        }
    }

    if escaped {
        return Err(RowFilterError::InvalidOperation(format!(
            "LIKE pattern has dangling escape character: {:?}",
            pattern
        )));
    }

    regex_pattern.push('$');
    Ok(regex_pattern)
}

fn parse_like_escape_char(escape_char: Option<&Value>) -> RowFilterResult<Option<char>> {
    match escape_char {
        None => Ok(None),
        Some(Value::SingleQuotedString(value)) | Some(Value::DoubleQuotedString(value)) => {
            let mut chars = value.chars();
            let ch = chars.next().ok_or_else(|| {
                RowFilterError::InvalidOperation("LIKE ESCAPE cannot be empty".to_string())
            })?;
            if chars.next().is_some() {
                return Err(RowFilterError::InvalidOperation(format!(
                    "LIKE ESCAPE must be a single character, got {:?}",
                    value
                )));
            }
            Ok(Some(ch))
        },
        Some(other) => Err(RowFilterError::UnsupportedExpression(format!("{:?}", other))),
    }
}
