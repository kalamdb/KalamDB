//! KalamDB SQL dialect.
//!
//! `KalamDbDialect` delegates to PostgreSQL and preserves
//! `TypeId::of::<PostgreSqlDialect>()` so DataFusion and sqlparser treat it as
//! PostgreSQL. `Dialect::parse_prefix` extends PostgreSQL context keywords
//! (`CURRENT_USER`, `CURRENT_ROLE`, `CURRENT_SCHEMA`, `CURRENT_DATABASE`,
//! `CURRENT_CATALOG`, `SESSION_USER`) so both bare keyword forms and empty
//! parenthesized forms (`CURRENT_USER()`, `CURRENT_ROLE()`, etc.) parse
//! natively as zero-argument `Expr::Function` nodes without string/regex
//! pre-normalization.

use std::any::TypeId;

use sqlparser::{
    ast::{Expr, FunctionArguments, ObjectName},
    dialect::{Dialect, PostgreSqlDialect, Precedence},
    keywords::Keyword,
    parser::{Parser, ParserError},
    tokenizer::Token,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct KalamDbDialect {
    dialect: PostgreSqlDialect,
}

impl KalamDbDialect {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Dialect for KalamDbDialect {
    fn dialect(&self) -> TypeId {
        Dialect::dialect(&self.dialect)
    }

    fn parse_prefix(&self, parser: &mut Parser) -> Option<Result<Expr, ParserError>> {
        let Token::Word(w) = &parser.peek_token_ref().token else {
            return self.dialect.parse_prefix(parser);
        };
        let is_context_keyword = matches!(
            w.keyword,
            Keyword::CURRENT_USER
                | Keyword::CURRENT_ROLE
                | Keyword::CURRENT_SCHEMA
                | Keyword::CURRENT_CATALOG
                | Keyword::SESSION_USER
        ) || w.value.eq_ignore_ascii_case("current_database");
        if w.quote_style.is_some() || !is_context_keyword {
            return self.dialect.parse_prefix(parser);
        }

        // Qualified prefix (`current_user.id`) or field access (`u.current_user` without `()`)
        // is an identifier, not a context function call.
        let followed_by_period = parser.peek_nth_token_ref(1).token == Token::Period;
        let preceded_by_period_without_call = parser.get_current_token().token == Token::Period
            && parser.peek_nth_token_ref(1).token != Token::LParen;
        if followed_by_period || preceded_by_period_without_call {
            let tok = parser.next_token();
            let Token::Word(w) = tok.token else {
                unreachable!();
            };
            return Some(Ok(Expr::Identifier(w.into_ident(tok.span))));
        }

        let tok = parser.next_token();
        let Token::Word(w) = tok.token else {
            unreachable!();
        };
        let name = ObjectName::from(vec![w.into_ident(tok.span)]);
        Some(parser.parse_time_functions(name).map(|expr| match expr {
            Expr::Function(mut func) => {
                if let FunctionArguments::List(list) = &func.args {
                    if list.args.is_empty()
                        && list.duplicate_treatment.is_none()
                        && list.clauses.is_empty()
                    {
                        func.args = FunctionArguments::None;
                    }
                }
                Expr::Function(func)
            },
            other => other,
        }))
    }

    fn identifier_quote_style(&self, identifier: &str) -> Option<char> {
        self.dialect.identifier_quote_style(identifier)
    }

    fn is_delimited_identifier_start(&self, ch: char) -> bool {
        self.dialect.is_delimited_identifier_start(ch)
    }

    fn is_identifier_start(&self, ch: char) -> bool {
        self.dialect.is_identifier_start(ch)
    }

    fn is_identifier_part(&self, ch: char) -> bool {
        self.dialect.is_identifier_part(ch)
    }

    fn supports_unicode_string_literal(&self) -> bool {
        self.dialect.supports_unicode_string_literal()
    }

    fn is_reserved_for_identifier(&self, kw: Keyword) -> bool {
        self.dialect.is_reserved_for_identifier(kw)
    }

    fn is_custom_operator_part(&self, ch: char) -> bool {
        self.dialect.is_custom_operator_part(ch)
    }

    fn get_next_precedence(&self, parser: &Parser) -> Option<Result<u8, ParserError>> {
        self.dialect.get_next_precedence(parser)
    }

    fn supports_filter_during_aggregation(&self) -> bool {
        self.dialect.supports_filter_during_aggregation()
    }

    fn supports_group_by_expr(&self) -> bool {
        self.dialect.supports_group_by_expr()
    }

    fn prec_value(&self, prec: Precedence) -> u8 {
        self.dialect.prec_value(prec)
    }

    fn allow_extract_custom(&self) -> bool {
        self.dialect.allow_extract_custom()
    }

    fn allow_extract_single_quotes(&self) -> bool {
        self.dialect.allow_extract_single_quotes()
    }

    fn supports_create_index_with_clause(&self) -> bool {
        self.dialect.supports_create_index_with_clause()
    }

    fn supports_explain_with_utility_options(&self) -> bool {
        self.dialect.supports_explain_with_utility_options()
    }

    fn supports_listen_notify(&self) -> bool {
        self.dialect.supports_listen_notify()
    }

    fn supports_factorial_operator(&self) -> bool {
        self.dialect.supports_factorial_operator()
    }

    fn supports_bitwise_shift_operators(&self) -> bool {
        self.dialect.supports_bitwise_shift_operators()
    }

    fn supports_comment_on(&self) -> bool {
        self.dialect.supports_comment_on()
    }

    fn supports_load_extension(&self) -> bool {
        self.dialect.supports_load_extension()
    }

    fn supports_named_fn_args_with_colon_operator(&self) -> bool {
        self.dialect.supports_named_fn_args_with_colon_operator()
    }

    fn supports_named_fn_args_with_expr_name(&self) -> bool {
        self.dialect.supports_named_fn_args_with_expr_name()
    }

    fn supports_empty_projections(&self) -> bool {
        self.dialect.supports_empty_projections()
    }

    fn supports_nested_comments(&self) -> bool {
        self.dialect.supports_nested_comments()
    }

    fn supports_string_escape_constant(&self) -> bool {
        self.dialect.supports_string_escape_constant()
    }

    fn supports_numeric_literal_underscores(&self) -> bool {
        self.dialect.supports_numeric_literal_underscores()
    }

    fn supports_array_typedef_with_brackets(&self) -> bool {
        self.dialect.supports_array_typedef_with_brackets()
    }

    fn supports_geometric_types(&self) -> bool {
        self.dialect.supports_geometric_types()
    }

    fn supports_set_names(&self) -> bool {
        self.dialect.supports_set_names()
    }

    fn supports_alter_column_type_using(&self) -> bool {
        self.dialect.supports_alter_column_type_using()
    }

    fn supports_notnull_operator(&self) -> bool {
        self.dialect.supports_notnull_operator()
    }

    fn supports_interval_options(&self) -> bool {
        self.dialect.supports_interval_options()
    }

    fn supports_insert_table_alias(&self) -> bool {
        self.dialect.supports_insert_table_alias()
    }

    fn supports_create_table_like_parenthesized(&self) -> bool {
        self.dialect.supports_create_table_like_parenthesized()
    }

    fn supports_select_wildcard_with_alias(&self) -> bool {
        self.dialect.supports_select_wildcard_with_alias()
    }

    fn supports_comma_separated_trim(&self) -> bool {
        self.dialect.supports_comma_separated_trim()
    }

    fn supports_xml_expressions(&self) -> bool {
        self.dialect.supports_xml_expressions()
    }

    fn supports_comment_optimizer_hint(&self) -> bool {
        self.dialect.supports_comment_optimizer_hint()
    }
}
