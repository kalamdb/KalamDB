//! Effective routine signatures discard parenthesized type modifiers.

use kalamdb_commons::{models::NamespaceId, KalamDataType};
use kalamdb_sql::ddl::{CreateProcedureStatement, CreateTypeBody, CreateTypeStatement};

fn app() -> NamespaceId {
    NamespaceId::new("app")
}

#[test]
fn function_binding_discards_parenthesized_modifiers() {
    let wide =
        CreateProcedureStatement::parse("CREATE PROCEDURE app.wide(amount numeric(12,2))", &app())
            .expect("wide");
    let narrow =
        CreateProcedureStatement::parse("CREATE PROCEDURE app.narrow(amount numeric(4,1))", &app())
            .expect("narrow");

    let wide_arg = &wide.parameters[0].type_ref;
    let narrow_arg = &narrow.parameters[0].type_ref;
    assert_ne!(wide_arg.syntactic_modifier, narrow_arg.syntactic_modifier);

    let wide_signature = wide_arg.function_signature_type();
    let narrow_signature = narrow_arg.function_signature_type();
    assert_eq!(wide_signature, narrow_signature);
    assert!(wide_signature.syntactic_modifier.is_none());
    assert_eq!(wide_signature.name, "numeric");
    assert_eq!(wide_signature.data_type, KalamDataType::from_sql_name("numeric"));
}

#[test]
fn varchar_modifier_is_discarded_for_the_signature_only() {
    let statement =
        CreateProcedureStatement::parse("CREATE PROCEDURE app.label(name varchar(10))", &app())
            .expect("procedure");
    let argument = &statement.parameters[0].type_ref;
    assert_eq!(argument.syntactic_modifier.as_deref(), Some("10"));

    let signature = argument.function_signature_type();
    assert!(signature.syntactic_modifier.is_none());
    assert_eq!(signature.data_type, Some(KalamDataType::Text));
}

#[test]
fn composite_attributes_do_not_use_the_function_signature_rule() {
    let statement =
        CreateTypeStatement::parse("CREATE TYPE app.payment AS (amount numeric(12,2))", &app())
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
            scale:     2,
        })
    );
    assert_ne!(amount, &amount.function_signature_type());
}

#[test]
fn procedure_returns_is_explicit_and_not_a_function() {
    let statement = CreateProcedureStatement::parse(
        "CREATE PROCEDURE app.charge() RETURNS numeric(12,2)",
        &app(),
    )
    .expect("procedure");
    let returned = statement.return_type.expect("RETURNS stays on the procedure");
    assert_eq!(returned.syntactic_modifier.as_deref(), Some("12,2"));
    let signature = returned.function_signature_type();
    assert!(signature.syntactic_modifier.is_none());
    assert_eq!(signature.data_type, KalamDataType::from_sql_name("numeric"));
}
