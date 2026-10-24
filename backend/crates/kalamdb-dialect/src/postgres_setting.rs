//! Parsed session commands shared by every SQL transport.
use kalamdb_session::postgres_settings::canonical_name;
use sqlparser::ast::{ContextModifier, Expr, Reset, Set, Statement, Value};

use crate::parser::KalamParser;

#[derive(Debug, Clone)]
pub enum PostgresSetting {
    Set {
        name:  String,
        value: Option<String>,
        local: bool,
    },
    ResetAll,
    Show {
        name: String,
    },
}

impl PostgresSetting {
    pub fn parse(sql: &str) -> Result<Option<Self>, String> {
        let statement = KalamParser::parse_one(sql).map_err(|e| e.to_string())?;
        Self::from_statement(&statement)
    }
    pub fn from_statement(statement: &Statement) -> Result<Option<Self>, String> {
        let command = match statement {
            Statement::Set(Set::SingleAssignment {
                scope,
                variable,
                values,
                ..
            }) => {
                let name = canonical_name(&variable.to_string().replace('"', ""));
                if name.starts_with("datafusion.") {
                    return Ok(None);
                }
                let value = if values.len() == 1
                    && matches!(&values[0], Expr::Identifier(i) if i.value.eq_ignore_ascii_case("default") && i.quote_style.is_none())
                {
                    None
                } else {
                    Some(
                        values
                            .iter()
                            .map(|e| setting_value(e, name == "search_path"))
                            .collect::<Result<Vec<_>, _>>()?
                            .join(", "),
                    )
                };
                Self::Set {
                    name,
                    value,
                    local: matches!(scope, Some(ContextModifier::Local)),
                }
            },
            Statement::Set(Set::SetTimeZone { local, value }) => Self::Set {
                name:  "timezone".into(),
                value: Some(setting_value(value, false)?),
                local: *local,
            },
            Statement::Reset(reset) => match &reset.reset {
                Reset::ALL => Self::ResetAll,
                Reset::ConfigurationParameter(name) => Self::Set {
                    name:  canonical_name(&name.to_string()),
                    value: None,
                    local: false,
                },
            },
            Statement::ShowVariable { variable } => {
                let parts: Vec<&str> = variable.iter().map(|v| v.value.as_str()).collect();
                let name = show_variable_name(&parts);
                if name.starts_with("datafusion.") || name == "all" {
                    return Ok(None);
                }
                Self::Show { name }
            },
            _ => return Ok(None),
        };
        Ok(Some(command))
    }
}

fn show_variable_name(parts: &[&str]) -> String {
    let phrase = parts.join(" ");
    let canonical = canonical_name(&phrase);
    if canonical != phrase.trim().to_ascii_lowercase() {
        canonical
    } else {
        canonical_name(&parts.join("."))
    }
}

fn setting_value(expr: &Expr, search_path: bool) -> Result<String, String> {
    match expr {
        Expr::Value(v) => match &v.value {
            Value::SingleQuotedString(s) | Value::DoubleQuotedString(s) => Ok(s.clone()),
            Value::Number(s, _) => Ok(s.clone()),
            Value::Boolean(v) => Ok(if *v { "on" } else { "off" }.into()),
            _ => Err("unsupported setting value".into()),
        },
        Expr::Identifier(i) => Ok(if search_path {
            i.to_string()
        } else {
            i.value.clone()
        }),
        Expr::UnaryOp { .. } => Ok(expr.to_string()),
        _ => Err("setting values must be literals".into()),
    }
}
