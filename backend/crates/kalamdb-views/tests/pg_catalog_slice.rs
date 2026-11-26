use std::sync::Arc;

use datafusion::{
    arrow::array::{BooleanArray, Float64Array, Int64Array, StringArray},
    catalog::SchemaProvider,
    execution::context::SessionContext,
};
use kalamdb_commons::{
    models::{
        datatypes::KalamDataType, AuthType, CatalogTypeKind, LogicalTypeRef, Role, RoutineId,
        RoutineKind, RoutineNullSafety, RoutineParallelSafety, RoutineParameterId,
        RoutineParameterMode, RoutineSecurityMode, RoutineVolatility, SchemaId, StorageId, TableId,
        TableName, TypeFieldId, TypeId, UserId,
    },
    schemas::{
        ColumnDefault, ColumnDefinition, ForeignKeyAction, ForeignKeyDefinition,
        ScalarIndexDefinition, TableDefinition, TableOptions, TableType,
    },
    ColumnId,
};
use kalamdb_store::{test_utils::InMemoryBackend, StorageBackend};
use kalamdb_system::{
    providers::storages::models::StorageMode, CatalogRoutine, CatalogRoutineParameter, CatalogType,
    CatalogTypeField, Namespace, SystemTablesRegistry, User,
};
use kalamdb_views::pg_catalog::PgCatalogSchemaProvider;

#[tokio::test]
async fn pg_catalog_slice_projects_schemas_tables_enums_routines_defaults_and_roles() {
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
    let registry = Arc::new(SystemTablesRegistry::new(Arc::clone(&backend)));

    // 1. Seed a user/schema
    let mut opts = serde_json::Map::new();
    opts.insert("comment".to_string(), serde_json::Value::String("Billing schema".to_string()));
    registry
        .namespaces()
        .create_namespace(Namespace {
            namespace_id: SchemaId::new("billing"),
            name: "billing".to_string(),
            created_at: 1_000,
            options: Some(serde_json::Value::Object(opts)),
            table_count: 1,
        })
        .expect("create schema");

    registry
        .users()
        .create_user(User {
            user_id: UserId::new("alice"),
            password_hash: "secret-bcrypt-hash".to_string(),
            role: Role::User,
            name: None,
            email: Some("alice@example.com".to_string()),
            auth_type: AuthType::Password,
            auth_data: None,
            storage_mode: StorageMode::Table,
            storage_id: Some(StorageId::local()),
            failed_login_attempts: 0,
            locked_until: None,
            last_login_at: None,
            created_at: 1_000,
            updated_at: 1_000,
            last_seen: None,
            deleted_at: None,
            invite_expires_at: None,
            invited_by: None,
        })
        .expect("create user");

    // 2. Seed a table with a default expression and comment
    let mut id_col = ColumnDefinition::new(
        1,
        "id",
        1,
        KalamDataType::BigInt,
        false,
        true,
        false,
        ColumnDefault::None,
        Some("Primary invoice id".to_string()),
    );
    id_col.is_primary_key = true;
    let status_col = ColumnDefinition::new(
        2,
        "status",
        2,
        KalamDataType::Text,
        false,
        false,
        false,
        ColumnDefault::Literal(serde_json::Value::String("draft".to_string())),
        None,
    );
    let mut table_def = TableDefinition::new(
        SchemaId::new("billing"),
        TableName::new("invoices"),
        TableType::Shared,
        vec![id_col, status_col],
        TableOptions::shared(),
        Some("Customer invoices".to_string()),
    )
    .expect("table definition");
    table_def.schema_version = 1;
    let table_id = TableId::new(SchemaId::new("billing"), TableName::new("invoices"));
    registry.tables().create_table(&table_id, &table_def).expect("register table");

    // 3. Seed an enum type and its labels
    let enum_id = TypeId::from_parts(Some(&SchemaId::new("billing")), "invoice_state");
    let mut enum_type = CatalogType::named(
        enum_id.clone(),
        SchemaId::new("billing"),
        "invoice_state",
        CatalogTypeKind::Enum,
    );
    enum_type.comment = Some("Invoice lifecycle state".to_string());
    registry.catalog_stores().upsert_type(enum_type).expect("put enum type");
    for (pos, label) in [(1i32, "draft"), (2i32, "paid")] {
        registry
            .catalog_stores()
            .upsert_type_field(CatalogTypeField {
                type_field_id: TypeFieldId::from_slot(&enum_id, pos).expect("field id"),
                type_id: enum_id.clone(),
                name: label.to_string(),
                ordinal: pos,
                field_type_id: None,
                type_name: "text".to_string(),
                is_array: false,
                not_null: true,
                nonempty: false,
                data_type: None,
                slot: pos,
                dropped: false,
                element_nullable: true,
            })
            .expect("put enum label");
    }

    // 4. Seed a routine and parameter
    let routine_id = RoutineId::from_parts(Some(&SchemaId::new("billing")), "compute_total(int8)");
    registry
        .catalog_stores()
        .upsert_routine(CatalogRoutine {
            routine_id: routine_id.clone(),
            namespace_id: SchemaId::new("billing"),
            name: "compute_total".to_string(),
            owner: UserId::new("alice"),
            security: RoutineSecurityMode::Invoker,
            language: Some("sql".to_string()),
            body: Some("SELECT $1 * 2".to_string()),
            return_type_id: None,
            return_type_name: Some("bigint".to_string()),
            return_is_array: false,
            return_not_null: false,
            comment: Some("Doubles the amount".to_string()),
            return_data_type: Some(KalamDataType::BigInt),
            inline_source_hash: None,
            inline_artifact_id: None,
            kind: RoutineKind::Function,
            volatility: RoutineVolatility::Immutable,
            null_safety: RoutineNullSafety::ReturnsNullOnNullInput,
            parallel_safety: RoutineParallelSafety::Safe,
            cost: None,
            rows: None,
            config_params: None,
        })
        .expect("put routine");
    registry
        .catalog_stores()
        .upsert_parameter(CatalogRoutineParameter {
            parameter_id: RoutineParameterId::new(&routine_id, 1).expect("param id"),
            routine_id,
            name: "amount".to_string(),
            ordinal: 1,
            type_id: None,
            type_name: "bigint".to_string(),
            is_array: false,
            not_null: true,
            nonempty: false,
            data_type: Some(KalamDataType::BigInt),
            mode: RoutineParameterMode::In,
            default_expr: None,
        })
        .expect("put parameter");

    let pg_catalog =
        Arc::new(PgCatalogSchemaProvider::new(Arc::clone(&registry), Arc::new(Vec::new)));
    let ctx = SessionContext::new();
    ctx.catalog("datafusion")
        .expect("default catalog")
        .register_schema("pg_catalog", pg_catalog as Arc<dyn SchemaProvider>)
        .expect("register pg_catalog");

    // Verify pg_namespace
    let ns_batches = ctx
        .sql("SELECT nspname FROM pg_catalog.pg_namespace WHERE nspname = 'billing'")
        .await
        .expect("sql")
        .collect()
        .await
        .expect("collect");
    assert_eq!(ns_batches[0].num_rows(), 1);

    // Verify pg_class
    let cls_batches = ctx
        .sql("SELECT relname, relkind FROM pg_catalog.pg_class WHERE relname = 'invoices'")
        .await
        .expect("sql")
        .collect()
        .await
        .expect("collect");
    assert_eq!(cls_batches[0].num_rows(), 1);
    let relkinds = cls_batches[0].column(1).as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(relkinds.value(0), "r");

    // Verify pg_enum (no longer an empty stub!)
    let enum_batches = ctx
        .sql("SELECT enumlabel, enumsortorder FROM pg_catalog.pg_enum ORDER BY enumsortorder")
        .await
        .expect("sql")
        .collect()
        .await
        .expect("collect");
    assert_eq!(enum_batches[0].num_rows(), 2);
    let labels = enum_batches[0].column(0).as_any().downcast_ref::<StringArray>().unwrap();
    let orders = enum_batches[0].column(1).as_any().downcast_ref::<Float64Array>().unwrap();
    assert_eq!(labels.value(0), "draft");
    assert_eq!(orders.value(0), 1.0);
    assert_eq!(labels.value(1), "paid");
    assert_eq!(orders.value(1), 2.0);

    // Verify pg_language (no longer an empty stub!)
    let lang_batches = ctx
        .sql(
            "SELECT lanname, lanpltrusted FROM pg_catalog.pg_language WHERE lanname IN ('sql', \
             'plpgsql', 'javascript') ORDER BY lanname",
        )
        .await
        .expect("sql")
        .collect()
        .await
        .expect("collect");
    assert_eq!(lang_batches[0].num_rows(), 3);

    // Verify pg_attrdef (no longer an empty stub!)
    let attrdef_batches = ctx
        .sql("SELECT adnum, adbin FROM pg_catalog.pg_attrdef")
        .await
        .expect("sql")
        .collect()
        .await
        .expect("collect");
    assert_eq!(attrdef_batches[0].num_rows(), 1);
    let adnums = attrdef_batches[0].column(0).as_any().downcast_ref::<Int64Array>().unwrap();
    assert_eq!(adnums.value(0), 2);

    // Verify pg_description (no longer an empty stub!)
    let desc_batches = ctx
        .sql("SELECT description FROM pg_catalog.pg_description ORDER BY description")
        .await
        .expect("sql")
        .collect()
        .await
        .expect("collect");
    assert_eq!(desc_batches[0].num_rows(), 5);

    // Verify pg_roles & pg_authid (and confirm pg_authid has no rolpassword column)
    let authid_schema = ctx
        .sql("SELECT * FROM pg_catalog.pg_authid")
        .await
        .expect("sql")
        .schema()
        .inner()
        .clone();
    assert!(
        authid_schema.column_with_name("rolpassword").is_none(),
        "pg_authid must never expose credentials"
    );
    let roles_batches = ctx
        .sql("SELECT rolname, rolcanlogin FROM pg_catalog.pg_roles WHERE rolname = 'alice'")
        .await
        .expect("sql")
        .collect()
        .await
        .expect("collect");
    assert_eq!(roles_batches[0].num_rows(), 1);
    let can_login = roles_batches[0].column(1).as_any().downcast_ref::<BooleanArray>().unwrap();
    assert!(can_login.value(0));
}

#[tokio::test]
async fn directus_catalog_reports_indexes_foreign_keys_and_column_types() {
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
    let registry = Arc::new(SystemTablesRegistry::new(Arc::clone(&backend)));
    registry
        .namespaces()
        .create_namespace(Namespace {
            namespace_id: SchemaId::new("directus"),
            name: "directus".to_string(),
            created_at: 1_000,
            options: None,
            table_count: 2,
        })
        .expect("create schema");

    let mut authors = TableDefinition::new(
        SchemaId::new("directus"),
        TableName::new("authors"),
        TableType::Shared,
        vec![ColumnDefinition::primary_key(
            1,
            "id",
            1,
            KalamDataType::Int,
        )],
        TableOptions::shared(),
        None,
    )
    .expect("authors");
    authors.schema_version = 1;
    registry
        .tables()
        .create_table(&TableId::new(SchemaId::new("directus"), TableName::new("authors")), &authors)
        .expect("register authors");

    let mut title = ColumnDefinition::simple(2, "title", 2, KalamDataType::Text);
    title.character_maximum_length = Some(255);
    let mut shape = ColumnDefinition::simple(3, "shape", 3, KalamDataType::Text);
    shape.logical_type = LogicalTypeRef::geometry();
    let mut articles = TableDefinition::new(
        SchemaId::new("directus"),
        TableName::new("articles"),
        TableType::Shared,
        vec![
            ColumnDefinition::primary_key(1, "id", 1, KalamDataType::Int),
            title,
            shape,
            ColumnDefinition::simple(4, "author", 4, KalamDataType::Int),
        ],
        TableOptions::shared(),
        None,
    )
    .expect("articles");
    articles.schema_version = 1;
    articles.scalar_indexes.push(ScalarIndexDefinition::new(
        "articles_title_index",
        vec![ColumnId::new(2)],
        false,
    ));
    articles.scalar_indexes.push(ScalarIndexDefinition::new(
        "articles_missing_index",
        vec![ColumnId::new(99)],
        false,
    ));
    articles.foreign_keys.push(ForeignKeyDefinition {
        name: "articles_author_foreign".to_string(),
        columns: vec!["author".to_string()],
        referenced_namespace: SchemaId::new("directus"),
        referenced_table: TableName::new("authors"),
        referenced_columns: vec!["id".to_string()],
        on_update: ForeignKeyAction::NoAction,
        on_delete: ForeignKeyAction::Cascade,
    });
    articles.foreign_keys.push(ForeignKeyDefinition {
        name: "articles_editor_foreign".to_string(),
        columns: vec!["author".to_string()],
        referenced_namespace: SchemaId::new("directus"),
        referenced_table: TableName::new("editors"),
        referenced_columns: vec!["id".to_string()],
        on_update: ForeignKeyAction::NoAction,
        on_delete: ForeignKeyAction::SetNull,
    });
    registry
        .tables()
        .create_table(
            &TableId::new(SchemaId::new("directus"), TableName::new("articles")),
            &articles,
        )
        .expect("register articles");

    let pg_catalog =
        Arc::new(PgCatalogSchemaProvider::new(Arc::clone(&registry), Arc::new(Vec::new)));
    let ctx = SessionContext::new();
    ctx.catalog("datafusion")
        .expect("default catalog")
        .register_schema("pg_catalog", pg_catalog as Arc<dyn SchemaProvider>)
        .expect("register pg_catalog");

    let foreign_keys = query_rows(
        &ctx,
        "SELECT conname, confupdtype, confdeltype FROM pg_catalog.pg_constraint WHERE contype = \
         'f' ORDER BY conname",
    )
    .await;
    assert_eq!(
        foreign_keys,
        vec![vec![
            "articles_author_foreign".to_string(),
            "a".to_string(),
            "c".to_string()
        ]]
    );

    let index_class = query_rows(
        &ctx,
        "SELECT relkind FROM pg_catalog.pg_class WHERE relname = 'articles_title_index'",
    )
    .await;
    assert_eq!(index_class, vec![vec!["i".to_string()]]);
    let hidden = query_rows(
        &ctx,
        "SELECT relname FROM pg_catalog.pg_class WHERE relname = 'articles_missing_index'",
    )
    .await;
    assert!(hidden.is_empty());

    let index_flags = query_rows(
        &ctx,
        "SELECT indisunique, indisprimary FROM pg_catalog.pg_index WHERE indexrelid = (SELECT oid \
         FROM pg_catalog.pg_class WHERE relname = 'articles_title_index')",
    )
    .await;
    assert_eq!(index_flags, vec![vec!["false".to_string(), "false".to_string()]]);

    let columns = query_rows(
        &ctx,
        "SELECT att.attname, att.atttypmod, typ.typname FROM pg_catalog.pg_attribute att JOIN \
         pg_catalog.pg_class rel ON att.attrelid = rel.oid JOIN pg_catalog.pg_type typ ON typ.oid \
         = att.atttypid WHERE rel.relname = 'articles' AND att.attname IN ('title', 'shape', \
         '_version') ORDER BY att.attname",
    )
    .await;
    assert_eq!(
        columns,
        vec![
            vec![
                "shape".to_string(),
                "-1".to_string(),
                "geometry".to_string()
            ],
            vec![
                "title".to_string(),
                "259".to_string(),
                "varchar".to_string()
            ],
        ]
    );
}

async fn query_rows(ctx: &SessionContext, sql: &str) -> Vec<Vec<String>> {
    let batches = ctx.sql(sql).await.expect(sql).collect().await.expect(sql);
    let mut rows = Vec::new();
    for batch in batches {
        for row in 0..batch.num_rows() {
            let mut values = Vec::with_capacity(batch.num_columns());
            for column in 0..batch.num_columns() {
                let array = batch.column(column);
                if let Some(strings) = array.as_any().downcast_ref::<StringArray>() {
                    values.push(strings.value(row).to_string());
                } else if let Some(ints) = array.as_any().downcast_ref::<Int64Array>() {
                    values.push(ints.value(row).to_string());
                } else if let Some(flags) = array.as_any().downcast_ref::<BooleanArray>() {
                    values.push(flags.value(row).to_string());
                } else {
                    panic!("unsupported catalog column {}", array.data_type());
                }
            }
            rows.push(values);
        }
    }
    rows
}

#[tokio::test]
async fn pg_catalog_and_columns_extension_follow_table_macro_order() {
    let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
    let registry = Arc::new(SystemTablesRegistry::new(backend));
    let pg_catalog = PgCatalogSchemaProvider::new(Arc::clone(&registry), Arc::new(Vec::new));
    for name in ["pg_class", "pg_attribute", "pg_namespace"] {
        let provider = pg_catalog.table(name).await.expect("lookup").expect(name);
        let schema = provider.schema();
        let actual: Vec<_> = schema.fields().iter().map(|field| field.name().to_string()).collect();
        let expected: Vec<_> = kalamdb_views::pg_catalog::relation_column_order(name)
            .expect(name)
            .iter()
            .map(|field| (*field).to_string())
            .collect();
        assert_eq!(actual, expected, "{name}");
    }

    let ctx = SessionContext::new();
    let info = kalamdb_views::information_schema::KalamInformationSchemaProvider::new(
        Arc::clone(ctx.state().catalog_list()),
        registry,
    );
    let columns = SchemaProvider::table(&info, "columns")
        .await
        .expect("columns")
        .expect("columns");
    let schema = columns.schema();
    let names: Vec<_> = schema.fields().iter().map(|field| field.name().to_string()).collect();
    let extension: Vec<_> = kalamdb_views::information_schema::columns_extension_order()
        .iter()
        .map(|field| (*field).to_string())
        .collect();
    assert!(
        names.ends_with(extension.as_slice()),
        "information_schema.columns extension order {names:?} does not end with {extension:?}"
    );
}
