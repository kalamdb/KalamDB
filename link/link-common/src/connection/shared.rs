//! Shared WebSocket connection manager for real-time subscriptions.
//!
//! Provides a single WebSocket connection multiplexed across multiple
//! subscriptions. Handles a shared connection handle, subscription registry,
//! event routing, and reconnect behavior.

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};

use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    auth::ResolvedAuth,
    error::{KalamLinkError, Result},
    event_handlers::EventHandlers,
    models::{ChangeEvent, ConnectionOptions, SubscriptionInfo, SubscriptionOptions},
    timeouts::KalamLinkTimeouts,
    SeqId,
};

mod reconnect;
mod registry;
mod routing;

use reconnect::connection_task;
use registry::{ConnCmd, SubscriptionReady};

const DISCONNECT_TIMEOUT: Duration = Duration::from_millis(500);

pub(crate) struct SharedConnection {
    cmd_tx:              mpsc::Sender<ConnCmd>,
    connected:           Arc<AtomicBool>,
    _reconnect_attempts: Arc<AtomicU32>,
    _task:               JoinHandle<()>,
}

#[derive(Clone)]
pub(crate) struct SharedSubscriptionControl {
    cmd_tx: mpsc::Sender<ConnCmd>,
}

impl SharedSubscriptionControl {
    pub(crate) async fn unsubscribe(&self, id: String, generation: u64) {
        let _ = self
            .cmd_tx
            .send(ConnCmd::Unsubscribe {
                id,
                generation: Some(generation),
            })
            .await;
    }

    pub(crate) fn try_unsubscribe(&self, id: String, generation: u64) {
        let _ = self.cmd_tx.try_send(ConnCmd::Unsubscribe {
            id,
            generation: Some(generation),
        });
    }

    pub(crate) async fn progress(
        &self,
        id: String,
        generation: u64,
        seq_id: SeqId,
        advance_resume: bool,
    ) {
        let _ = self
            .cmd_tx
            .send(ConnCmd::Progress {
                id,
                generation,
                seq_id,
                advance_resume,
            })
            .await;
    }

    #[cfg(test)]
    pub(crate) fn test_control() -> Self {
        let (cmd_tx, _cmd_rx) = mpsc::channel::<ConnCmd>(1);
        Self { cmd_tx }
    }
}

impl SharedConnection {
    pub async fn connect(
        base_url: String,
        resolved_auth: Arc<RwLock<ResolvedAuth>>,
        timeouts: KalamLinkTimeouts,
        connection_options: ConnectionOptions,
        event_handlers: EventHandlers,
    ) -> Result<Self> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<ConnCmd>(256);
        let connected = Arc::new(AtomicBool::new(false));
        let reconnect_attempts = Arc::new(AtomicU32::new(0));

        let connected_clone = connected.clone();
        let reconnect_clone = reconnect_attempts.clone();
        let auto_reconnect = connection_options.auto_reconnect;
        let (ready_tx, ready_rx) = oneshot::channel::<Result<()>>();

        let task = tokio::spawn(async move {
            connection_task(
                cmd_rx,
                base_url,
                resolved_auth,
                timeouts,
                connection_options,
                event_handlers,
                connected_clone,
                reconnect_clone,
                Some(ready_tx),
            )
            .await;
        });

        match ready_rx.await {
            Ok(Ok(())) => {},
            Ok(Err(error)) => {
                if !auto_reconnect {
                    task.abort();
                    return Err(error);
                }
            },
            Err(_) => {
                task.abort();
                return Err(KalamLinkError::WebSocketError(
                    "Connection task exited before signalling readiness".to_string(),
                ));
            },
        }

        Ok(Self {
            cmd_tx,
            connected,
            _reconnect_attempts: reconnect_attempts,
            _task: task,
        })
    }

    /// Send a subscribe command without waiting for the server Ready ack.
    ///
    /// Returns the event receiver and a oneshot that resolves when the server
    /// confirms the subscription is ready. Callers can drop their lock on the
    /// connection before awaiting the oneshot, allowing other subscribes to
    /// pipeline through the same shared connection concurrently.
    pub async fn subscribe_send(
        &self,
        id: String,
        sql: String,
        options: Option<SubscriptionOptions>,
    ) -> Result<(mpsc::Receiver<Result<ChangeEvent>>, oneshot::Receiver<SubscriptionReady>)> {
        let (event_tx, event_rx) = mpsc::channel(crate::connection::DEFAULT_EVENT_CHANNEL_CAPACITY);
        let (result_tx, result_rx) = oneshot::channel();
        let request_initial_data = options.is_some();
        let options = options.unwrap_or_default();

        self.cmd_tx
            .send(ConnCmd::Subscribe {
                id: id.clone(),
                sql,
                options,
                request_initial_data,
                event_tx,
                result_tx,
            })
            .await
            .map_err(|_| {
                KalamLinkError::WebSocketError("Connection task is not running".to_string())
            })?;

        Ok((event_rx, result_rx))
    }

    pub async fn unsubscribe(&self, id: &str) -> Result<()> {
        self.cmd_tx
            .send(ConnCmd::Unsubscribe {
                id:         id.to_string(),
                generation: None,
            })
            .await
            .map_err(|_| {
                KalamLinkError::WebSocketError("Connection task is not running".to_string())
            })?;
        Ok(())
    }

    pub async fn disconnect(&self) {
        let (completed_tx, completed_rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(ConnCmd::Shutdown {
                completed: Some(completed_tx),
            })
            .await
            .is_ok()
        {
            if !matches!(tokio::time::timeout(DISCONNECT_TIMEOUT, completed_rx).await, Ok(Ok(()))) {
                self._task.abort();
                self.connected.store(false, Ordering::Release);
            }
        } else {
            self.connected.store(false, Ordering::Release);
        }
    }

    pub async fn list_subscriptions(&self) -> Vec<SubscriptionInfo> {
        let (result_tx, result_rx) = oneshot::channel();
        if self.cmd_tx.send(ConnCmd::ListSubscriptions { result_tx }).await.is_err() {
            return Vec::new();
        }
        result_rx.await.unwrap_or_default()
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub(crate) fn subscription_control(&self) -> SharedSubscriptionControl {
        SharedSubscriptionControl {
            cmd_tx: self.cmd_tx.clone(),
        }
    }
}

impl Drop for SharedConnection {
    fn drop(&mut self) {
        let _ = self.cmd_tx.try_send(ConnCmd::Shutdown { completed: None });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tokio::{
        sync::{mpsc, oneshot},
        time::Instant as TokioInstant,
    };

    use super::{
        registry::{
            clear_startup_deadline, reset_startup_deadline, resolve_subscription_key,
            resume_startup_deadline, startup_deadline, SubEntry,
        },
        *,
    };

    #[test]
    fn startup_deadline_disabled_when_initial_timeout_is_zero() {
        let mut timeouts = KalamLinkTimeouts::default();
        timeouts.initial_data_timeout = Duration::ZERO;
        assert_eq!(startup_deadline(&timeouts), None);
    }

    #[test]
    fn startup_deadline_helpers_toggle_resume_state() {
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (result_tx, _result_rx) = oneshot::channel();
        let mut entry = SubEntry {
            sql: "SELECT 1".to_string(),
            options: SubscriptionOptions::default(),
            request_initial_data: true,
            event_tx,
            last_seq_id: None,
            consumed_seq_id: None,
            batch_seq_id: None,
            is_loading: true,
            generation: 1,
            created_at_ms: 0,
            last_event_time_ms: None,
            pending_result_tx: Some(result_tx),
            ready_deadline: None,
            reconnect_resubscribe_pending: false,
        };

        reset_startup_deadline(&mut entry, &KalamLinkTimeouts::default(), true);
        assert!(entry.ready_deadline.is_some());
        assert!(entry.reconnect_resubscribe_pending);

        clear_startup_deadline(&mut entry);
        assert!(entry.ready_deadline.is_none());
        assert!(!entry.reconnect_resubscribe_pending);
    }

    #[test]
    fn resume_startup_deadline_uses_subscribe_timeout_window() {
        let timeouts = KalamLinkTimeouts {
            subscribe_timeout: Duration::from_secs(5),
            initial_data_timeout: Duration::from_secs(30),
            ..KalamLinkTimeouts::default()
        };

        let deadline = resume_startup_deadline(&timeouts).expect("resume deadline should exist");
        let remaining = deadline.saturating_duration_since(TokioInstant::now());

        assert!(remaining <= Duration::from_millis(5_100));
        assert!(remaining > Duration::from_secs(4));
    }

    #[test]
    fn subscription_key_resolution_requires_exact_id() {
        let subs = HashMap::from([("sub_1".to_string(), make_test_entry("SELECT 1"))]);

        let direct = resolve_subscription_key("sub_1", &subs).expect("direct match");
        assert_eq!(direct.as_str("sub_1"), "sub_1");

        let fallback =
            resolve_subscription_key("prefix-sub_1", &subs).expect("unique suffix match");
        assert_eq!(fallback.as_str("prefix-sub_1"), "sub_1");
    }

    #[test]
    fn subscription_key_resolution_rejects_ambiguous_suffixes() {
        let subs = HashMap::from([
            ("sub_1".to_string(), make_test_entry("SELECT 1")),
            ("xsub_1".to_string(), make_test_entry("SELECT 2")),
        ]);

        assert!(resolve_subscription_key("prefix-xsub_1", &subs).is_none());
    }

    fn make_test_entry(sql: &str) -> SubEntry {
        SubEntry {
            sql: sql.to_string(),
            options: SubscriptionOptions::default(),
            request_initial_data: true,
            event_tx: mpsc::channel(1).0,
            last_seq_id: None,
            consumed_seq_id: None,
            batch_seq_id: None,
            is_loading: true,
            generation: 1,
            created_at_ms: 0,
            last_event_time_ms: None,
            pending_result_tx: Some(oneshot::channel().0),
            ready_deadline: None,
            reconnect_resubscribe_pending: false,
        }
    }

    #[tokio::test]
    async fn disconnect_waits_for_connection_task_shutdown_ack() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<ConnCmd>(1);
        let connected = Arc::new(AtomicBool::new(true));
        let task_connected = Arc::clone(&connected);
        let task = tokio::spawn(async move {
            match cmd_rx.recv().await {
                Some(ConnCmd::Shutdown { completed }) => {
                    task_connected.store(false, Ordering::Release);
                    if let Some(completed) = completed {
                        let _ = completed.send(());
                    }
                },
                _ => panic!("expected shutdown command"),
            }
        });

        let connection = SharedConnection {
            cmd_tx,
            connected,
            _reconnect_attempts: Arc::new(AtomicU32::new(0)),
            _task: task,
        };

        tokio::time::timeout(Duration::from_secs(1), connection.disconnect())
            .await
            .expect("disconnect should complete after the connection task acknowledges shutdown");
    }

    #[tokio::test]
    async fn disconnect_marks_connection_closed_when_shutdown_ack_is_dropped() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<ConnCmd>(1);
        let connected = Arc::new(AtomicBool::new(true));
        let task = tokio::spawn(async move {
            match cmd_rx.recv().await {
                Some(ConnCmd::Shutdown { completed }) => drop(completed),
                _ => panic!("expected shutdown command"),
            }
        });

        let connection = SharedConnection {
            cmd_tx,
            connected: Arc::clone(&connected),
            _reconnect_attempts: Arc::new(AtomicU32::new(0)),
            _task: task,
        };

        connection.disconnect().await;
        assert!(!connected.load(Ordering::Acquire));
    }

    #[test]
    fn closed_subscription_cursor_cache_is_bounded() {
        let mut cache = HashMap::new();

        for index in 0..1_100 {
            let mut entry = make_test_entry("SELECT 1");
            entry.last_seq_id = Some(SeqId::from(index + 1));
            registry::cache_entry_seq(&mut cache, format!("sub-{index}"), &entry);
        }

        assert!(
            cache.len() <= 1_024,
            "closed subscription cursor history must not grow without bound"
        );
    }
}
