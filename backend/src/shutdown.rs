//! Termination selection and ordered server cleanup.

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use anyhow::{anyhow, Result};
use kalamdb_core::app_context::AppContext;
use kalamdb_jobs::{JobsManagerRuntime, JobsShutdownStatus};
use kalamdb_live::ConnectionsManager;
use kalamdb_postgres_wire::PostgresWireListener;
use log::{info, warn};

use crate::http_server::HttpServerRuntime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShutdownSignal {
    CtrlC,
    SigTerm,
}

pub(crate) enum TerminationReason {
    Signal(ShutdownSignal),
    SignalHandlerFailed(std::io::Error),
    HttpStopped(std::io::Result<()>),
    PostgresWireStopped(std::io::Result<()>),
    Explicit,
}

impl TerminationReason {
    fn log(&self) {
        match self {
            Self::Signal(ShutdownSignal::CtrlC) => {
                info!("Received Ctrl+C, initiating graceful shutdown...");
            },
            Self::Signal(ShutdownSignal::SigTerm) => {
                info!("Received SIGTERM, initiating graceful shutdown...");
            },
            Self::SignalHandlerFailed(error) => {
                warn!(
                    "Failed to install shutdown signal handlers ({}), initiating graceful \
                     shutdown anyway",
                    error
                );
            },
            Self::HttpStopped(Ok(())) => {
                warn!("HTTP server stopped unexpectedly; initiating cleanup");
            },
            Self::HttpStopped(Err(error)) => {
                warn!("HTTP server failed ({}); initiating cleanup", error);
            },
            Self::PostgresWireStopped(Ok(())) => {
                warn!("PostgreSQL wire listener stopped unexpectedly; initiating cleanup");
            },
            Self::PostgresWireStopped(Err(error)) => {
                warn!("PostgreSQL wire listener failed ({}); initiating cleanup", error);
            },
            Self::Explicit => info!("Explicit server shutdown requested"),
        }
    }

    pub(crate) fn into_result(self) -> Result<()> {
        match self {
            Self::HttpStopped(Err(error)) => Err(error.into()),
            Self::PostgresWireStopped(Err(error)) => Err(error.into()),
            Self::SignalHandlerFailed(error) => Err(error.into()),
            Self::Signal(_)
            | Self::HttpStopped(Ok(()))
            | Self::PostgresWireStopped(Ok(()))
            | Self::Explicit => Ok(()),
        }
    }
}

pub(crate) async fn wait_for_termination(
    http_server: &mut HttpServerRuntime,
    postgres_wire: &mut Option<PostgresWireListener>,
) -> TerminationReason {
    let shutdown_signal = shutdown_signal_listener();

    tokio::select! {
        result = http_server.wait() => TerminationReason::HttpStopped(result),
        result = wait_for_postgres_wire(postgres_wire) => {
            TerminationReason::PostgresWireStopped(result)
        },
        signal = async {
            match shutdown_signal {
                Ok(signal) => signal.await,
                Err(error) => Err(error),
            }
        } => {
            match signal {
                Ok(signal) => TerminationReason::Signal(signal),
                Err(error) => TerminationReason::SignalHandlerFailed(error),
            }
        },
    }
}

async fn wait_for_postgres_wire(
    postgres_wire: &mut Option<PostgresWireListener>,
) -> std::io::Result<()> {
    match postgres_wire {
        Some(listener) => listener.wait().await,
        None => std::future::pending().await,
    }
}

pub(crate) async fn shutdown_server(
    reason: TerminationReason,
    http_server: HttpServerRuntime,
    mut postgres_wire: Option<PostgresWireListener>,
    jobs_runtime: JobsManagerRuntime,
    connection_registry: Arc<ConnectionsManager>,
    app_context: Arc<AppContext>,
    job_drain_timeout: Duration,
) -> Result<()> {
    reason.log();
    let mut cleanup_error = None;

    // Quiesce every ingress path before asking established connections to drain.
    http_server.stop_accepting().await;
    if let Some(listener) = postgres_wire.as_mut() {
        listener.stop_accepting();
    }

    info!("Shutting down WebSocket connections...");
    connection_registry.shutdown(Duration::from_secs(1)).await;

    info!("Stopping HTTP server...");
    if let Err(error) = http_server.shutdown(true).await {
        cleanup_error = Some(anyhow!("HTTP server shutdown failed: {error}"));
    }

    if let Some(listener) = postgres_wire {
        if let Err(error) = listener.finish().await {
            warn!("PostgreSQL wire listener shutdown failed: {}", error);
            cleanup_error.get_or_insert_with(|| anyhow!(error));
        }
    }

    if let Err(error) =
        shutdown_background_services(Some(jobs_runtime), app_context, job_drain_timeout).await
    {
        cleanup_error.get_or_insert(error);
    }

    reason.into_result()?;
    if let Some(error) = cleanup_error {
        return Err(error);
    }
    Ok(())
}

/// Stop services started during bootstrap when a later startup phase fails.
pub(crate) async fn shutdown_background_services(
    jobs_runtime: Option<JobsManagerRuntime>,
    app_context: Arc<AppContext>,
    job_drain_timeout: Duration,
) -> Result<()> {
    let mut cleanup_error = None;

    if let Some(jobs_runtime) = jobs_runtime {
        info!(
            "Waiting up to {:.1}s for in-flight jobs to complete...",
            job_drain_timeout.as_secs_f64()
        );
        match jobs_runtime.shutdown(job_drain_timeout).await {
            Ok(JobsShutdownStatus::Completed) => info!("In-flight jobs completed"),
            Ok(JobsShutdownStatus::TimedOut) => {
                warn!("Timed out waiting for in-flight jobs to complete");
            },
            Err(error) => {
                warn!("Jobs manager shutdown failed: {}", error);
                cleanup_error = Some(anyhow!(error));
            },
        }
    }

    info!("Shutting down Raft executor...");
    if let Err(error) = app_context.executor().shutdown().await {
        warn!("Raft executor shutdown failed: {}", error);
        cleanup_error.get_or_insert_with(|| anyhow!(error));
    }

    match cleanup_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

async fn select_shutdown_signal<CtrlCFut, SigTermFut>(
    ctrl_c: CtrlCFut,
    sigterm: SigTermFut,
) -> std::io::Result<ShutdownSignal>
where
    CtrlCFut: Future<Output = std::io::Result<()>>,
    SigTermFut: Future<Output = std::io::Result<()>>,
{
    tokio::select! {
        result = ctrl_c => {
            result?;
            Ok(ShutdownSignal::CtrlC)
        },
        result = sigterm => {
            result?;
            Ok(ShutdownSignal::SigTerm)
        },
    }
}

type ShutdownSignalFuture = Pin<Box<dyn Future<Output = std::io::Result<ShutdownSignal>> + Send>>;

fn shutdown_signal_listener() -> std::io::Result<ShutdownSignalFuture> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let ctrl_c = tokio::signal::ctrl_c();
        let mut sigterm = signal(SignalKind::terminate())?;
        Ok(Box::pin(async move {
            select_shutdown_signal(ctrl_c, async move {
                sigterm.recv().await.ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "SIGTERM signal stream closed unexpectedly",
                    )
                })
            })
            .await
        }))
    }

    #[cfg(not(unix))]
    {
        let ctrl_c = tokio::signal::ctrl_c();
        Ok(Box::pin(async move {
            select_shutdown_signal(ctrl_c, std::future::pending::<std::io::Result<()>>()).await
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::future::{pending, ready};

    use super::{select_shutdown_signal, ShutdownSignal, TerminationReason};

    #[tokio::test]
    #[ntest::timeout(1000)]
    async fn select_shutdown_signal_returns_ctrl_c_when_ctrl_c_resolves_first() {
        let signal = select_shutdown_signal(
            ready::<std::io::Result<()>>(Ok(())),
            pending::<std::io::Result<()>>(),
        )
        .await
        .expect("ctrl+c future should succeed");

        assert_eq!(signal, ShutdownSignal::CtrlC);
    }

    #[tokio::test]
    #[ntest::timeout(1000)]
    async fn select_shutdown_signal_returns_sigterm_when_sigterm_resolves_first() {
        let signal = select_shutdown_signal(
            pending::<std::io::Result<()>>(),
            ready::<std::io::Result<()>>(Ok(())),
        )
        .await
        .expect("sigterm future should succeed");

        assert_eq!(signal, ShutdownSignal::SigTerm);
    }

    #[test]
    fn termination_reason_preserves_http_server_error() {
        let reason = TerminationReason::HttpStopped(Err(std::io::Error::other("accept failed")));

        let error = reason.into_result().expect_err("HTTP failure must be returned");

        assert!(error.to_string().contains("accept failed"));
    }
}
