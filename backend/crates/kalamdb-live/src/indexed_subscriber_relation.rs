use std::{collections::HashSet, sync::Arc};

use dashmap::DashMap;
use datafusion_common::ScalarValue;
use kalamdb_commons::models::{rows::Row, LiveQueryId, TableId};

use crate::models::{LiveRoute, SubscriptionHandle};

type SubscriptionBucket = Arc<DashMap<LiveQueryId, ()>>;
type ValueBuckets = DashMap<ScalarValue, SubscriptionBucket>;
type ColumnBuckets = DashMap<Arc<str>, Arc<ValueBuckets>>;

struct IndexedSubscriber {
    table_id: TableId,
    route:    LiveRoute,
    handle:   SubscriptionHandle,
}

/// In-memory shared-table subscriber relation with exact typed-key lookup.
///
/// The relation produces candidates only. Notification delivery must still run the bound RLS
/// evaluator so stale policy or membership generations fail closed.
#[derive(Default)]
pub(crate) struct IndexedSubscriberRelation {
    subscribers: DashMap<LiveQueryId, IndexedSubscriber>,
    broadcast:   DashMap<TableId, SubscriptionBucket>,
    keyed:       DashMap<TableId, Arc<ColumnBuckets>>,
}

impl IndexedSubscriberRelation {
    pub fn index(
        &self,
        table_id: TableId,
        live_id: LiveQueryId,
        route: &LiveRoute,
        handle: SubscriptionHandle,
    ) {
        self.unindex(&live_id);
        if matches!(route, LiveRoute::Deny) {
            return;
        }

        self.subscribers.insert(
            live_id.clone(),
            IndexedSubscriber {
                table_id: table_id.clone(),
                route: route.clone(),
                handle,
            },
        );
        match route {
            LiveRoute::Broadcast => {
                self.broadcast.entry(table_id).or_default().insert(live_id, ());
            },
            LiveRoute::Deny => {},
            LiveRoute::Keyed(keys) => {
                let columns = self.keyed.entry(table_id).or_default().clone();
                for (column, value) in keys.iter() {
                    let values = columns.entry(Arc::clone(column)).or_default().clone();
                    let subscriptions = values.entry(value.clone()).or_default().clone();
                    subscriptions.insert(live_id.clone(), ());
                }
            },
        }
    }

    pub fn unindex(&self, live_id: &LiveQueryId) {
        let Some((_, subscriber)) = self.subscribers.remove(live_id) else {
            return;
        };
        match &subscriber.route {
            LiveRoute::Broadcast => {
                if let Some(bucket) = self.broadcast.get(&subscriber.table_id) {
                    bucket.remove(live_id);
                    let is_empty = bucket.is_empty();
                    drop(bucket);
                    if is_empty {
                        self.broadcast.remove_if(&subscriber.table_id, |_, value| value.is_empty());
                    }
                }
            },
            LiveRoute::Deny => {},
            LiveRoute::Keyed(keys) => {
                if let Some(columns) =
                    self.keyed.get(&subscriber.table_id).map(|entry| Arc::clone(entry.value()))
                {
                    for (column, value) in keys.iter() {
                        let Some(values) =
                            columns.get(column.as_ref()).map(|entry| Arc::clone(entry.value()))
                        else {
                            continue;
                        };
                        if let Some(subscriptions) = values.get(value) {
                            subscriptions.remove(live_id);
                            let is_empty = subscriptions.is_empty();
                            drop(subscriptions);
                            if is_empty {
                                values.remove_if(value, |_, bucket| bucket.is_empty());
                            }
                        }
                        if values.is_empty() {
                            columns.remove_if(column.as_ref(), |_, buckets| buckets.is_empty());
                        }
                    }
                    if columns.is_empty() {
                        self.keyed.remove_if(&subscriber.table_id, |_, buckets| buckets.is_empty());
                    }
                }
            },
        }
        if self.subscribers.is_empty() {
            self.shrink_to_fit();
        }
    }

    pub fn candidates(
        &self,
        table_id: &TableId,
        new_row: &Row,
        old_row: Option<&Row>,
    ) -> Vec<SubscriptionHandle> {
        let mut candidates = HashSet::new();
        if let Some(bucket) = self.broadcast.get(table_id) {
            extend_candidates(&mut candidates, bucket.value());
        }
        let Some(columns) = self.keyed.get(table_id).map(|entry| Arc::clone(entry.value())) else {
            return self.resolve_candidates(candidates);
        };

        extend_row_candidates(&mut candidates, &columns, new_row);
        if let Some(old_row) = old_row {
            extend_row_candidates(&mut candidates, &columns, old_row);
        }
        self.resolve_candidates(candidates)
    }

    pub fn has_subscriptions(&self, table_id: &TableId) -> bool {
        self.broadcast.contains_key(table_id) || self.keyed.contains_key(table_id)
    }

    pub fn clear(&self) {
        self.subscribers.clear();
        self.broadcast.clear();
        self.keyed.clear();
    }

    pub fn shrink_to_fit(&self) {
        self.subscribers.shrink_to_fit();
        self.broadcast.shrink_to_fit();
        self.keyed.shrink_to_fit();
    }

    fn resolve_candidates(&self, candidates: HashSet<LiveQueryId>) -> Vec<SubscriptionHandle> {
        let mut handles = Vec::with_capacity(candidates.len());
        for live_id in candidates {
            if let Some(entry) = self.subscribers.get(&live_id) {
                handles.push(entry.value().handle.clone());
            }
        }
        handles
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.subscribers.is_empty() && self.broadcast.is_empty() && self.keyed.is_empty()
    }

    #[cfg(test)]
    fn keyed_subscriber_count(
        &self,
        table_id: &TableId,
        column: &str,
        value: &ScalarValue,
    ) -> usize {
        self.keyed
            .get(table_id)
            .and_then(|columns| columns.get(column).map(|entry| Arc::clone(entry.value())))
            .and_then(|values| values.get(value).map(|bucket| bucket.len()))
            .unwrap_or(0)
    }
}

fn extend_row_candidates(
    candidates: &mut HashSet<LiveQueryId>,
    columns: &ColumnBuckets,
    row: &Row,
) {
    for column_entry in columns.iter() {
        let Some(value) = row.values.get(column_entry.key().as_ref()) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        let Some(bucket) = column_entry.value().get(value) else {
            continue;
        };
        extend_candidates(candidates, bucket.value());
    }
}

fn extend_candidates(candidates: &mut HashSet<LiveQueryId>, bucket: &DashMap<LiveQueryId, ()>) {
    candidates.extend(bucket.iter().map(|entry| entry.key().clone()));
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, HashSet},
        sync::Arc,
    };

    use datafusion_common::ScalarValue;
    use kalamdb_commons::models::{rows::Row, ConnectionId, LiveQueryId, TableId, UserId};
    use tokio::sync::mpsc;

    use super::IndexedSubscriberRelation;
    use crate::models::{LiveRoute, SubscriptionHandle, SubscriptionRuntimeMetadata};

    fn handle(subscription_id: &str) -> SubscriptionHandle {
        let (notification_tx, _notification_rx) = mpsc::channel(8);
        SubscriptionHandle {
            subscription_id: Arc::from(subscription_id),
            filter_expr: None,
            authorization: None,
            projections: None,
            notification_tx,
            flow_control: None,
            runtime_metadata: Arc::new(SubscriptionRuntimeMetadata::new(
                "SELECT * FROM chat.conversations",
                None,
                1,
            )),
        }
    }

    fn live_id(user: &str, subscription: &str) -> LiveQueryId {
        LiveQueryId::new(
            UserId::new(user),
            ConnectionId::new(format!("connection-{user}")),
            subscription.to_string(),
        )
    }

    fn keyed(conversation_id: &str) -> LiveRoute {
        LiveRoute::Keyed(Arc::new(HashSet::from([(
            Arc::from("conversation_id"),
            ScalarValue::Utf8(Some(conversation_id.to_string())),
        )])))
    }

    fn row(conversation_id: &str) -> Row {
        Row::new(BTreeMap::from([
            (
                "conversation_id".to_string(),
                ScalarValue::Utf8(Some(conversation_id.to_string())),
            ),
            ("body".to_string(), ScalarValue::Utf8(Some("unused-wide-column".to_string()))),
        ]))
    }

    #[test]
    fn keyed_lookup_returns_only_matching_conversation_subscribers() {
        let relation = IndexedSubscriberRelation::default();
        let table_id = TableId::from_strings("chat", "conversations");
        let alice = live_id("alice", "alice-sub");
        let bob = live_id("bob", "bob-sub");

        relation.index(table_id.clone(), alice, &keyed("conv-123"), handle("alice-sub"));
        relation.index(table_id.clone(), bob, &keyed("conv-456"), handle("bob-sub"));

        let candidates = relation.candidates(&table_id, &row("conv-123"), None);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].subscription_id.as_ref(), "alice-sub");
    }

    #[test]
    fn deny_routes_are_not_stored_or_returned() {
        let relation = IndexedSubscriberRelation::default();
        let table_id = TableId::from_strings("chat", "conversations");
        relation.index(
            table_id.clone(),
            live_id("alice", "denied"),
            &LiveRoute::Deny,
            handle("denied"),
        );

        assert!(!relation.has_subscriptions(&table_id));
        assert!(relation.candidates(&table_id, &row("conv-123"), None).is_empty());
    }

    #[test]
    fn update_routes_to_both_old_and_new_keys_without_duplicates() {
        let relation = IndexedSubscriberRelation::default();
        let table_id = TableId::from_strings("chat", "conversations");
        let alice = live_id("alice", "alice-sub");
        let bob = live_id("bob", "bob-sub");

        relation.index(
            table_id.clone(),
            alice.clone(),
            &LiveRoute::Keyed(Arc::new(HashSet::from([
                (Arc::from("conversation_id"), ScalarValue::Utf8(Some("conv-old".to_string()))),
                (Arc::from("conversation_id"), ScalarValue::Utf8(Some("conv-new".to_string()))),
            ]))),
            handle("alice-sub"),
        );
        relation.index(table_id.clone(), bob, &LiveRoute::Broadcast, handle("bob-sub"));

        let candidates = relation.candidates(&table_id, &row("conv-new"), Some(&row("conv-old")));

        assert_eq!(candidates.len(), 2);
        relation.unindex(&alice);
        let remaining = relation.candidates(&table_id, &row("conv-new"), None);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].subscription_id.as_ref(), "bob-sub");
    }

    #[test]
    fn last_keyed_unsubscribe_drops_conversation_buckets_and_table_maps() {
        let relation = IndexedSubscriberRelation::default();
        let table_id = TableId::from_strings("chat", "messages");
        let alice = live_id("alice", "alice-sub");
        let bob = live_id("bob", "bob-sub");
        let conv_123 = ScalarValue::Utf8(Some("conv-123".to_string()));
        let conv_456 = ScalarValue::Utf8(Some("conv-456".to_string()));

        relation.index(table_id.clone(), alice.clone(), &keyed("conv-123"), handle("alice-sub"));
        relation.index(table_id.clone(), bob.clone(), &keyed("conv-456"), handle("bob-sub"));
        assert_eq!(relation.keyed_subscriber_count(&table_id, "conversation_id", &conv_123), 1);
        assert_eq!(relation.keyed_subscriber_count(&table_id, "conversation_id", &conv_456), 1);

        relation.unindex(&alice);
        assert_eq!(relation.keyed_subscriber_count(&table_id, "conversation_id", &conv_123), 0);
        assert_eq!(relation.keyed_subscriber_count(&table_id, "conversation_id", &conv_456), 1);
        assert!(relation.has_subscriptions(&table_id));
        assert_eq!(
            relation.candidates(&table_id, &row("conv-123"), None).len(),
            0,
            "unsubscribed conversation must not keep a lookup bucket"
        );

        relation.unindex(&bob);
        assert!(
            relation.is_empty(),
            "last unsubscribe must drop keyed DashMaps, not leave empty table entries"
        );
        assert!(!relation.has_subscriptions(&table_id));
    }

    #[test]
    fn last_broadcast_unsubscribe_drops_table_bucket() {
        let relation = IndexedSubscriberRelation::default();
        let table_id = TableId::from_strings("chat", "messages");
        let alice = live_id("alice", "alice-sub");
        relation.index(table_id.clone(), alice.clone(), &LiveRoute::Broadcast, handle("alice-sub"));
        relation.unindex(&alice);
        assert!(relation.is_empty());
        assert!(!relation.has_subscriptions(&table_id));
    }

    #[test]
    fn reindex_after_unsubscribe_does_not_leave_stale_conversation_keys() {
        let relation = IndexedSubscriberRelation::default();
        let table_id = TableId::from_strings("chat", "messages");
        let alice = live_id("alice", "alice-sub");
        relation.index(table_id.clone(), alice.clone(), &keyed("conv-123"), handle("alice-sub"));
        relation.index(table_id.clone(), alice.clone(), &keyed("conv-456"), handle("alice-sub"));

        assert_eq!(
            relation.keyed_subscriber_count(
                &table_id,
                "conversation_id",
                &ScalarValue::Utf8(Some("conv-123".to_string()))
            ),
            0
        );
        assert_eq!(
            relation.keyed_subscriber_count(
                &table_id,
                "conversation_id",
                &ScalarValue::Utf8(Some("conv-456".to_string()))
            ),
            1
        );

        relation.unindex(&alice);
        assert!(relation.is_empty());
    }
}
