use kalamdb_raft::RaftError;

pub(super) async fn run_blocking_raft<T, F>(operation: F) -> Result<T, RaftError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, RaftError> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|join_error| RaftError::Internal(format!("Task join error: {}", join_error)))?
}
