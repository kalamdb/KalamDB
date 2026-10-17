//! Cold-storage primary-key lookup and uniqueness checks.

use std::{collections::HashSet, sync::Arc};

use datafusion::{logical_expr::Expr, scalar::ScalarValue};
use kalamdb_commons::{
    models::rows::Row, schemas::TableType, try_pk_bucket_key, StorageKey, TableId, UserId,
};
use kalamdb_datafusion_sources::exec::prefers_version;
use kalamdb_filestore::registry::{ListResult, StorageCached};
use kalamdb_system::Manifest;

use super::provider::BaseTableProvider;
use crate::{
    error::KalamDbError,
    manifest::ManifestAccessPlanner,
    utils::{
        core::TableProviderCore, parquet::scan_parquet_files_as_batch_async,
        row_utils::resolve_user_scope, unified_dml,
    },
};

/// Locate the latest non-deleted row matching the provided primary-key value (async).
///
/// This function scans cold storage (Parquet files) to find a row by its primary key.
/// For UPDATE/DELETE operations on cold storage data, this is needed to:
/// 1. Get the current row data to merge with updates
/// 2. Verify the row exists before creating a tombstone (delete)
///
/// Uses async I/O to avoid blocking the tokio runtime.
/// For hot storage lookups, providers should use their own O(1) PK index first.
pub async fn find_row_by_pk<P, K, V>(
    provider: &P,
    scope: Option<&UserId>,
    pk_value: &str,
) -> Result<Option<(K, V)>, KalamDbError>
where
    P: BaseTableProvider<K, V>,
    K: StorageKey,
{
    use datafusion::prelude::{col, lit};

    use crate::utils::version_resolution::{parquet_batch_to_rows, ParquetRowData};

    let pk_name = provider.primary_key_field_name();
    let user_scope = resolve_user_scope(scope);

    // Build filter for the specific PK value
    let filter: Expr = col(pk_name).eq(lit(pk_value));

    // Get core from provider (we need schema, table_id, table_type, and storage access)
    let core = provider.core();
    let table_id = provider.table_id();
    let table_type = provider.provider_table_type();
    let schema = provider.schema_ref();

    // Scan cold storage for this PK value using async I/O
    let batch = scan_parquet_files_as_batch_async(
        core,
        table_id,
        table_type,
        scope,
        schema,
        Some(&filter),
        None,
    )
    .await?;

    if batch.num_rows() == 0 {
        return Ok(None);
    }

    // Parse rows from Parquet batch
    let rows_data: Vec<ParquetRowData> = parquet_batch_to_rows(&batch)?;

    // Find the latest non-deleted version with matching PK
    // Rows should already be filtered by PK, but we need version resolution
    let mut latest: Option<ParquetRowData> = None;

    for row_data in rows_data {
        // Skip deleted rows
        if row_data.deleted {
            continue;
        }

        // Check if this row matches the PK value
        if let Some(row_pk) = row_data.fields.values.get(pk_name) {
            let Ok(row_pk_key) = try_pk_bucket_key(row_pk) else {
                continue;
            };
            if row_pk_key.to_string() != pk_value {
                continue;
            }

            // Keep the row with the highest (commit_seq, seq) version.
            if latest
                .as_ref()
                .map(|current| prefers_version(row_data.version, current.version))
                .unwrap_or(true)
            {
                latest = Some(row_data);
            }
        }
    }

    // Convert ParquetRowData to the provider's (K, V) types
    if let Some(row_data) = latest {
        let result = provider.construct_row_from_parquet_data(user_scope, &row_data)?;
        return Ok(result);
    }

    Ok(None)
}

fn resolve_storage_cached_for_pk(
    core: &TableProviderCore,
    table_id: &TableId,
) -> Result<Option<Arc<StorageCached>>, KalamDbError> {
    let storage_id = match core.services.schema_registry.get_storage_id(table_id) {
        Ok(id) => id,
        Err(_) => return Ok(None),
    };

    let storage_registry = core.services.storage_registry.as_ref().ok_or_else(|| {
        KalamDbError::InvalidOperation("Storage registry not configured".to_string())
    })?;
    storage_registry.get_cached(&storage_id).map_err(KalamDbError::from)
}

fn collect_parquet_files_from_list(list_result: &ListResult) -> Vec<String> {
    let prefix = list_result.prefix.trim_end_matches('/');
    list_result
        .paths
        .iter()
        .filter_map(|path| {
            let stripped = strip_list_prefix(path, prefix).unwrap_or(path);
            if stripped.ends_with(".parquet") {
                Some(stripped.to_string())
            } else {
                None
            }
        })
        .collect()
}

fn strip_list_prefix<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let trimmed_prefix = prefix.trim_end_matches('/');
    if trimmed_prefix.is_empty() {
        return Some(path.trim_start_matches('/'));
    }
    if path == trimmed_prefix {
        return None;
    }
    path.strip_prefix(trimmed_prefix)
        .map(|stripped| stripped.trim_start_matches('/'))
}

enum ColdPkQuery<'a> {
    Single { pk_value: &'a str },
    Batch { pk_values: &'a [String] },
}

struct ColdPkTargets {
    storage:     Arc<StorageCached>,
    files:       Vec<String>,
    scope_label: String,
}

/// Shared manifest load, segment prune, and storage listing for cold PK checks.
///
/// The single-PK path does not trace candidate selection. The batch path keeps
/// its pruning and listing traces.
async fn resolve_cold_pk_files(
    core: &TableProviderCore,
    table_id: &TableId,
    table_type: TableType,
    user_id: Option<&UserId>,
    pk_column_id: u64,
    query: ColdPkQuery<'_>,
) -> Result<Option<ColdPkTargets>, KalamDbError> {
    let namespace = table_id.namespace_id();
    let table = table_id.table_name();
    let scope_label = user_id
        .map(|uid| format!("user={}", uid.as_str()))
        .unwrap_or_else(|| format!("scope={}", table_type.as_str()));
    let label = match &query {
        ColdPkQuery::Single { .. } => "pk_exists_in_cold",
        ColdPkQuery::Batch { .. } => "pk_exists_batch_in_cold",
    };

    let manifest_service = core.services.manifest_service.clone();
    let cache_result = kalamdb_observability::kdb_await_in_info_span!(
        manifest_service.get_or_load_async(table_id, user_id),
        "table.pk_cold_exists"
    );

    let manifest: Option<&Manifest> = match &cache_result {
        Ok(Some(entry)) => Some(&entry.manifest),
        Ok(None) => None,
        Err(kalamdb_store::StorageError::SerializationError(e)) => {
            return Err(KalamDbError::InvalidOperation(format!(
                "Failed to load manifest for {} {}: {}",
                table_id, scope_label, e
            )));
        },
        Err(e) => {
            log::warn!(
                "[{label}] Manifest cache error for {}.{} {}: {}",
                namespace.as_str(),
                table.as_str(),
                scope_label,
                e
            );
            None
        },
    };

    // Fast path: manifest loaded and has no cold segments.
    // Avoid storage listing on hot-only write paths.
    if let Some(ref manifest) = manifest {
        if manifest.segments.is_empty() {
            return Ok(None);
        }
    }

    let planner = ManifestAccessPlanner::new();
    let mut storage_cached_for_scan: Option<Arc<StorageCached>> = None;
    let files_to_scan: Vec<String> = if let Some(ref manifest) = manifest {
        let pruned_paths = match &query {
            ColdPkQuery::Single { pk_value } => {
                planner.plan_by_pk_value(manifest, pk_column_id, pk_value)
            },
            ColdPkQuery::Batch { pk_values } => {
                planner.plan_by_pk_values(manifest, pk_column_id, pk_values)
            },
        };
        if pruned_paths.is_empty() {
            if let ColdPkQuery::Batch { .. } = &query {
                log::trace!(
                    "[pk_exists_batch_in_cold] Manifest pruning returned no candidate segments \
                     for {}.{} {} - PKs not in cold",
                    namespace.as_str(),
                    table.as_str(),
                    scope_label
                );
            }
            return Ok(None);
        }
        if let ColdPkQuery::Batch { pk_values } = &query {
            log::trace!(
                "[pk_exists_batch_in_cold] Manifest pruning: {} of {} segments may contain {} PKs \
                 for {}.{} {}",
                pruned_paths.len(),
                manifest.segments.len(),
                pk_values.len(),
                namespace.as_str(),
                table.as_str(),
                scope_label
            );
        }
        pruned_paths
    } else {
        let Some(storage_cached) = resolve_storage_cached_for_pk(core, table_id)? else {
            if let ColdPkQuery::Batch { .. } = &query {
                log::trace!(
                    "[pk_exists_batch_in_cold] No storage id for {}.{} {} - PK not in cold",
                    namespace.as_str(),
                    table.as_str(),
                    scope_label
                );
            }
            return Ok(None);
        };
        let list_result = match storage_cached.list(table_type, table_id, user_id).await {
            Ok(result) => result,
            Err(_) => {
                if let ColdPkQuery::Batch { .. } = &query {
                    log::trace!(
                        "[pk_exists_batch_in_cold] No storage dir for {}.{} {} - PK not in cold",
                        namespace.as_str(),
                        table.as_str(),
                        scope_label
                    );
                }
                return Ok(None);
            },
        };
        if list_result.is_empty() {
            if let ColdPkQuery::Batch { .. } = &query {
                log::trace!(
                    "[pk_exists_batch_in_cold] No files in storage for {}.{} {} - PK not in cold",
                    namespace.as_str(),
                    table.as_str(),
                    scope_label
                );
            }
            return Ok(None);
        }
        storage_cached_for_scan = Some(storage_cached);
        collect_parquet_files_from_list(&list_result)
    };

    if files_to_scan.is_empty() {
        return Ok(None);
    }

    let storage = match storage_cached_for_scan {
        Some(cached) => cached,
        None => match resolve_storage_cached_for_pk(core, table_id)? {
            Some(cached) => cached,
            None => return Ok(None),
        },
    };

    Ok(Some(ColdPkTargets {
        storage,
        files: files_to_scan,
        scope_label,
    }))
}

/// Check if a PK value exists in cold storage (Parquet files) using manifest-based pruning (async).
///
/// **Optimized for PK existence checks during INSERT**:
/// 1. Load manifest from cache (no disk I/O if cached)
/// 2. Use column_stats min/max to prune segments that definitely don't contain the PK
/// 3. Only scan relevant Parquet files (if any)
/// 4. Scan with version resolution to handle MVCC (latest non-deleted wins)
///
/// This is much faster than `find_row_by_pk` which scans ALL cold storage rows.
///
/// # Arguments
/// * `core` - TableProviderCore for app_context access
/// * `table_id` - Table identifier
/// * `table_type` - TableType (User, Shared, Stream)
/// * `user_id` - Optional user ID for scoping (User tables)
/// * `pk_column` - Name of the primary key column
/// * `pk_column_id` - Column ID of the primary key column (for manifest column_stats lookup)
/// * `pk_value` - The PK value to check for
///
/// # Returns
/// * `Ok(true)` - PK exists in cold storage (non-deleted)
/// * `Ok(false)` - PK does not exist in cold storage
pub async fn pk_exists_in_cold(
    core: &TableProviderCore,
    table_id: &TableId,
    table_type: TableType,
    user_id: Option<&UserId>,
    pk_column: &str,
    pk_column_id: u64,
    pk_value: &str,
) -> Result<bool, KalamDbError> {
    let Some(ColdPkTargets {
        storage,
        files,
        scope_label,
    }) = resolve_cold_pk_files(
        core,
        table_id,
        table_type,
        user_id,
        pk_column_id,
        ColdPkQuery::Single { pk_value },
    )
    .await?
    else {
        return Ok(false);
    };

    let namespace = table_id.namespace_id();
    let table = table_id.table_name();
    // Scan pruned Parquet files and check for PK using StorageCached.
    // Manifest paths are just filenames (e.g., "batch-0.parquet"), so prepend storage_path
    for file_name in files {
        if crate::utils::pk::pk_exists_in_parquet_file(
            storage.as_ref(),
            table_type,
            table_id,
            user_id,
            &file_name,
            pk_column,
            pk_value,
        )
        .await?
        {
            log::trace!(
                "[pk_exists_in_cold] Found PK {} in {} for {}.{} {}",
                pk_value,
                file_name,
                namespace.as_str(),
                table.as_str(),
                scope_label
            );
            return Ok(true);
        }
    }

    Ok(false)
}

/// Batch check if any PK values exist in cold storage (Parquet files) (async).
///
/// **OPTIMIZED for batch INSERT**: Checks multiple PK values in a single pass through cold storage.
/// This is O(files) instead of O(files × N) where N is the number of PK values.
///
/// # Arguments
/// * `core` - TableProviderCore for app_context access
/// * `table_id` - Table identifier
/// * `table_type` - TableType (User, Shared, Stream)
/// * `user_id` - Optional user ID for scoping (User tables)
/// * `pk_column` - Name of the primary key column
/// * `pk_column_id` - Column ID of the primary key column (for manifest column_stats lookup)
/// * `pk_values` - The PK values to check for
///
/// # Returns
/// * `Ok(Some(pk))` - First PK that exists in cold storage (non-deleted)
/// * `Ok(None)` - None of the PKs exist in cold storage
pub async fn pk_exists_batch_in_cold(
    core: &TableProviderCore,
    table_id: &TableId,
    table_type: TableType,
    user_id: Option<&UserId>,
    pk_column: &str,
    pk_column_id: u64,
    pk_values: &[String],
) -> Result<Option<String>, KalamDbError> {
    if pk_values.is_empty() {
        return Ok(None);
    }

    let Some(ColdPkTargets {
        storage,
        files,
        scope_label,
    }) = resolve_cold_pk_files(
        core,
        table_id,
        table_type,
        user_id,
        pk_column_id,
        ColdPkQuery::Batch { pk_values },
    )
    .await?
    else {
        return Ok(None);
    };

    let namespace = table_id.namespace_id();
    let table = table_id.table_name();
    let pk_set: HashSet<&str> = pk_values.iter().map(|value| value.as_str()).collect();
    for file_name in files {
        if let Some(found_pk) = crate::utils::pk::first_existing_pk_in_parquet_file(
            storage.as_ref(),
            table_type,
            table_id,
            user_id,
            &file_name,
            pk_column,
            &pk_set,
        )
        .await?
        {
            log::trace!(
                "[pk_exists_batch_in_cold] Found PK {} in {} for {}.{} {}",
                found_pk,
                file_name,
                namespace.as_str(),
                table.as_str(),
                scope_label
            );
            return Ok(Some(found_pk));
        }
    }

    Ok(None)
}

/// Ensure an INSERT payload either auto-generates or provides a unique primary-key value
///
/// This uses find_row_key_by_id_field which providers can override to use PK indexes
/// for O(1) lookup instead of scanning all rows.
///
/// **Optimization**: If the PK column is AUTO_INCREMENT or SNOWFLAKE_ID, this check
/// is skipped since the system guarantees unique values.
///
/// **Cold Storage Check**: After checking hot storage (RocksDB), this also checks
/// cold storage (Parquet files) using PkExistenceChecker for full PK uniqueness validation.
pub async fn ensure_unique_pk_value<P, K, V>(
    provider: &P,
    scope: Option<&UserId>,
    row_data: &Row,
) -> Result<(), KalamDbError>
where
    P: BaseTableProvider<K, V>,
    K: StorageKey,
{
    let table_id = provider.table_id();

    // Fast path: Skip uniqueness check if PK is auto-increment (O(1) cached value)
    if provider.core().is_auto_increment_pk() {
        log::trace!(
            "[ensure_unique_pk_value] Skipping PK check for {} - PK is auto-increment",
            table_id
        );
        return Ok(());
    }

    let pk_name = provider.primary_key_field_name();
    if let Some(pk_value) = row_data.get(pk_name) {
        if !matches!(pk_value, ScalarValue::Null) {
            let pk_str = unified_dml::extract_user_pk_value(row_data, pk_name)?;
            let user_scope = resolve_user_scope(scope);

            // Step 1: Check hot storage (RocksDB) - fast PK index lookup
            if provider.find_row_key_by_id_field(user_scope, &pk_str).await?.is_some() {
                return Err(KalamDbError::AlreadyExists(format!(
                    "Primary key violation: value '{}' already exists in column '{}' (hot storage)",
                    pk_str, pk_name
                )));
            }

            ensure_pk_absent_in_cold(provider, scope, &pk_str).await?;
        }
    }
    Ok(())
}

/// Reject a primary key that already exists in a cold segment.
///
/// Call this after a hot lookup has already shown the key is absent from
/// RocksDB, so the insert path does not scan the primary-key index twice.
pub async fn ensure_pk_absent_in_cold<P, K, V>(
    provider: &P,
    scope: Option<&UserId>,
    pk_str: &str,
) -> Result<(), KalamDbError>
where
    P: BaseTableProvider<K, V>,
    K: StorageKey,
{
    let core = provider.core();
    let Some(storage_registry) = core.services.storage_registry.clone() else {
        return Ok(());
    };

    let pk_checker = crate::utils::pk::PkExistenceChecker::new(
        core.services.schema_registry.clone(),
        storage_registry,
        core.services.manifest_service.clone(),
    );
    let check_result = pk_checker.check_pk_exists(core, scope, pk_str).await?;
    if let crate::utils::pk::PkCheckResult::FoundInCold { segment_path } = check_result {
        return Err(KalamDbError::AlreadyExists(format!(
            "Primary key violation: value '{}' already exists in column '{}' (cold storage: {})",
            pk_str,
            provider.primary_key_field_name(),
            segment_path
        )));
    }
    Ok(())
}

/// Validate that an UPDATE operation doesn't change the PK to an existing value
///
/// This is called when an UPDATE includes the PK column in the SET clause.
/// If the new PK value already exists (for a different row), returns an error.
///
/// **Skip conditions**:
/// - PK value is not being changed (new value == old value)
/// - PK column is not in the updates
/// - PK column has AUTO_INCREMENT (not allowed to be updated)
///
/// # Arguments
/// * `provider` - The table provider to check against
/// * `scope` - Optional user ID for scoping (User tables)
/// * `updates` - The Row containing update values
/// * `current_pk_value` - The current PK value of the row being updated
///
/// # Returns
/// * `Ok(())` if the update is valid
/// * `Err(AlreadyExists)` if the new PK value already exists
/// * `Err(InvalidOperation)` if trying to change an auto-increment PK
pub async fn validate_pk_update<P, K, V>(
    provider: &P,
    scope: Option<&UserId>,
    updates: &Row,
    current_pk_value: &ScalarValue,
) -> Result<(), KalamDbError>
where
    P: BaseTableProvider<K, V>,
    K: StorageKey,
{
    let table_id = provider.table_id();
    let pk_name = provider.primary_key_field_name();

    // Check if PK is in the update values
    let new_pk_value = match updates.get(pk_name) {
        Some(v) if !matches!(v, ScalarValue::Null) => v,
        _ => return Ok(()), // PK not being updated, nothing to validate
    };

    // Check if the value is actually changing
    if new_pk_value == current_pk_value {
        return Ok(()); // Same value, no change
    }

    // Fast path: Reject PK modification if it's auto-increment (O(1) cached value)
    if provider.core().is_auto_increment_pk() {
        return Err(KalamDbError::InvalidOperation(format!(
            "Cannot modify auto-increment primary key column '{}' in table {}",
            pk_name, table_id
        )));
    }

    // Check if the new PK value already exists
    let new_pk_str = unified_dml::extract_user_pk_value(updates, pk_name)?;
    let user_scope = resolve_user_scope(scope);

    if provider.find_row_key_by_id_field(user_scope, &new_pk_str).await?.is_some() {
        return Err(KalamDbError::AlreadyExists(format!(
            "Primary key violation: value '{}' already exists in column '{}' (UPDATE would create \
             duplicate)",
            new_pk_str, pk_name
        )));
    }

    log::trace!(
        "[validate_pk_update] PK change validated: {} -> {} for {}",
        current_pk_value,
        new_pk_str,
        table_id
    );

    Ok(())
}
