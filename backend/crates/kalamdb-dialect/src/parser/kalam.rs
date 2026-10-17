//! One PostgreSQL syntax entry for KalamDB.
//!
//! Callers share this tokenizer and parser. `Dialect::parse_statement` is not
//! overridden: ordinary PostgreSQL statements stay on the inherited parser,
//! which returns `None` from that hook.

use sqlparser::{
    ast::{Expr, Statement},
    parser::{Parser, ParserError},
    tokenizer::{Token, Tokenizer},
};

use crate::{
    dialect::KalamDbDialect,
    parser::utils::{parse_sql_expression, parser_options, DEFAULT_SQL_RECURSION_LIMIT},
};

/// PostgreSQL syntax parser used by classification, DDL, and planning.
pub struct KalamParser;

impl KalamParser {
    /// Parse one or more statements from `sql`.
    pub fn parse(sql: &str) -> Result<Vec<Statement>, ParserError> {
        Self::parse_with_locations(sql)
    }

    /// Parse `sql` when it contains exactly one statement.
    pub fn parse_one(sql: &str) -> Result<Statement, ParserError> {
        let mut statements = Self::parse(sql)?;
        if statements.len() != 1 {
            return Err(ParserError::ParserError("expected exactly one SQL statement".to_string()));
        }
        Ok(statements.remove(0))
    }

    /// Parse one SQL expression. The expression text is its own source unit.
    pub fn parse_expr(sql: &str) -> Result<Expr, ParserError> {
        parse_sql_expression(sql, &KalamDbDialect::new())
    }

    /// Tokenize `sql` with `KalamDbDialect`.
    pub fn tokenize(sql: &str) -> Result<Vec<Token>, ParserError> {
        let dialect = KalamDbDialect::new();
        Tokenizer::new(&dialect, sql)
            .tokenize()
            .map_err(|error| ParserError::TokenizerError(error.to_string()))
    }

    fn parse_with_locations(sql: &str) -> Result<Vec<Statement>, ParserError> {
        let dialect = KalamDbDialect::new();
        let options = parser_options();
        let tokens = Tokenizer::new(&dialect, sql)
            .with_unescape(options.unescape)
            .tokenize_with_location()
            .map_err(|error| ParserError::TokenizerError(error.to_string()))?;
        Parser::new(&dialect)
            .with_options(options)
            .with_recursion_limit(DEFAULT_SQL_RECURSION_LIMIT)
            .with_tokens_with_locations(tokens)
            .parse_statements()
    }
}
