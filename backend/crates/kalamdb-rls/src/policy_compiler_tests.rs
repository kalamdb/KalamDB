use std::{collections::HashMap, sync::Arc};

use kalamdb_commons::{
    datatypes::KalamDataType,
    schemas::{ColumnDefinition, TableDefinition, TableOptions, TableType},
    AuthorizationRelation, BoundExprShape, InvalidationStrategy, NamespaceId, PolicyProgram,
    PrincipalExpr, TableId, TableName,
};

use super::{PolicyCompiler, PolicyTableResolver};

#[derive(Default)]
struct TestResolver {
    tables: HashMap<TableId, Arc<TableDefinition>>,
}

impl PolicyTableResolver for TestResolver {
    fn resolve_table(&self, table_id: &TableId) -> Result<Arc<TableDefinition>, String> {
        self.tables
            .get(table_id)
            .cloned()
            .ok_or_else(|| format!("table {table_id} does not exist"))
    }
}

fn shared_table(name: &str, columns: &[(&str, bool)]) -> Arc<TableDefinition> {
    let columns = columns
        .iter()
        .enumerate()
        .map(|(index, (name, primary_key))| {
            let column_id = u64::try_from(index + 1).expect("small test schema");
            let ordinal = u32::try_from(index + 1).expect("small test schema");
            if *primary_key {
                ColumnDefinition::primary_key(column_id, *name, ordinal, KalamDataType::Text)
            } else {
                ColumnDefinition::simple(column_id, *name, ordinal, KalamDataType::Text)
            }
        })
        .collect();
    Arc::new(
        TableDefinition::new(
            NamespaceId::new("chat"),
            TableName::new(name),
            TableType::Shared,
            columns,
            TableOptions::shared(),
            None,
        )
        .expect("valid test table"),
    )
}

fn compiler() -> (Arc<TableDefinition>, PolicyCompiler<TestResolver>) {
    let messages = shared_table("messages", &[("id", true), ("group_id", false), ("body", false)]);
    let members =
        shared_table("group_members", &[("user_id", true), ("group_id", false), ("active", false)]);
    let mut resolver = TestResolver::default();
    resolver.tables.insert(table_id(&messages), messages.clone());
    resolver.tables.insert(table_id(&members), members);
    (messages, PolicyCompiler::new(resolver))
}

fn table_id(table: &TableDefinition) -> TableId {
    TableId::new(table.namespace_id.clone(), table.table_name.clone())
}

#[test]
fn compiles_row_local_current_user_without_binding_identity() {
    let (messages, compiler) = compiler();

    let program = compiler
        .compile(&messages, "group_id = CURRENT_USER")
        .expect("row-local policy should compile");

    assert_eq!(
        program,
        PolicyProgram::RowLocal {
            expr: BoundExprShape::ColumnEqualsPrincipal {
                column_id: 2,
                principal: PrincipalExpr::CurrentUser,
            },
        }
    );
}

#[test]
fn in_subquery_normalizes_to_authorization_relation() {
    let (messages, compiler) = compiler();

    let program = compiler
        .compile(
            &messages,
            "group_id IN (SELECT group_id FROM chat.group_members WHERE user_id = CURRENT_USER)",
        )
        .expect("membership policy should compile");

    assert_eq!(
        program,
        PolicyProgram::AuthorizationRelation(AuthorizationRelation {
            protected_table:   table_id(&messages),
            protected_keys:    vec![2],
            relation_table:    TableId::from_strings("chat", "group_members"),
            relation_keys:     vec![2],
            principal_column:  1,
            principal:         PrincipalExpr::CurrentUser,
            static_predicates: Vec::new(),
            dependencies:      vec![TableId::from_strings("chat", "group_members")],
            invalidation:      InvalidationStrategy::TargetedPrincipal,
        })
    );
}

#[test]
fn correlated_exists_normalizes_to_same_authorization_relation() {
    let (messages, compiler) = compiler();
    let in_program = compiler
        .compile(
            &messages,
            "group_id IN (SELECT group_id FROM chat.group_members WHERE user_id = CURRENT_USER)",
        )
        .expect("IN membership should compile");
    let exists_program = compiler
        .compile(
            &messages,
            "EXISTS (SELECT 1 FROM chat.group_members gm WHERE gm.group_id = messages.group_id \
             AND gm.user_id = CURRENT_USER)",
        )
        .expect("EXISTS membership should compile");

    assert_eq!(exists_program, in_program);
}

#[test]
fn rejects_unsafe_or_unsupported_policy_shapes() {
    let (messages, compiler) = compiler();

    for expression in [
        "EXISTS (SELECT 1 FROM chat.messages WHERE id = CURRENT_USER)",
        "group_id IN (SELECT max(group_id) FROM chat.group_members WHERE user_id = CURRENT_USER)",
        "random() > 0.5",
        "group_id IN (SELECT group_id FROM chat.group_members)",
    ] {
        assert!(
            compiler.compile(&messages, expression).is_err(),
            "unsafe policy unexpectedly compiled: {expression}"
        );
    }
}

#[test]
fn membership_without_covering_primary_key_warns() {
    let (messages, compiler) = compiler();
    let program = compiler
        .compile(
            &messages,
            "group_id IN (SELECT group_id FROM chat.group_members WHERE user_id = CURRENT_USER)",
        )
        .unwrap();
    let warning = compiler.covering_membership_index_warning(&program).unwrap();
    assert!(warning.contains("group_members"));
    assert!(warning.contains("covering primary key"));
}

#[test]
fn membership_with_covering_primary_key_does_not_warn() {
    let messages = shared_table("messages", &[("id", true), ("group_id", false), ("body", false)]);
    let members =
        shared_table("group_members", &[("user_id", true), ("group_id", true), ("active", false)]);
    let mut resolver = TestResolver::default();
    resolver.tables.insert(table_id(&messages), messages.clone());
    resolver.tables.insert(table_id(&members), members);
    let compiler = PolicyCompiler::new(resolver);
    let program = compiler
        .compile(
            &messages,
            "group_id IN (SELECT group_id FROM chat.group_members WHERE user_id = CURRENT_USER)",
        )
        .unwrap();
    assert_eq!(compiler.covering_membership_index_warning(&program), None);
}
