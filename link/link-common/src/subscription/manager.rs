//! `SubscriptionManager` – consumer handle for a single subscription.
//!
//! Receives events routed by the shared
//! [`SharedConnection`](crate::connection::SharedConnection).

use std::collections::VecDeque;

use tokio::sync::mpsc;

use crate::{
    connection::SharedSubscriptionControl,
    error::Result,
    models::ChangeEvent,
    seq_tracking,
    subscription::{buffer_event, event_progress, SubscriptionAckMode},
    timeouts::KalamLinkTimeouts,
    SeqId,
};

/// Manages WebSocket subscriptions for real-time change notifications.
///
/// # Examples
///
/// ```rust,no_run
/// use kalam_client::KalamLinkClient;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let client = KalamLinkClient::builder().base_url("http://localhost:3000").build()?;
///
/// let mut subscription = client.live_events("SELECT * FROM messages").await?;
///
/// while let Some(event) = subscription.next().await {
///     match event {
///         Ok(change) => println!("Change detected: {:?}", change),
///         Err(e) => eprintln!("Error: {}", e),
///     }
/// }
/// # Ok(())
/// # }
/// ```
pub struct SubscriptionManager {
    subscription_id: String,
    /// Receives parsed events from the shared connection task.
    event_rx: mpsc::Receiver<Result<ChangeEvent>>,
    /// Sends unsubscribe and checkpoint progress back to the shared connection.
    shared_control: Option<SharedSubscriptionControl>,
    /// Generation tag assigned by the shared `connection_task`.
    generation: u64,
    /// Local event buffer for yielding batched events from a single WS message.
    event_queue: VecDeque<ChangeEvent>,
    /// Changes received while initial data is still loading.
    buffered_changes: Vec<ChangeEvent>,
    /// Whether initial data is still loading.
    is_loading: bool,
    /// Original `from` cursor used to open this subscription, if any.
    resume_from: Option<SeqId>,
    /// Highest progress delivered to the consumer, acknowledged or not.
    delivered_seq_id: Option<SeqId>,
    ack_mode: SubscriptionAckMode,
    timeouts: KalamLinkTimeouts,
    closed: bool,
}

impl SubscriptionManager {
    /// Create a `SubscriptionManager` that receives events from a
    /// [`SharedConnection`](crate::connection::SharedConnection) rather than
    /// owning its own WebSocket.
    pub(crate) fn from_shared(
        subscription_id: String,
        event_rx: mpsc::Receiver<Result<ChangeEvent>>,
        shared_control: SharedSubscriptionControl,
        generation: u64,
        resume_from: Option<SeqId>,
        ack_mode: SubscriptionAckMode,
        timeouts: &KalamLinkTimeouts,
    ) -> Self {
        Self {
            subscription_id,
            event_rx,
            shared_control: Some(shared_control),
            generation,
            event_queue: VecDeque::new(),
            buffered_changes: Vec::new(),
            is_loading: true,
            resume_from,
            delivered_seq_id: resume_from,
            ack_mode,
            timeouts: timeouts.clone(),
            closed: false,
        }
    }

    async fn report_shared_progress(&mut self, event: &ChangeEvent) {
        let Some(progress) = event_progress(event) else {
            return;
        };

        seq_tracking::advance_seq(&mut self.resume_from, progress.seq_id);

        let Some(shared_control) = self.shared_control.as_ref() else {
            return;
        };

        shared_control
            .progress(
                self.subscription_id.clone(),
                self.generation,
                progress.seq_id,
                progress.advance_resume,
            )
            .await;
    }

    fn record_delivery(&mut self, event: &ChangeEvent) {
        if let Some(progress) = event_progress(event) {
            seq_tracking::advance_seq(&mut self.delivered_seq_id, progress.seq_id);
        }
    }

    /// Buffer incoming events: hold live changes while initial data is loading,
    /// then flush them in order once the snapshot is complete.
    fn apply_buffering(&mut self, event: ChangeEvent) {
        buffer_event(
            &mut self.event_queue,
            &mut self.buffered_changes,
            &mut self.is_loading,
            self.resume_from,
            event,
        );
    }

    /// Receive the next change event from the subscription.
    ///
    /// Returns `None` when the connection is closed.
    pub async fn next(&mut self) -> Option<Result<ChangeEvent>> {
        loop {
            // 1. Drain local event queue first
            if let Some(event) = self.event_queue.pop_front() {
                self.record_delivery(&event);
                if self.ack_mode == SubscriptionAckMode::Automatic {
                    self.report_shared_progress(&event).await;
                }
                return Some(Ok(event));
            }

            // 2. If already closed, signal end-of-stream
            if self.closed {
                return None;
            }

            // 3. Read next parsed event from the shared connection task
            match self.event_rx.recv().await {
                Some(Ok(event)) => {
                    self.apply_buffering(event);
                    // Loop back to drain event_queue
                },
                Some(Err(e)) => return Some(Err(e)),
                None => {
                    self.closed = true;
                    return None;
                },
            }
        }
    }

    /// Advance explicit subscription progress after durable consumer work commits.
    pub async fn acknowledge(&mut self, seq_id: SeqId) -> Result<()> {
        if self.ack_mode != SubscriptionAckMode::Explicit {
            return Err(crate::error::KalamLinkError::ConfigurationError(
                "explicit acknowledgement is not enabled for this subscription".to_string(),
            ));
        }
        if self.delivered_seq_id.is_none_or(|delivered| seq_id > delivered) {
            return Err(crate::error::KalamLinkError::ConfigurationError(format!(
                "sequence {seq_id} was not delivered to this subscription"
            )));
        }

        seq_tracking::advance_seq(&mut self.resume_from, seq_id);
        if let Some(shared_control) = self.shared_control.as_ref() {
            shared_control
                .progress(self.subscription_id.clone(), self.generation, seq_id, true)
                .await;
        }
        Ok(())
    }

    /// Get the subscription ID assigned by the server
    pub fn subscription_id(&self) -> &str {
        &self.subscription_id
    }

    /// Get the configured timeouts
    pub fn timeouts(&self) -> &KalamLinkTimeouts {
        &self.timeouts
    }

    /// Close the subscription gracefully.
    ///
    /// Safe to call multiple times — subsequent calls are no-ops.
    pub async fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;

        if let Some(shared_control) = self.shared_control.take() {
            shared_control.unsubscribe(self.subscription_id.clone(), self.generation).await;
        }

        Ok(())
    }

    /// Returns `true` if `close()` has been called or `Drop` has run.
    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

impl Drop for SubscriptionManager {
    fn drop(&mut self) {
        if let Some(shared_control) = self.shared_control.take() {
            shared_control.try_unsubscribe(self.subscription_id.clone(), self.generation);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription::SubscriptionAckMode;

    /// Create a minimal `SubscriptionManager` with no live shared connection
    /// for testing state-flag logic without a network dependency.
    fn make_test_sub() -> SubscriptionManager {
        let (event_tx, event_rx) = mpsc::channel(1);
        drop(event_tx);

        let mut subscription = SubscriptionManager::from_shared(
            "unit-test-id".to_string(),
            event_rx,
            SharedSubscriptionControl::test_control(),
            0,
            None,
            SubscriptionAckMode::Automatic,
            &KalamLinkTimeouts::default(),
        );
        subscription.is_loading = false;
        subscription
    }

    #[tokio::test]
    async fn test_is_not_closed_initially() {
        let sub = make_test_sub();
        assert!(!sub.is_closed(), "subscription should start as open");
    }

    #[tokio::test]
    async fn test_close_marks_subscription_as_closed() {
        let mut sub = make_test_sub();
        assert!(!sub.is_closed());
        sub.close().await.expect("close should succeed on a stream-less sub");
        assert!(sub.is_closed(), "subscription should be closed after close()");
    }

    #[tokio::test]
    async fn test_close_is_idempotent() {
        let mut sub = make_test_sub();
        sub.close().await.expect("first close should succeed");
        sub.close().await.expect("second close should also succeed (no-op)");
        assert!(sub.is_closed());
    }

    #[tokio::test]
    async fn test_next_returns_none_when_stream_is_none() {
        let mut sub = make_test_sub();
        let result = tokio::time::timeout(std::time::Duration::from_millis(100), sub.next())
            .await
            .expect("next() should complete quickly when stream is None");
        assert!(result.is_none(), "next() should return None when stream is None");
    }

    #[tokio::test]
    async fn test_next_returns_none_after_close() {
        let mut sub = make_test_sub();
        sub.close().await.unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_millis(100), sub.next())
            .await
            .expect("next() should complete quickly after close");
        assert!(result.is_none());
    }

    #[test]
    fn test_drop_without_runtime_does_not_panic() {
        let sub = make_test_sub();
        drop(sub);
    }

    #[tokio::test]
    async fn test_consumed_initial_batch_advances_local_replay_filter() {
        let mut sub = make_test_sub();
        let event = ChangeEvent::InitialDataBatch {
            subscription_id: "unit-test-id".to_string(),
            rows: vec![{
                let mut row = std::collections::HashMap::new();
                row.insert("id".to_string(), crate::models::KalamCellValue::text("seed"));
                row.insert("_seq".to_string(), crate::models::KalamCellValue::text("10"));
                row
            }],
            batch_control: crate::models::BatchControl {
                batch_num: 0,
                has_more: true,
                status: crate::models::BatchStatus::Loading,
                last_seq_id: Some(SeqId::from_i64(10)),
            },
        };

        sub.report_shared_progress(&event).await;
        sub.apply_buffering(ChangeEvent::Insert {
            subscription_id: "unit-test-id".to_string(),
            rows: vec![{
                let mut row = std::collections::HashMap::new();
                row.insert("id".to_string(), crate::models::KalamCellValue::text("seed"));
                row.insert("_seq".to_string(), crate::models::KalamCellValue::text("10"));
                row
            }],
        });

        assert!(sub.event_queue.is_empty());
        assert!(sub.buffered_changes.is_empty());
    }

    #[tokio::test]
    async fn test_explicit_ack_does_not_advance_resume_before_acknowledgement() {
        let mut sub = make_test_sub();
        sub.ack_mode = SubscriptionAckMode::Explicit;
        sub.event_queue.push_back(ChangeEvent::Insert {
            subscription_id: "unit-test-id".to_string(),
            rows: vec![{
                let mut row = std::collections::HashMap::new();
                row.insert("id".to_string(), crate::models::KalamCellValue::text("one"));
                row.insert("_seq".to_string(), crate::models::KalamCellValue::text("10"));
                row
            }],
        });

        let event = sub.next().await.expect("event").expect("valid event");
        assert!(matches!(event, ChangeEvent::Insert { .. }));
        assert_eq!(sub.resume_from, None, "delivery must not acknowledge progress");

        sub.acknowledge(SeqId::from_i64(10)).await.expect("acknowledge");
        assert_eq!(sub.resume_from, Some(SeqId::from_i64(10)));
    }

    #[tokio::test]
    async fn test_explicit_ack_rejects_sequence_that_was_not_delivered() {
        let mut sub = make_test_sub();
        sub.ack_mode = SubscriptionAckMode::Explicit;

        let error = sub
            .acknowledge(SeqId::from_i64(11))
            .await
            .expect_err("undelivered sequence must fail");

        assert!(error.to_string().contains("not delivered"));
        assert_eq!(sub.resume_from, None);
    }

    #[tokio::test]
    async fn test_drop_inside_runtime_does_not_panic() {
        let sub = make_test_sub();
        drop(sub);
        tokio::task::yield_now().await;
    }
}
