//! Push a locally stored catalog type through the meta Raft log.
//!
//! Named types used to be written only on the node that accepted the SQL.
//! Followers then failed `CREATE TABLE` while resolving that type and the
//! meta group stopped applying.

use std::sync::Arc;

use kalamdb_commons::models::TypeId;
use kalamdb_core::{app_context::AppContext, error::KalamDbError};
use kalamdb_system::CatalogStores;

use crate::helpers::async_blocking::run_blocking;

pub async fn replicate_stored_type(
    app: &Arc<AppContext>,
    type_id: &TypeId,
) -> Result<(), KalamDbError> {
    let app_for_read = Arc::clone(app);
    let type_id = type_id.clone();
    let snapshot = run_blocking(move || load_type_snapshot(&app_for_read, &type_id)).await?;
    let Some((catalog_type, fields)) = snapshot else {
        return Ok(());
    };
    app.applier()
        .upsert_catalog_type(catalog_type, fields)
        .await
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(())
}

pub async fn replicate_named_type(
    app: &Arc<AppContext>,
    namespace_id: &kalamdb_commons::models::NamespaceId,
    name: &str,
) -> Result<(), KalamDbError> {
    let app_for_read = Arc::clone(app);
    let namespace_id = namespace_id.clone();
    let name = name.to_string();
    let type_id = run_blocking(move || {
        app_for_read
            .system_tables()
            .catalog_stores()
            .find_type(&namespace_id, &name)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))
            .map(|found| found.map(|catalog_type| catalog_type.type_id))
    })
    .await?;
    if let Some(type_id) = type_id {
        replicate_stored_type(app, &type_id).await?;
    }
    Ok(())
}

pub async fn replicate_dropped_type(
    app: &Arc<AppContext>,
    type_id: TypeId,
) -> Result<(), KalamDbError> {
    app.applier()
        .drop_catalog_type(type_id)
        .await
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(())
}

fn load_type_snapshot(
    app: &AppContext,
    type_id: &TypeId,
) -> Result<
    Option<(kalamdb_system::CatalogType, Vec<kalamdb_system::CatalogTypeField>)>,
    KalamDbError,
> {
    let stores: CatalogStores = app.system_tables().catalog_stores();
    let Some(catalog_type) = stores
        .get_type(type_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
    else {
        return Ok(None);
    };
    let fields = stores
        .list_type_fields(type_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(Some((catalog_type, fields)))
}
