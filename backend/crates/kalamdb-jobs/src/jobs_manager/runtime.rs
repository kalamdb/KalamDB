//! Owned runtime for the jobs manager background loop.

use std::{sync::Arc, time::Duration};

use kalamdb_core::error::KalamDbError;
use tokio::task::{JoinError, JoinHandle, JoinSet};

use super::JobsManager;

/// Result of stopping the jobs runtime within its configured deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobsShutdownStatus {
    Completed,
    TimedOut,
}

/// Owns the jobs manager run loop so callers can stop and join it deterministically.
pub struct JobsManagerRuntime {
    manager: Arc<JobsManager>,
    task:    JoinHandle<Result<(), KalamDbError>>,
}

impl JobsManagerRuntime {
    #[must_use]
    pub fn start(manager: Arc<JobsManager>, max_concurrent: usize) -> Self {
        let task_manager = Arc::clone(&manager);
        let task = tokio::spawn(async move { task_manager.run_loop(max_concurrent).await });
        Self { manager, task }
    }

    /// Stop scheduling new jobs and wait for in-flight jobs to finish.
    pub async fn shutdown(self, deadline: Duration) -> Result<JobsShutdownStatus, KalamDbError> {
        self.manager.shutdown();

        match wait_for_task(self.task, deadline).await {
            TaskWaitStatus::Completed(result) => {
                result?;
                Ok(JobsShutdownStatus::Completed)
            },
            TaskWaitStatus::JoinFailed(error) => {
                Err(KalamDbError::Other(format!("jobs manager task failed: {error}")))
            },
            TaskWaitStatus::TimedOut => Ok(JobsShutdownStatus::TimedOut),
        }
    }
}

enum TaskWaitStatus<T, E> {
    Completed(Result<T, E>),
    JoinFailed(JoinError),
    TimedOut,
}

async fn wait_for_task<T, E>(
    mut task: JoinHandle<Result<T, E>>,
    deadline: Duration,
) -> TaskWaitStatus<T, E> {
    match tokio::time::timeout(deadline, &mut task).await {
        Ok(Ok(result)) => TaskWaitStatus::Completed(result),
        Ok(Err(error)) => TaskWaitStatus::JoinFailed(error),
        Err(_) => {
            task.abort();
            let _ = task.await;
            TaskWaitStatus::TimedOut
        },
    }
}

pub(super) async fn drain_job_tasks(tasks: &mut JoinSet<()>) {
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            log::error!("Job task failed while draining during shutdown: {}", error);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Duration,
    };

    use tokio::task::{JoinHandle, JoinSet};

    use super::{drain_job_tasks, wait_for_task, TaskWaitStatus};

    #[tokio::test]
    #[ntest::timeout(1000)]
    async fn wait_for_task_allows_in_flight_work_to_finish() {
        let finished = Arc::new(AtomicBool::new(false));
        let task_finished = Arc::clone(&finished);
        let task: JoinHandle<Result<(), &'static str>> = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            task_finished.store(true, Ordering::Release);
            Ok(())
        });

        let status = wait_for_task(task, Duration::from_millis(100)).await;

        assert!(finished.load(Ordering::Acquire));
        assert!(matches!(status, TaskWaitStatus::Completed(Ok(()))));
    }

    #[tokio::test]
    #[ntest::timeout(1000)]
    async fn wait_for_task_aborts_work_after_the_deadline() {
        let task: JoinHandle<Result<(), &'static str>> = tokio::spawn(async {
            std::future::pending::<()>().await;
            Ok(())
        });

        let status = wait_for_task(task, Duration::from_millis(10)).await;

        assert!(matches!(status, TaskWaitStatus::TimedOut));
    }

    #[tokio::test]
    #[ntest::timeout(1000)]
    async fn drain_job_tasks_allows_in_flight_work_to_finish() {
        let finished = Arc::new(AtomicBool::new(false));
        let task_finished = Arc::clone(&finished);
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            task_finished.store(true, Ordering::Release);
        });

        drain_job_tasks(&mut tasks).await;

        assert!(finished.load(Ordering::Acquire));
        assert!(tasks.is_empty());
    }
}
