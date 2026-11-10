use kalamdb_commons::{
    models::{TableId, UserId},
    TableType,
};

/// Interface for NotificationService implementations used by table providers.
///
/// Provides a generic way to dispatch notifications about data changes
/// (INSERT, UPDATE, DELETE) to subscribers.
///
/// Unified approach:
/// - User tables: call with `Some(user_id)` → notifies live queries + topics
/// - Shared tables: call with `None` → only notifies topics
///
/// This prevents duplicate notifications and provides a single code path.
pub trait NotificationService: Send + Sync {
    type Notification: Send + Sync + 'static;

    /// Check if there are any subscribers (live queries or topics) for a table.
    /// - If `user_id` is provided: checks both user-scoped live queries AND table-scoped topics
    /// - If `user_id` is None: checks only table-scoped topics
    /// Returns true if at least one subscriber exists.
    fn has_subscribers(&self, user_id: Option<&UserId>, table_id: &TableId) -> bool;

    /// Notify subscribers about a data change (fire-and-forget async).
    ///
    /// Unified notification method that handles both:
    /// - Live query notifications (if `user_id` is provided)
    /// - Topic/CDC notifications (if topics are configured for this table)
    ///
    /// # Arguments
    /// * `user_id` - Optional user ID for user-scoped live query notifications
    /// * `table_id` - Table identifier
    /// * `notification` - Change notification data
    fn notify_table_change(
        &self,
        user_id: Option<UserId>,
        table_id: TableId,
        notification: Self::Notification,
    );
}

/// How a live reader consumes history after a cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveReadContract {
    /// Replay every promised change after the cursor, in domain order.
    CompleteEvents,
    /// Read the latest row versions after the cursor, including tombstones.
    LatestStateSinceCursor,
}

/// Resume decision for one live history contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveHistoryDecision {
    Ready(LiveReadContract),
    /// Expired, overloaded, or incompatible history. The caller resynchronizes.
    Gap {
        contract: LiveReadContract,
        reason: &'static str,
    },
}

/// Subscriber scope for a committed table change.
///
/// User and stream commits stay on the owning user. Shared and system commits
/// use the table-wide subscription set.
pub fn live_notify_scope(table_type: TableType, owner: Option<&UserId>) -> Option<UserId> {
    match table_type {
        TableType::User | TableType::Stream => owner.cloned(),
        TableType::Shared | TableType::System => None,
    }
}

/// Publish one committed change to live subscribers.
///
/// `None` skips the send when the process has no notification service yet.
/// The service records replay history inside `notify_table_change`.
pub fn publish_live_table_change<S>(
    service: Option<&S>,
    table_type: TableType,
    owner: Option<&UserId>,
    table_id: &TableId,
    notification: S::Notification,
) where
    S: NotificationService + ?Sized,
{
    let Some(service) = service else {
        return;
    };
    let scope = live_notify_scope(table_type, owner);
    service.notify_table_change(scope, table_id.clone(), notification);
}

/// Separate complete-event replay from a latest-state since-cursor read.
///
/// An expired complete-event cursor is a gap. It is not a silent snapshot.
/// An expired latest-state cursor is a different gap and does not replay events.
pub fn classify_live_history(contract: LiveReadContract, expired: bool) -> LiveHistoryDecision {
    if !expired {
        return LiveHistoryDecision::Ready(contract);
    }
    let reason = match contract {
        LiveReadContract::CompleteEvents => "complete-event history expired; resynchronize",
        LiveReadContract::LatestStateSinceCursor => "latest-state cursor expired; resynchronize",
    };
    LiveHistoryDecision::Gap { contract, reason }
}

/// Gap text when history cannot be resumed. `None` when the contract is ready.
pub fn live_history_gap_message(decision: &LiveHistoryDecision) -> Option<&'static str> {
    match decision {
        LiveHistoryDecision::Gap { reason, .. } => Some(*reason),
        LiveHistoryDecision::Ready(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct RecordingService {
        notified: Mutex<Vec<TableId>>,
    }

    impl NotificationService for RecordingService {
        type Notification = &'static str;

        fn has_subscribers(&self, _user_id: Option<&UserId>, _table_id: &TableId) -> bool {
            true
        }

        fn notify_table_change(
            &self,
            user_id: Option<UserId>,
            table_id: TableId,
            _notification: Self::Notification,
        ) {
            assert!(user_id.is_none());
            self.notified.lock().expect("notifier lock").push(table_id);
        }
    }

    #[test]
    fn publish_live_table_change_notifies_system_without_a_user_scope() {
        let service = RecordingService {
            notified: Mutex::new(Vec::new()),
        };
        let table_id = TableId::new(
            kalamdb_commons::SchemaId::new("system"),
            kalamdb_commons::TableName::new("users"),
        );
        publish_live_table_change(
            Some(&service),
            TableType::System,
            Some(&UserId::new("alice")),
            &table_id,
            "created",
        );
        let notified = service.notified.lock().expect("notifier lock");
        assert_eq!(notified.as_slice(), &[table_id]);
    }

    #[test]
    fn expired_complete_events_stay_distinct_from_latest_state() {
        let expired = classify_live_history(LiveReadContract::CompleteEvents, true);
        let latest = classify_live_history(LiveReadContract::LatestStateSinceCursor, false);
        let latest_expired = classify_live_history(LiveReadContract::LatestStateSinceCursor, true);
        assert_eq!(
            live_history_gap_message(&expired),
            Some("complete-event history expired; resynchronize")
        );
        assert_eq!(latest, LiveHistoryDecision::Ready(LiveReadContract::LatestStateSinceCursor));
        assert_eq!(
            live_history_gap_message(&latest_expired),
            Some("latest-state cursor expired; resynchronize")
        );
        assert_ne!(live_history_gap_message(&expired), live_history_gap_message(&latest_expired));
    }
}
