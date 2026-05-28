use kalamdb_core::error::KalamDbError;

pub async fn run_blocking<T, E, F>(operation: F) -> Result<T, KalamDbError>
where
    T: Send + 'static,
    E: Into<KalamDbError> + Send + 'static,
    F: FnOnce() -> Result<T, E> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|join_error| {
            KalamDbError::ExecutionError(format!("Task join error: {}", join_error))
        })?
        .map_err(Into::into)
}
