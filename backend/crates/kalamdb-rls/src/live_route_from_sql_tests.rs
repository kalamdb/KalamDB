use std::{collections::HashMap, sync::Arc};

use kalamdb_commons::{
    datatypes::KalamDataType,
    schemas::{ColumnDefinition, TableDefinition, TableOptions, TableType},
    NamespaceId, PolicyCommand, PolicyId, PolicyTarget, Role, TableId, TableName, TablePolicy,
    UserId,
};

use crate::{
    AuthorizationSet, BoundAuthorization, LiveAuthorizationRoute, PolicyCompiler,
    PolicyTableResolver,
};

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

fn conversation_compiler() -> (Arc<TableDefinition>, PolicyCompiler<TestResolver>) {
    let messages = shared_table(
        "messages",
        &[
            ("id", true),
            ("conversation_id", false),
            ("owner_id", false),
            ("visibility", false),
            ("body", false),
        ],
    );
    let members = shared_table(
        "conversation_members",
        &[
            ("user_id", true),
            ("conversation_id", true),
            ("role", false),
        ],
    );
    let mut resolver = TestResolver {
        tables: HashMap::new(),
    };
    resolver.tables.insert(table_id(&messages), messages.clone());
    resolver.tables.insert(table_id(&members), members);
    (messages, PolicyCompiler::new(resolver))
}

fn table_id(table: &TableDefinition) -> TableId {
    TableId::new(table.namespace_id.clone(), table.table_name.clone())
}

fn utf8(value: &str) -> datafusion::scalar::ScalarValue {
    datafusion::scalar::ScalarValue::Utf8(Some(value.to_string()))
}

fn live_route(
    table: &Arc<TableDefinition>,
    compiler: &PolicyCompiler<TestResolver>,
    using_sql: &str,
    principal: &str,
    membership_keys: Vec<&str>,
) -> LiveAuthorizationRoute {
    let program = compiler
        .compile(table, using_sql)
        .unwrap_or_else(|error| panic!("policy SQL should compile ({using_sql}): {error}"));
    let table_id = table_id(table);
    let policy_id = PolicyId::new(table_id.clone(), "select_policy").unwrap();
    let policy = TablePolicy::new(
        policy_id.clone(),
        table_id,
        "select_policy",
        PolicyCommand::Select,
        vec![PolicyTarget::Role(Role::User)],
        Some(using_sql.to_string()),
        None,
        Some(program.clone()),
        None,
        1,
        1,
    );
    let policies = crate::BoundTablePolicies::bind(
        &[policy],
        UserId::new(principal),
        Role::User,
        PolicyCommand::Select,
    );
    let sets = match &program {
        kalamdb_commons::PolicyProgram::AuthorizationRelation(relation) => {
            let keys = membership_keys.into_iter().map(|key| vec![utf8(key)]).collect();
            HashMap::from([(
                policy_id,
                Arc::new(AuthorizationSet::from_keys(relation.clone(), keys)),
            )])
        },
        _ => HashMap::new(),
    };
    BoundAuthorization::new(Arc::clone(table), policies, sets, Vec::new())
        .live_route()
        .expect("bound live route")
}

const CONVERSATION_IN: &str = "conversation_id IN (SELECT conversation_id FROM \
                               chat.conversation_members WHERE user_id = CURRENT_USER)";
const CONVERSATION_EXISTS: &str = "EXISTS (SELECT 1 FROM chat.conversation_members m WHERE \
                                   m.conversation_id = messages.conversation_id AND m.user_id = \
                                   CURRENT_USER)";
const CONVERSATION_IN_WITH_ROLE: &str = "conversation_id IN (SELECT conversation_id FROM \
                                         chat.conversation_members WHERE user_id = CURRENT_USER \
                                         AND role = 'member')";

#[test]
fn conversation_membership_in_subquery_indexes_conversation_id_keys() {
    let (messages, compiler) = conversation_compiler();

    assert_eq!(
        live_route(&messages, &compiler, CONVERSATION_IN, "alice", vec!["conv-123", "conv-456"]),
        LiveAuthorizationRoute::Keyed(std::collections::HashSet::from([
            (2, utf8("conv-123")),
            (2, utf8("conv-456")),
        ]))
    );
}

#[test]
fn conversation_membership_exists_indexes_the_same_conversation_id_keys() {
    let (messages, compiler) = conversation_compiler();

    assert_eq!(
        live_route(
            &messages,
            &compiler,
            CONVERSATION_EXISTS,
            "alice",
            vec!["conv-123", "conv-456"]
        ),
        live_route(&messages, &compiler, CONVERSATION_IN, "alice", vec!["conv-123", "conv-456"])
    );
}

#[test]
fn conversation_membership_with_static_role_predicate_still_indexes_conversation_id() {
    let (messages, compiler) = conversation_compiler();

    assert_eq!(
        live_route(&messages, &compiler, CONVERSATION_IN_WITH_ROLE, "alice", vec!["conv-123"]),
        LiveAuthorizationRoute::Keyed(std::collections::HashSet::from([(2, utf8("conv-123"))]))
    );
}

#[test]
fn owner_equality_query_indexes_current_user_on_owner_id() {
    let (messages, compiler) = conversation_compiler();

    assert_eq!(
        live_route(&messages, &compiler, "owner_id = CURRENT_USER", "alice", Vec::new()),
        LiveAuthorizationRoute::Keyed(std::collections::HashSet::from([(3, utf8("alice"))]))
    );
}

#[test]
fn public_visibility_literal_indexes_that_scalar() {
    let (messages, compiler) = conversation_compiler();

    assert_eq!(
        live_route(&messages, &compiler, "visibility = 'public'", "alice", Vec::new()),
        LiveAuthorizationRoute::Keyed(std::collections::HashSet::from([(4, utf8("public"))]))
    );
}

#[test]
fn true_query_is_broadcast_and_false_query_is_deny() {
    let (messages, compiler) = conversation_compiler();

    assert_eq!(
        live_route(&messages, &compiler, "true", "alice", Vec::new()),
        LiveAuthorizationRoute::Broadcast
    );
    assert_eq!(
        live_route(&messages, &compiler, "false", "alice", Vec::new()),
        LiveAuthorizationRoute::Deny
    );
}
