use kalamdb_commons::ScheduleId;
use kalamdb_sql::ddl::{AlterScheduleStatement, CreateScheduleStatement, DropScheduleStatement};
pub mod catalog_type;
pub mod comment;
pub mod helpers;
pub mod policy;
pub mod procedure;
mod schedule;
pub mod schema;
pub mod storage;
pub mod table;
pub mod trigger;
pub mod view;

use std::{collections::HashMap, sync::Arc};

use datafusion::arrow::datatypes::Schema as ArrowSchema;
use kalamdb_commons::{
    models::{
        RoutineId, RoutineKind, RoutineNullSafety, RoutineParallelSafety, RoutineVolatility,
        SchemaId, StorageId, TableName, TriggerId,
    },
    RoutineSecurityMode, TableType,
};
use kalamdb_core::{app_context::AppContext, sql::executor::handler_registry::HandlerRegistry};
use kalamdb_handlers_support::register_typed_handler;
use kalamdb_sql::{
    classifier::SqlStatementKind,
    ddl::{
        AlterDomainOperation, AlterDomainStatement, AlterPolicyOperation, AlterPolicyStatement,
        AlterRoutineStatement, AlterTableStatement, AlterTriggerStatement, AlterTypeOperation,
        AlterTypeStatement, CommentOnStatement, CommentOnTarget, CreateDomainStatement,
        CreatePolicyStatement, CreateProcedureStatement, CreateTableStatement,
        CreateTriggerStatement, CreateTypeBody, CreateTypeStatement, CreateViewStatement,
        DescribeTableStatement, DropDomainStatement, DropPolicyStatement, DropProcedureStatement,
        DropTableStatement, DropTriggerStatement, DropTypeStatement, ExecuteGrantee, ExecuteTarget,
        GrantExecuteStatement, PolicyCommand, PolicyTarget, RevokeExecuteStatement,
        ShowTableStatsStatement, ShowTablesStatement, TypeReference,
    },
};

pub fn register_ddl_handlers(registry: &HandlerRegistry, app_context: Arc<AppContext>) {
    register_typed_handler!(
        registry,
        SqlStatementKind::CreateSchema(kalamdb_sql::ddl::CreateSchemaStatement {
            name: SchemaId::new("_placeholder"),
            if_not_exists: false,
        }),
        schema::CreateSchemaHandler::new(app_context.clone()),
        SqlStatementKind::CreateSchema
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::AlterSchema(kalamdb_sql::ddl::AlterSchemaStatement {
            name: SchemaId::new("_placeholder"),
            options: HashMap::new(),
        }),
        schema::AlterSchemaHandler::new(app_context.clone()),
        SqlStatementKind::AlterSchema
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::DropSchema(kalamdb_sql::ddl::DropSchemaStatement {
            name: SchemaId::new("_placeholder"),
            if_exists: false,
            cascade: false,
        }),
        schema::DropSchemaHandler::new(app_context.clone()),
        SqlStatementKind::DropSchema
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::ShowSchemas(kalamdb_sql::ddl::ShowSchemasStatement),
        schema::ShowSchemasHandler::new(app_context.clone()),
        SqlStatementKind::ShowSchemas
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::UseSchema(kalamdb_sql::ddl::UseSchemaStatement {
            namespace: SchemaId::new("_placeholder"),
        }),
        schema::UseSchemaHandler::new(app_context.clone()),
        SqlStatementKind::UseSchema
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::CreateStorage(kalamdb_sql::ddl::CreateStorageStatement {
            storage_id: StorageId::new("_placeholder"),
            storage_type: kalamdb_system::providers::storages::models::StorageType::Filesystem,
            storage_name: String::new(),
            description: None,
            base_directory: String::new(),
            shared_tables_template: String::new(),
            user_tables_template: String::new(),
            credentials: None,
            config_json: None,
        }),
        storage::CreateStorageHandler::new(app_context.clone()),
        SqlStatementKind::CreateStorage
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::AlterStorage(kalamdb_sql::ddl::AlterStorageStatement {
            storage_id: StorageId::new("_placeholder"),
            storage_name: None,
            description: None,
            shared_tables_template: None,
            user_tables_template: None,
            config_json: None,
        }),
        storage::AlterStorageHandler::new(app_context.clone()),
        SqlStatementKind::AlterStorage
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::DropStorage(kalamdb_sql::ddl::DropStorageStatement {
            storage_id: kalamdb_commons::StorageId::from(""),
            if_exists: false,
        }),
        storage::DropStorageHandler::new(app_context.clone()),
        SqlStatementKind::DropStorage
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::ShowStorages(kalamdb_sql::ddl::ShowStoragesStatement),
        storage::ShowStoragesHandler::new(app_context.clone()),
        SqlStatementKind::ShowStorages
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::CheckStorage(kalamdb_sql::ddl::CheckStorageStatement {
            storage_id: kalamdb_commons::StorageId::from(""),
            extended: false,
        }),
        storage::CheckStorageHandler::new(app_context.clone()),
        SqlStatementKind::CheckStorage
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::CreateTable(CreateTableStatement {
            table_name: TableName::new("_placeholder"),
            namespace_id: SchemaId::new("_placeholder"),
            table_type: TableType::Shared,
            schema: Arc::new(ArrowSchema::empty()),
            column_defaults: HashMap::new(),
            column_type_refs: HashMap::new(),
            primary_key_column: None,
            storage_id: None,
            use_user_storage: false,
            flush_policy: None,
            deleted_retention_hours: None,
            ttl_seconds: None,
            compression: None,
            eviction_strategy: None,
            max_stream_size_bytes: None,
            if_not_exists: false,
        }),
        table::CreateTableHandler::new(app_context.clone()),
        SqlStatementKind::CreateTable
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::CreateView(CreateViewStatement {
            namespace_id: SchemaId::new("_placeholder"),
            view_name: TableName::new("_placeholder"),
            or_replace: false,
            if_not_exists: false,
            columns: Vec::new(),
            query_sql: String::new(),
            original_sql: String::new(),
        }),
        view::CreateViewHandler::new(app_context.clone()),
        SqlStatementKind::CreateView
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::AlterTable(AlterTableStatement {
            table_name: TableName::new("_placeholder"),
            namespace_id: SchemaId::new("_placeholder"),
            operation: kalamdb_sql::ddl::ColumnOperation::Drop {
                column_name: "_placeholder".to_string(),
            },
        }),
        table::AlterTableHandler::new(app_context.clone()),
        SqlStatementKind::AlterTable
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::DropTable(DropTableStatement {
            table_name: TableName::new("_placeholder"),
            namespace_id: SchemaId::new("_placeholder"),
            table_type: kalamdb_sql::ddl::TableKind::Shared,
            if_exists: false,
        }),
        table::DropTableHandler::new(app_context.clone()),
        SqlStatementKind::DropTable
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::CreatePolicy(CreatePolicyStatement {
            policy_name: "_placeholder".to_string(),
            table_id: kalamdb_commons::TableId::from_strings("_placeholder", "_placeholder"),
            command: PolicyCommand::All,
            targets: vec![PolicyTarget::Public],
            using_sql: None,
            with_check_sql: None,
            original_sql: String::new(),
        }),
        policy::CreatePolicyHandler::new(app_context.clone()),
        SqlStatementKind::CreatePolicy
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::AlterPolicy(AlterPolicyStatement {
            policy_name: "_placeholder".to_string(),
            table_id: kalamdb_commons::TableId::from_strings("_placeholder", "_placeholder"),
            operation: AlterPolicyOperation::Apply {
                targets: None,
                using_sql: None,
                with_check_sql: None,
            },
        }),
        policy::AlterPolicyHandler::new(app_context.clone()),
        SqlStatementKind::AlterPolicy
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::DropPolicy(DropPolicyStatement {
            policy_name: "_placeholder".to_string(),
            table_id: kalamdb_commons::TableId::from_strings("_placeholder", "_placeholder"),
            if_exists: false,
        }),
        policy::DropPolicyHandler::new(app_context.clone()),
        SqlStatementKind::DropPolicy
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::CreateType(CreateTypeStatement {
            namespace_id: SchemaId::new("_placeholder"),
            name: String::new(),
            if_not_exists: false,
            body: CreateTypeBody::Composite { fields: Vec::new() },
            comment: None,
        }),
        catalog_type::CreateTypeHandler::new(app_context.clone()),
        SqlStatementKind::CreateType
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::AlterType(AlterTypeStatement {
            namespace_id: SchemaId::new("_placeholder"),
            name: String::new(),
            operation: AlterTypeOperation::AddAttribute {
                field: String::new(),
                type_ref: TypeReference {
                    namespace_id: None,
                    name: "text".to_string(),
                    data_type: Some(kalamdb_commons::KalamDataType::Text),
                    syntactic_modifier: None,
                    is_array: false,
                    not_null: false,
                    nonempty: false,
                },
            },
        }),
        catalog_type::AlterTypeHandler::new(app_context.clone()),
        SqlStatementKind::AlterType
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::DropType(DropTypeStatement {
            namespace_id: SchemaId::new("_placeholder"),
            name: String::new(),
            targets: Vec::new(),
            if_exists: false,
            cascade: false,
        }),
        catalog_type::DropTypeHandler::new(app_context.clone()),
        SqlStatementKind::DropType
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::CreateDomain(CreateDomainStatement {
            namespace_id: SchemaId::new("_placeholder"),
            name: String::new(),
            base_type: TypeReference {
                namespace_id: None,
                name: "text".to_string(),
                data_type: Some(kalamdb_commons::KalamDataType::Text),
                syntactic_modifier: None,
                is_array: false,
                not_null: false,
                nonempty: false,
            },
            default_expr: None,
            collation: None,
            not_null: false,
            checks: Vec::new(),
        }),
        catalog_type::CreateDomainHandler::new(app_context.clone()),
        SqlStatementKind::CreateDomain
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::AlterDomain(AlterDomainStatement {
            namespace_id: SchemaId::new("_placeholder"),
            name: String::new(),
            operation: AlterDomainOperation::DropDefault,
        }),
        catalog_type::AlterDomainHandler::new(app_context.clone()),
        SqlStatementKind::AlterDomain
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::DropDomain(DropDomainStatement {
            namespace_id: SchemaId::new("_placeholder"),
            name: String::new(),
            targets: Vec::new(),
            if_exists: false,
            cascade: false,
        }),
        catalog_type::DropDomainHandler::new(app_context.clone()),
        SqlStatementKind::DropDomain
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::CreateProcedure(CreateProcedureStatement {
            routine_id: RoutineId::new("_placeholder.proc"),
            namespace_id: SchemaId::new("_placeholder"),
            name: String::new(),
            kind: RoutineKind::Procedure,
            or_replace: false,
            parameters: Vec::new(),
            return_type: None,
            returns_setof: false,
            returns_table: Vec::new(),
            language: None,
            security: RoutineSecurityMode::Invoker,
            volatility: RoutineVolatility::Volatile,
            null_safety: RoutineNullSafety::CalledOnNullInput,
            parallel_safety: RoutineParallelSafety::Unsafe,
            cost: None,
            rows: None,
            config_changes: Vec::new(),
            body: None,
            comment: None,
        }),
        procedure::CreateProcedureHandler::new(app_context.clone()),
        SqlStatementKind::CreateProcedure
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::AlterRoutine(AlterRoutineStatement {
            routine_id: RoutineId::new("_placeholder.proc"),
            namespace_id: SchemaId::new("_placeholder"),
            name: String::new(),
            expected_kind: RoutineKind::Procedure,
            action: kalamdb_sql::ddl::AlterRoutineAction::RenameTo {
                new_name: String::new(),
            },
        }),
        procedure::AlterRoutineHandler::new(app_context.clone()),
        SqlStatementKind::AlterRoutine
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::DropProcedure(DropProcedureStatement {
            routine_id: RoutineId::new("_placeholder.proc"),
            expected_kind: RoutineKind::Procedure,
            if_exists: false,
            cascade: false,
        }),
        procedure::DropProcedureHandler::new(app_context.clone()),
        SqlStatementKind::DropProcedure
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::CommentOn(CommentOnStatement {
            target: CommentOnTarget::Type {
                namespace_id: SchemaId::new("_placeholder"),
                name: String::new(),
            },
            comment: None,
            if_exists: false,
        }),
        comment::CommentOnHandler::new(app_context.clone()),
        SqlStatementKind::CommentOn
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::CreateSchedule(CreateScheduleStatement {
            schedule_id: ScheduleId::new("_placeholder.schedule"),
            namespace_id: SchemaId::new("_placeholder"),
            name: String::new(),
            routine_id: RoutineId::new("_placeholder.proc"),
            cron: None,
            interval_ms: Some(1000),
            timezone: "UTC".into(),
            principal: String::new(),
        }),
        schedule::CreateScheduleHandler::new(app_context.clone()),
        SqlStatementKind::CreateSchedule
    );
    register_typed_handler!(
        registry,
        SqlStatementKind::AlterSchedule(AlterScheduleStatement {
            schedule_id: ScheduleId::new("_placeholder.schedule"),
            enabled: true,
        }),
        schedule::AlterScheduleHandler::new(app_context.clone()),
        SqlStatementKind::AlterSchedule
    );
    register_typed_handler!(
        registry,
        SqlStatementKind::DropSchedule(DropScheduleStatement {
            schedule_id: ScheduleId::new("_placeholder.schedule"),
            if_exists: false,
        }),
        schedule::DropScheduleHandler::new(app_context.clone()),
        SqlStatementKind::DropSchedule
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::CreateTrigger(CreateTriggerStatement {
            trigger_id: TriggerId::new("_placeholder.trig"),
            namespace_id: SchemaId::new("_placeholder"),
            name: String::new(),
            topic_id: kalamdb_commons::models::TopicId::new("_placeholder.topic"),
            routine_id: RoutineId::new("_placeholder.proc"),
            principal: String::new(),
            start_from: "latest".to_string(),
            retries: 5,
            retry_backoff_ms: 1000,
            concurrency: 1,
        }),
        trigger::CreateTriggerHandler::new(app_context.clone()),
        SqlStatementKind::CreateTrigger
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::DropTrigger(DropTriggerStatement {
            trigger_id: TriggerId::new("_placeholder.trig"),
            if_exists: false,
        }),
        trigger::DropTriggerHandler::new(app_context.clone()),
        SqlStatementKind::DropTrigger
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::AlterTrigger(AlterTriggerStatement {
            trigger_id: TriggerId::new("_placeholder.trig"),
            enabled: true,
        }),
        trigger::AlterTriggerHandler::new(app_context.clone()),
        SqlStatementKind::AlterTrigger
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::GrantExecute(GrantExecuteStatement {
            target: ExecuteTarget::Routine {
                routine_id: RoutineId::new("_placeholder.proc"),
                expected_kind: RoutineKind::Procedure,
            },
            routine_id: RoutineId::new("_placeholder.proc"),
            grantee: ExecuteGrantee::Public,
        }),
        procedure::GrantExecuteHandler::new(app_context.clone()),
        SqlStatementKind::GrantExecute
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::RevokeExecute(RevokeExecuteStatement {
            target: ExecuteTarget::Routine {
                routine_id: RoutineId::new("_placeholder.proc"),
                expected_kind: RoutineKind::Procedure,
            },
            routine_id: RoutineId::new("_placeholder.proc"),
            grantee: ExecuteGrantee::Public,
        }),
        procedure::RevokeExecuteHandler::new(app_context.clone()),
        SqlStatementKind::RevokeExecute
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::ShowTables(ShowTablesStatement { namespace_id: None }),
        table::ShowTablesHandler::new(app_context.clone()),
        SqlStatementKind::ShowTables
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::DescribeTable(DescribeTableStatement {
            namespace_id: None,
            table_name: TableName::new("_placeholder"),
            show_history: false,
        }),
        table::DescribeTableHandler::new(app_context.clone()),
        SqlStatementKind::DescribeTable
    );

    register_typed_handler!(
        registry,
        SqlStatementKind::ShowStats(ShowTableStatsStatement {
            namespace_id: None,
            table_name: TableName::new("_placeholder"),
        }),
        table::ShowStatsHandler::new(app_context),
        SqlStatementKind::ShowStats
    );
}
