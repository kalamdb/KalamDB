use kalamdb_commons::TableId;
use kalamdb_core::error::KalamDbError;

use crate::executors::{JobContext, JobParams};

pub(crate) async fn cleanup_empty_shared_scope_if_needed<T: JobParams>(
    ctx: &JobContext<T>,
    table_id: &TableId,
) -> Result<(), KalamDbError> {
    let provider = ctx.app_ctx.schema_registry().get_kalam_provider(table_id).ok_or_else(|| {
        KalamDbError::TableNotFound(format!("Provider not found for {}", table_id))
    })?;

    if !provider.is_shared() {
        return Err(KalamDbError::InvalidOperation(format!(
            "Provider for {} is not a shared table",
            table_id
        )));
    }

    if provider.shared_has_live_row().await? {
        return Ok(());
    }

    let manifest_service = ctx.app_ctx.manifest_service();
    let cleanup_table_id = table_id.clone();
    let deleted_files = tokio::task::spawn_blocking(move || {
        manifest_service.clear_segments_and_delete_files(&cleanup_table_id, None)
    })
    .await
    .map_err(|e| {
        KalamDbError::InvalidOperation(format!("Empty-scope cleanup task panicked: {}", e))
    })??;

    ctx.log_info(&format!(
        "Removed {} cold segment file(s) for empty shared table {}",
        deleted_files, table_id
    ));

    Ok(())
}
