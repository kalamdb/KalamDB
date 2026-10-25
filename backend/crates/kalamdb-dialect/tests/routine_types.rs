//! Modifier fixtures for routine signatures and composite attributes.

use kalamdb_commons::{models::SchemaId, KalamDataType};
use kalamdb_dialect::ddl::{CreateProcedureStatement, CreateTypeBody, CreateTypeStatement};

fn app() -> SchemaId {
    SchemaId::new("app")
}

#[test]
fn composite_attributes_keep_parenthesized_modifiers() {
    let statement = CreateTypeStatement::parse(
        "CREATE TYPE app.payment AS (amount numeric(12,2), label varchar(10))",
        &app(),
    )
    .expect("composite");
    let CreateTypeBody::Composite { fields } = statement.body else {
        panic!("expected composite");
    };

    let amount = &fields[0].type_ref;
    assert_eq!(amount.syntactic_modifier.as_deref(), Some("12,2"));
    assert_eq!(
        amount.data_type,
        Some(KalamDataType::Decimal {
            precision: 12,
            scale: 2,
        })
    );

    let label = &fields[1].type_ref;
    assert_eq!(label.syntactic_modifier.as_deref(), Some("10"));
    assert_eq!(label.data_type, Some(KalamDataType::Text));
}

#[test]
fn ast_keeps_routine_argument_modifiers() {
    let statement =
        CreateProcedureStatement::parse("CREATE PROCEDURE app.quote(amount numeric(12,2))", &app())
            .expect("procedure");
    let parsed = &statement.parameters[0].type_ref;
    assert_eq!(parsed.name, "numeric");
    assert_eq!(parsed.syntactic_modifier.as_deref(), Some("12,2"));
    assert_eq!(
        parsed.data_type,
        Some(KalamDataType::Decimal {
            precision: 12,
            scale: 2,
        })
    );
}

#[test]
fn function_returns_keeps_syntactic_modifier_and_procedure_returns_is_rejected() {
    let statement = CreateProcedureStatement::parse(
        "CREATE FUNCTION app.charge(amount numeric(12,2)) RETURNS numeric(4,1)",
        &app(),
    )
    .expect("function");
    let returned = statement.return_type.expect("RETURNS is recorded on the function");
    assert_eq!(returned.syntactic_modifier.as_deref(), Some("4,1"));
    assert_eq!(statement.parameters[0].type_ref.syntactic_modifier.as_deref(), Some("12,2"));

    let err = CreateProcedureStatement::parse(
        "CREATE PROCEDURE app.charge(amount numeric(12,2)) RETURNS numeric(4,1)",
        &app(),
    )
    .expect_err("CREATE PROCEDURE ... RETURNS must be rejected in PG18");
    assert!(
        err.contains("CREATE PROCEDURE cannot declare a RETURNS clause"),
        "unexpected error: {err}"
    );
}
