//! CREATE TABLE helper functions
//!
//! Provides unified logic for creating all table types (USER/SHARED/STREAM)

use std::sync::Arc;

use kalamdb_commons::{
    models::{StorageId, TableId, UserId},
    schemas::{ColumnDefault, TableType},
    Role,
};
use kalamdb_core::{
    app_context::AppContext, error::KalamDbError, error_extensions::KalamDbResultExt,
};
use kalamdb_sql::ddl::{CreateTableStatement, TypeReference};
use kalamdb_system::providers::storages::models::StorageType;

/// Build a TableDefinition by validating inputs and constructing the definition
/// WITHOUT persisting or registering providers.
///
/// This is used in cluster mode where the Raft applier handles the actual
/// persistence and provider registration on ALL nodes (including the leader).
///
/// # Arguments
/// * `app_context` - Application context
/// * `stmt` - Parsed CREATE TABLE statement
/// * `user_id` - User ID from execution context
/// * `user_role` - User role from execution context
///
/// # Returns
/// Ok with TableDefinition, or error
pub fn build_table_definition(
    app_context: Arc<AppContext>,
    stmt: &CreateTableStatement,
    user_id: &UserId,
    user_role: Role,
) -> Result<kalamdb_commons::models::schemas::TableDefinition, KalamDbError> {
    use kalamdb_commons::{
        datatypes::{FromArrowType, KalamDataType},
        models::schemas::{ColumnDefinition, TableDefinition, TableOptions},
        schemas::ColumnDefault,
    };

    let table_id_str = format!("{}.{}", stmt.namespace_id.as_str(), stmt.table_name.as_str());

    log::debug!(
        "🔨 BUILD TABLE DEFINITION: {} (type: {:?}, user: {}, role: {:?})",
        table_id_str,
        stmt.table_type,
        user_id.as_str(),
        user_role
    );

    // Block CREATE on system namespaces
    super::guards::block_system_namespace_modification(
        &stmt.namespace_id,
        "CREATE",
        "TABLE",
        Some(stmt.table_name.as_str()),
    )?;

    // RBAC check
    if !kalamdb_session::can_create_table(user_role, stmt.table_type) {
        log::error!(
            "❌ CREATE TABLE {:?} {}: Insufficient privileges (user: {}, role: {:?})",
            stmt.table_type,
            table_id_str,
            user_id.as_str(),
            user_role
        );
        return Err(KalamDbError::Unauthorized(format!(
            "Insufficient privileges to create {:?} tables",
            stmt.table_type
        )));
    }

    // Validate table name
    kalamdb_sql::validation::validate_table_name(stmt.table_name.as_str())
        .map_err(|e| KalamDbError::InvalidOperation(e.to_string()))?;

    // Validate namespace exists
    let namespaces_provider = app_context.system_tables().namespaces();
    let namespace_id = stmt.namespace_id.clone();
    if namespaces_provider.get_namespace(&namespace_id)?.is_none() {
        log::error!(
            "❌ CREATE TABLE failed: Namespace '{}' does not exist",
            stmt.namespace_id.as_str()
        );
        return Err(KalamDbError::InvalidOperation(format!(
            "Namespace '{}' does not exist. Create it first with CREATE NAMESPACE {}",
            stmt.namespace_id.as_str(),
            stmt.namespace_id.as_str()
        )));
    }

    // Type-specific validation
    match stmt.table_type {
        TableType::User | TableType::Shared => {
            // PRIMARY KEY validation
            if stmt.primary_key_column.is_none() {
                log::error!(
                    "❌ CREATE TABLE {:?} {}: PRIMARY KEY is required",
                    stmt.table_type,
                    table_id_str
                );
                return Err(KalamDbError::InvalidOperation(format!(
                    "{:?} tables require a PRIMARY KEY column",
                    stmt.table_type
                )));
            }
        },
        TableType::Stream => {
            // TTL validation
            if stmt.ttl_seconds.is_none() {
                log::error!("❌ CREATE TABLE STREAM {}: TTL clause is required", table_id_str);
                return Err(KalamDbError::InvalidOperation(
                    "STREAM tables require TTL clause (e.g., TTL 3600)".to_string(),
                ));
            }
        },
        TableType::System => {
            return Err(KalamDbError::InvalidOperation(
                "Cannot create SYSTEM tables via SQL".to_string(),
            ));
        },
    }

    // Check if table already exists
    let schema_registry = app_context.schema_registry();
    let table_id = TableId::from_ref(&stmt.namespace_id, &stmt.table_name);
    let existing_def = schema_registry
        .get_table_if_exists(&table_id)
        .into_kalamdb_error("Failed to check table existence")?;

    if let Some(existing_def) = existing_def {
        if stmt.if_not_exists {
            log::info!("ℹ️  TABLE {} already exists (IF NOT EXISTS)", table_id_str);
            // Return the existing definition
            return Ok((*existing_def).clone());
        } else {
            log::warn!("❌ CREATE TABLE failed: {} already exists", table_id_str);
            return Err(KalamDbError::AlreadyExists(format!(
                "Table {} already exists",
                table_id_str
            )));
        }
    }

    // Resolve storage
    let (storage_id, _storage_type) = resolve_storage_info(&app_context, stmt.storage_id.as_ref())?;

    // Build columns from Arrow schema
    let columns: Vec<ColumnDefinition> = stmt
        .schema
        .fields()
        .iter()
        .enumerate()
        .map(|(idx, field)| {
            kalamdb_sql::validation::validate_column_name(field.name()).map_err(|e| {
                KalamDbError::InvalidOperation(format!(
                    "Invalid column name '{}': {}",
                    field.name(),
                    e
                ))
            })?;

            // First try to read KalamDataType from field metadata (preserves FILE, JSON, etc.)
            // Fall back to Arrow type conversion if metadata not present
            let kalam_type = kalamdb_commons::conversions::read_kalam_data_type_metadata(field)
                .unwrap_or_else(|| {
                    KalamDataType::from_arrow_type(field.data_type()).unwrap_or(KalamDataType::Text)
                });

            let is_pk =
                stmt.primary_key_column.as_ref().map(|pk| pk == field.name()).unwrap_or(false);

            let default_val =
                stmt.column_defaults.get(field.name()).cloned().unwrap_or(ColumnDefault::None);

            let mut column = ColumnDefinition::new(
                (idx + 1) as u64,
                field.name().clone(),
                (idx + 1) as u32,
                kalam_type,
                field.is_nullable(),
                is_pk,
                false,
                default_val,
                None,
            );
            if let Some(type_ref) = stmt.column_type_refs.get(field.name()) {
                apply_column_type_ref(app_context.as_ref(), &stmt.namespace_id, &mut column, type_ref)?;
            }
            Ok(column)
        })
        .collect::<Result<Vec<_>, KalamDbError>>()?;

    for column in &columns {
        validate_column_default(&app_context, &column.default_value)?;
    }

    // Build table options
    let table_options = match stmt.table_type {
        TableType::User => TableOptions::user(),
        TableType::Shared => TableOptions::shared(),
        TableType::Stream => TableOptions::stream(stmt.ttl_seconds.unwrap_or(3600)),
        TableType::System => TableOptions::system(),
    };

    // Create TableDefinition
    let mut table_def = TableDefinition::new(
        stmt.namespace_id.clone(),
        stmt.table_name.clone(),
        stmt.table_type,
        columns,
        table_options.clone(),
        None,
    )
    .map_err(KalamDbError::SchemaError)?;

    // Apply table-level options from DDL
    match (&mut table_def.table_options, stmt.table_type) {
        (TableOptions::User(opts), TableType::User) => {
            opts.storage_id = storage_id.clone();
            opts.use_user_storage = stmt.use_user_storage;
            opts.flush_policy = stmt.flush_policy.clone();
            if let Some(compression) = &stmt.compression {
                opts.compression = compression.clone();
            }
        },
        (TableOptions::Shared(opts), TableType::Shared) => {
            opts.storage_id = storage_id.clone();
            opts.flush_policy = stmt.flush_policy.clone();
            if let Some(compression) = &stmt.compression {
                opts.compression = compression.clone();
            }
        },
        (TableOptions::Stream(opts), TableType::Stream) => {
            if let Some(ttl) = stmt.ttl_seconds {
                opts.ttl_seconds = ttl;
            }
            if let Some(eviction_strategy) = &stmt.eviction_strategy {
                opts.eviction_strategy = eviction_strategy.clone();
            }
            if let Some(max_stream_size_bytes) = stmt.max_stream_size_bytes {
                opts.max_stream_size_bytes = max_stream_size_bytes;
            }
        },
        _ => {},
    }

    // Inject system columns (_version, _deleted)
    let sys_cols = app_context.system_columns_service();
    sys_cols.add_system_columns(&mut table_def)?;

    log::debug!(
        "✅ Built TableDefinition for {} (type: {:?}, columns: {}, version: {})",
        table_id_str,
        stmt.table_type,
        table_def.columns.len(),
        table_def.schema_version
    );

    Ok(table_def)
}

fn resolve_storage_info(
    app_context: &Arc<AppContext>,
    requested: Option<&StorageId>,
) -> Result<(StorageId, StorageType), KalamDbError> {
    let storages_provider = app_context.system_tables().storages();
    let storage_id = requested.cloned().unwrap_or_else(|| StorageId::from("local"));

    let storage = storages_provider.get_storage_by_id(&storage_id)?.ok_or_else(|| {
        log::error!("❌ CREATE TABLE failed: Storage '{}' does not exist", storage_id.as_str());
        KalamDbError::InvalidOperation(format!("Storage '{}' does not exist", storage_id.as_str()))
    })?;

    let storage_type = storage.storage_type;
    Ok((storage_id, storage_type))
}

pub fn validate_column_default(
    app_context: &AppContext,
    default: &ColumnDefault,
) -> Result<(), KalamDbError> {
    let Some(call) = default.as_routine_call() else {
        return Ok(());
    };
    if call.has_placeholder() {
        return Err(KalamDbError::InvalidSql(
            "DEFAULT procedure arguments cannot use placeholders".to_string(),
        ));
    }
    if call.is_builtin_default() {
        if !call.arguments.is_empty() {
            return Err(KalamDbError::InvalidOperation(format!(
                "built-in default {}() does not take arguments",
                call.unqualified_name().to_ascii_uppercase()
            )));
        }
        return Ok(());
    }

    let stores = app_context.system_tables().catalog_stores();
    let routine = stores.get_routine(&call.routine_id).map_err(|error| {
        KalamDbError::ExecutionError(format!(
            "failed to load procedure {}: {error}",
            call.routine_id
        ))
    })?;
    if routine.is_none() {
        return Err(KalamDbError::NotFound(format!(
            "procedure {} not found for column default",
            call.routine_id
        )));
    }
    Ok(())
}

pub fn apply_column_type_ref(
    app_context: &AppContext,
    current_schema: &kalamdb_commons::models::NamespaceId,
    column: &mut kalamdb_commons::models::schemas::ColumnDefinition,
    type_ref: &TypeReference,
) -> Result<(), KalamDbError> {
    column.is_array = type_ref.is_array;
    column.element_nullable = !type_ref.not_null || type_ref.is_array;
    let Some((namespace_id, name)) = type_ref.resolved_name(current_schema) else {
        return Ok(());
    };
    let stores = app_context.system_tables().catalog_stores();
    let catalog = stores
        .find_type(&namespace_id, &name)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        .ok_or_else(|| KalamDbError::NotFound(format!("type {namespace_id}.{name} not found")))?;
    if catalog.kind == kalamdb_commons::models::CatalogTypeKind::TopicPayload {
        return Err(KalamDbError::InvalidSql(format!(
            "topic payload type {namespace_id}.{name} cannot be stored as a column"
        )));
    }
    column.named_type_id = Some(catalog.type_id);
    Ok(())
}
