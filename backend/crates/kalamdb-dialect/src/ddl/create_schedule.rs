//! KalamDB schedule extension, tokenized with PostgreSQL quoting and comments.
use std::collections::HashSet;

use kalamdb_commons::models::{NamespaceId, RoutineId, ScheduleId};
use sqlparser::{
    dialect::PostgreSqlDialect,
    tokenizer::{Token, Tokenizer},
};

use crate::ddl::DdlResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateScheduleStatement {
    pub schedule_id:  ScheduleId,
    pub namespace_id: NamespaceId,
    pub name:         String,
    pub routine_id:   RoutineId,
    pub cron:         Option<String>,
    pub interval_ms:  Option<i64>,
    pub timezone:     String,
    pub principal:    String,
}

impl CreateScheduleStatement {
    pub fn parse(sql: &str, namespace: &NamespaceId) -> DdlResult<Self> {
        let mut p = ScheduleParser::new(sql)?;
        p.keyword("CREATE")?;
        p.keyword("SCHEDULE")?;
        let (namespace_id, name) = p.name(namespace)?;
        let schedule_id = ScheduleId::from_parts(Some(&namespace_id), &name);
        let (cron, interval_ms) = if p.take_keyword("CRON") {
            (Some(p.string()?), None)
        } else {
            p.keyword("INTERVAL")?;
            let value = p.string()?;
            let duration =
                humantime::parse_duration(&value).map_err(|e| format!("Invalid INTERVAL: {e}"))?;
            let ms = i64::try_from(duration.as_millis()).map_err(|_| "Interval overflow")?;
            (None, Some(ms))
        };
        let timezone = if p.take_keyword("TIME") {
            p.keyword("ZONE")?;
            p.string()?
        } else {
            "UTC".into()
        };
        p.keyword("EXECUTE")?;
        p.keyword("PROCEDURE")?;
        let (routine_namespace, routine_name) = p.name(&namespace_id)?;
        let routine_id = RoutineId::new(format!("{routine_namespace}.{routine_name}"));
        p.expect(Token::LParen)?;
        p.expect(Token::RParen)?;
        let mut principal = String::new();
        if p.take_keyword("WITH") {
            p.expect(Token::LParen)?;
            let mut seen = HashSet::new();
            loop {
                let key = p.ident()?;
                if !seen.insert(key.clone()) {
                    return Err(format!("Duplicate schedule option '{key}'"));
                }
                p.expect(Token::Eq)?;
                let value = p.string()?;
                match key.as_str() {
                    "principal" if !value.is_empty() => principal = value,
                    "overlap" | "misfire" if value == "skip" => {},
                    "overlap" | "misfire" => {
                        return Err(format!("V1 supports only {key} = 'skip'"))
                    },
                    _ => return Err(format!("Unknown or invalid schedule option '{key}'")),
                }
                if !p.take(Token::Comma) {
                    break;
                }
            }
            p.expect(Token::RParen)?;
        }
        p.end()?;
        kalamdb_system::providers::catalog::schedule_timing::next_schedule_run(
            cron.as_deref(),
            interval_ms,
            &timezone,
            0,
        )?;
        Ok(Self {
            schedule_id,
            namespace_id,
            name,
            routine_id,
            cron,
            interval_ms,
            timezone,
            principal,
        })
    }
}

pub(super) struct ScheduleParser {
    tokens: std::collections::VecDeque<Token>,
}
impl ScheduleParser {
    pub fn new(sql: &str) -> DdlResult<Self> {
        let tokens = Tokenizer::new(&PostgreSqlDialect {}, sql)
            .tokenize()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|t| !matches!(t, Token::Whitespace(_)))
            .collect();
        Ok(Self { tokens })
    }
    pub fn take_keyword(&mut self, word: &str) -> bool {
        if matches!(self.tokens.front(), Some(Token::Word(w)) if w.quote_style.is_none() && w.value.eq_ignore_ascii_case(word))
        {
            self.tokens.pop_front();
            true
        } else {
            false
        }
    }
    pub fn keyword(&mut self, word: &str) -> DdlResult<()> {
        if self.take_keyword(word) {
            Ok(())
        } else {
            Err(format!("Expected {word}"))
        }
    }
    pub fn take(&mut self, token: Token) -> bool {
        if self.tokens.front() == Some(&token) {
            self.tokens.pop_front();
            true
        } else {
            false
        }
    }
    pub fn expect(&mut self, token: Token) -> DdlResult<()> {
        if self.tokens.front() == Some(&token) {
            self.tokens.pop_front();
            Ok(())
        } else {
            Err(format!("Expected {token}"))
        }
    }
    fn ident(&mut self) -> DdlResult<String> {
        match self.tokens.pop_front() {
            Some(Token::Word(w)) if w.quote_style.is_none() || w.quote_style == Some('"') => {
                if w.value.is_empty() {
                    return Err("Empty identifier".into());
                }
                Ok(if w.quote_style.is_some() {
                    w.value
                } else {
                    w.value.to_ascii_lowercase()
                })
            },
            _ => Err("Expected PostgreSQL identifier".into()),
        }
    }
    pub fn name(&mut self, namespace: &NamespaceId) -> DdlResult<(NamespaceId, String)> {
        let first = self.ident()?;
        if self.take(Token::Period) {
            Ok((NamespaceId::new(first), self.ident()?))
        } else {
            Ok((namespace.clone(), first))
        }
    }
    fn string(&mut self) -> DdlResult<String> {
        match self.tokens.pop_front() {
            Some(Token::SingleQuotedString(s)) => Ok(s),
            _ => Err("Expected single-quoted string".into()),
        }
    }
    pub fn end(&mut self) -> DdlResult<()> {
        self.take(Token::SemiColon);
        if self.tokens.is_empty() {
            Ok(())
        } else {
            Err("Unexpected trailing schedule SQL".into())
        }
    }
}
