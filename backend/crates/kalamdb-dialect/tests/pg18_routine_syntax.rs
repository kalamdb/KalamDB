//! T125: PostgreSQL 18 routine and type syntax, quoting, comment, and delimiter coverage.

use kalamdb_commons::{
    models::{CallArgument, RoutineKind, SchemaId},
    Role,
};
use kalamdb_dialect::{
    classifier::{SqlStatement, SqlStatementKind},
    ddl::{CreateProcedureStatement, CreateTypeBody, CreateTypeStatement, DropProcedureStatement},
};

fn classify(sql: &str) -> Result<SqlStatement, String> {
    SqlStatement::classify_and_parse(sql, &SchemaId::new("public"), Role::Dba)
        .map_err(|err| err.user_message().to_string())
}

#[test]
fn quoted_doubled_and_embedded_dot_identifiers_preserve_exact_names() {
    let stmt = classify(
        r#"CREATE FUNCTION "api"."a.b""c"("x.y" integer, "default" text, "__proto__" boolean)
           RETURNS integer
           LANGUAGE javascript
           AS $fn$ return input["x.y"]; $fn$;"#,
    )
    .expect("classify quoted routine");

    let SqlStatementKind::CreateProcedure(proc) = stmt.kind() else {
        panic!("expected CreateProcedure, got {:?}", stmt.kind());
    };
    assert_eq!(proc.namespace_id.as_str(), "api");
    assert_eq!(proc.name, r#"a.b"c"#);
    assert_eq!(proc.kind, RoutineKind::Function);
    assert_eq!(proc.parameters.len(), 3);
    assert_eq!(proc.parameters[0].name, "x.y");
    assert_eq!(proc.parameters[1].name, "default");
    assert_eq!(proc.parameters[2].name, "__proto__");
    assert_eq!(proc.language.as_deref(), Some("JAVASCRIPT"));
}

#[test]
fn unicode_and_byte_limit_enforcement_follows_pg_namedatalen_bytes() {
    let valid_63_bytes = "é".repeat(31) + "a"; // 31 * 2 + 1 = 63 bytes
    assert_eq!(valid_63_bytes.len(), 63);
    let sql_ok = format!(
        "CREATE FUNCTION public.\"{valid_63_bytes}\"(x integer) RETURNS integer LANGUAGE sql AS \
         $$ SELECT 1 $$"
    );
    assert!(classify(&sql_ok).is_ok(), "63-byte UTF-8 identifier must succeed");

    let invalid_64_bytes = "é".repeat(32); // 64 bytes (32 chars)
    assert_eq!(invalid_64_bytes.len(), 64);
    let sql_err = format!(
        "CREATE FUNCTION public.\"{invalid_64_bytes}\"(x integer) RETURNS integer LANGUAGE sql AS \
         $$ SELECT 1 $$"
    );
    let err = classify(&sql_err).expect_err("64-byte UTF-8 identifier must fail byte limit");
    assert!(err.contains("63 bytes"), "unexpected error: {err}");
}

#[test]
fn language_identifiers_fold_unquoted_and_preserve_quoted_case() {
    let unquoted = CreateProcedureStatement::parse(
        "CREATE FUNCTION public.f1() RETURNS int LANGUAGE PlPgSqL AS $$ BEGIN RETURN 1; END; $$",
        &SchemaId::new("public"),
    )
    .expect("unquoted mixed-case language");
    assert_eq!(unquoted.language.as_deref(), Some("PLPGSQL"));

    let quoted_lower = CreateProcedureStatement::parse(
        r#"CREATE FUNCTION public.f2() RETURNS int LANGUAGE "sql" AS $$ SELECT 1 $$"#,
        &SchemaId::new("public"),
    )
    .expect("quoted lowercase language");
    assert_eq!(quoted_lower.language.as_deref(), Some("SQL"));

    let quoted_upper = CreateProcedureStatement::parse(
        r#"CREATE FUNCTION public.f3() RETURNS int LANGUAGE "SQL" AS $$ SELECT 1 $$"#,
        &SchemaId::new("public"),
    )
    .expect_err("quoted uppercase language must be rejected as distinct from 'sql'");
    assert!(
        quoted_upper.contains("language \"SQL\" does not exist"),
        "unexpected error: {quoted_upper}"
    );
}

#[test]
fn tagged_dollar_quotes_escape_strings_and_as_language_order_parse_identically() {
    let as_before_lang = classify(
        r#"CREATE OR REPLACE FUNCTION public.echo_tag(msg text)
           RETURNS text
           AS $outer_tag$
               SELECT 'inner $$ and $other$ stays intact; -- not a comment';
           $outer_tag$
           LANGUAGE sql
           IMMUTABLE;"#,
    )
    .expect("AS before LANGUAGE");

    let lang_before_as = classify(
        r#"CREATE OR REPLACE FUNCTION public.echo_tag(msg text)
           RETURNS text
           LANGUAGE sql
           IMMUTABLE
           AS E'SELECT \'escaped\';'"#,
    )
    .expect("LANGUAGE before AS with E'...'");

    let SqlStatementKind::CreateProcedure(p1) = as_before_lang.kind() else {
        panic!("expected CreateProcedure");
    };
    let SqlStatementKind::CreateProcedure(p2) = lang_before_as.kind() else {
        panic!("expected CreateProcedure");
    };
    assert!(p1.body.as_deref().unwrap().contains("$$ and $other$"));
    assert_eq!(p2.body.as_deref(), Some("SELECT 'escaped';"));

    // Invalid dollar tag with hyphen must be rejected
    let bad_tag = CreateProcedureStatement::parse(
        "CREATE FUNCTION public.bad() RETURNS int LANGUAGE sql AS $bad-tag$ SELECT 1 $bad-tag$",
        &SchemaId::new("public"),
    );
    assert!(bad_tag.is_err(), "dollar quote tag with hyphen must be rejected");
}

#[test]
fn nested_block_comments_and_line_comments_around_clauses_and_trailing_tokens() {
    let sql = r#"
        /* outer comment /* nested inner comment */ still outer */
        CREATE /* c1 */ FUNCTION /* c2 */ public.commented(
            a /* c3 */ double precision, -- line comment inside parameter list
            b character varying(20) DEFAULT 'ok'
        )
        RETURNS /* c4 */ timestamp with time zone
        LANGUAGE /* c5 */ sql
        AS $$ SELECT NOW() $$ /* trailing block comment */ ; -- trailing line comment
    "#;
    let stmt = classify(sql).expect("nested comments must parse");
    let SqlStatementKind::CreateProcedure(proc) = stmt.kind() else {
        panic!("expected CreateProcedure");
    };
    assert_eq!(proc.name, "commented");
    assert_eq!(proc.parameters.len(), 2);

    // Trailing unexpected tokens after routine body or semicolon must be rejected
    let trailing_err = classify(
        "CREATE FUNCTION public.bad_trail() RETURNS int LANGUAGE sql AS $$ SELECT 1 $$ EXTRA_TOKEN",
    );
    assert!(trailing_err.is_err(), "trailing token after body must fail");

    let drop_trailing_err = DropProcedureStatement::parse(
        "DROP FUNCTION public.commented(double precision, varchar) CASCADE EXTRA",
        &SchemaId::new("public"),
    );
    assert!(drop_trailing_err.is_err(), "trailing token after DROP CASCADE must fail");
}

#[test]
fn call_supports_named_arrows_tuple_literals_and_quoted_targets() {
    let stmt = classify(r#"CALL "api"."Run.Job"(target => ROW('alpha', 42), "count" := 3)"#)
        .expect("CALL with named and ROW args");
    let SqlStatementKind::Call(call) = stmt.kind() else {
        panic!("expected Call");
    };
    assert_eq!(call.call.routine_id.namespace_id(), Some(SchemaId::new("api")));
    assert_eq!(call.call.unqualified_name(), "Run.Job");
    assert_eq!(
        call.call.named_arguments,
        vec![Some("target".to_string()), Some("count".to_string())]
    );
    assert!(matches!(&call.call.arguments[0], CallArgument::Tuple(items) if items.len() == 2));
}

#[test]
fn quoted_builtin_like_type_names_are_not_treated_as_builtins() {
    let stmt = CreateTypeStatement::parse(
        r#"CREATE TYPE public."Holder" AS (raw_int "INT", std_int INT)"#,
        &SchemaId::new("public"),
    )
    .expect("create type with quoted INT");
    let CreateTypeBody::Composite { fields } = stmt.body else {
        panic!("expected composite");
    };
    assert_eq!(fields[0].type_ref.name, "INT");
    assert!(
        fields[0].type_ref.data_type.is_none(),
        "quoted uppercase \"INT\" must resolve as a named user type, not builtin INT"
    );
    assert!(fields[1].type_ref.data_type.is_some());
}
