use kalamdb_dialect::{parser::KalamParser, KalamDbDialect};
use sqlparser::{
    dialect::{Dialect, PostgreSqlDialect},
    parser::Parser,
};

fn postgres_statements(sql: &str) -> Vec<sqlparser::ast::Statement> {
    let dialect = PostgreSqlDialect {};
    Parser::parse_sql(&dialect, sql).unwrap_or_else(|error| panic!("{sql}: {error}"))
}

#[test]
fn kalam_dialect_preserves_postgresql_type_id() {
    let dialect = KalamDbDialect::new();
    let dialect: &dyn Dialect = &dialect;
    assert!(dialect.is::<PostgreSqlDialect>());
}

#[test]
fn postgresql_syntax_matches_the_upstream_dialect() {
    let corpus = [
        "SELECT 1::int",
        "SELECT ARRAY[1, 2, 3]",
        "SELECT $$dollar quoted$$",
        "SELECT $tag$hello$tag$",
        "SELECT 1 /* outer /* inner */ still */",
        "SELECT '{\"a\":1}'::json -> 'a'",
        "SELECT \"QuotedIdent\"",
    ];

    for sql in corpus {
        let kalam = KalamParser::parse(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
        let postgres = postgres_statements(sql);
        assert_eq!(kalam.len(), postgres.len(), "{sql}");
        for (left, right) in kalam.iter().zip(postgres.iter()) {
            assert_eq!(left.to_string(), right.to_string(), "{sql}");
        }
    }
}
