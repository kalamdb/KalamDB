use std::sync::{
    atomic::{AtomicU64, Ordering},
    OnceLock,
};

const RATE_WINDOW_SECS: u64 = 60;
const RATE_BUCKET_COUNT: usize = RATE_WINDOW_SECS as usize;

static PUBSUB_MESSAGES_PUBLISHED_TOTAL: AtomicU64 = AtomicU64::new(0);
static PUBSUB_BYTES_PUBLISHED_TOTAL: AtomicU64 = AtomicU64::new(0);
static PUBSUB_MESSAGES_CONSUMED_TOTAL: AtomicU64 = AtomicU64::new(0);
static PUBSUB_BYTES_CONSUMED_TOTAL: AtomicU64 = AtomicU64::new(0);
static PUBSUB_ACTIVE_CONSUMERS: AtomicU64 = AtomicU64::new(0);
static PUBSUB_ACTIVE_CONSUMERS_PEAK: AtomicU64 = AtomicU64::new(0);
static PUBSUB_MESSAGES_PUBLISHED_PEAK_PER_SECOND: AtomicU64 = AtomicU64::new(0);
static PUBSUB_MESSAGES_CONSUMED_PEAK_PER_SECOND: AtomicU64 = AtomicU64::new(0);
static SUBSCRIPTION_CHANGES_DELIVERED_TOTAL: AtomicU64 = AtomicU64::new(0);
static FIRST_PUBSUB_EPOCH_SECOND: AtomicU64 = AtomicU64::new(0);

struct RateBuckets {
    epochs: [AtomicU64; RATE_BUCKET_COUNT],
    amounts: [AtomicU64; RATE_BUCKET_COUNT],
}

impl RateBuckets {
    fn new() -> Self {
        Self {
            epochs: std::array::from_fn(|_| AtomicU64::new(0)),
            amounts: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

static PUBSUB_PUBLISH_MESSAGE_RATE_BUCKETS: OnceLock<RateBuckets> = OnceLock::new();
static PUBSUB_PUBLISH_BYTE_RATE_BUCKETS: OnceLock<RateBuckets> = OnceLock::new();
static PUBSUB_CONSUME_MESSAGE_RATE_BUCKETS: OnceLock<RateBuckets> = OnceLock::new();
static PUBSUB_CONSUME_BYTE_RATE_BUCKETS: OnceLock<RateBuckets> = OnceLock::new();
static SUBSCRIPTION_DELIVERY_RATE_BUCKETS: OnceLock<RateBuckets> = OnceLock::new();

fn publish_message_rate_buckets() -> &'static RateBuckets {
    PUBSUB_PUBLISH_MESSAGE_RATE_BUCKETS.get_or_init(RateBuckets::new)
}

fn publish_byte_rate_buckets() -> &'static RateBuckets {
    PUBSUB_PUBLISH_BYTE_RATE_BUCKETS.get_or_init(RateBuckets::new)
}

fn consume_message_rate_buckets() -> &'static RateBuckets {
    PUBSUB_CONSUME_MESSAGE_RATE_BUCKETS.get_or_init(RateBuckets::new)
}

fn consume_byte_rate_buckets() -> &'static RateBuckets {
    PUBSUB_CONSUME_BYTE_RATE_BUCKETS.get_or_init(RateBuckets::new)
}

fn subscription_delivery_rate_buckets() -> &'static RateBuckets {
    SUBSCRIPTION_DELIVERY_RATE_BUCKETS.get_or_init(RateBuckets::new)
}

fn epoch_second() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn mark_first_pubsub_activity(now_second: u64) {
    let _ = FIRST_PUBSUB_EPOCH_SECOND.compare_exchange(
        0,
        now_second,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

fn observe_rate_amount(buckets: &RateBuckets, now_second: u64, amount: u64) -> u64 {
    if amount == 0 {
        return 0;
    }

    let index = (now_second % RATE_WINDOW_SECS) as usize;
    let bucket_epoch = &buckets.epochs[index];
    let bucket_amount = &buckets.amounts[index];
    let observed_epoch = bucket_epoch.load(Ordering::Acquire);
    if observed_epoch != now_second
        && bucket_epoch
            .compare_exchange(observed_epoch, now_second, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        bucket_amount.store(0, Ordering::Release);
    }

    bucket_amount.fetch_add(amount, Ordering::Relaxed).saturating_add(amount)
}

fn snapshot_rate(buckets: &RateBuckets, now_second: u64, elapsed_window: u64) -> f64 {
    let mut recent_amount = 0u64;
    for index in 0..RATE_BUCKET_COUNT {
        let bucket_epoch = buckets.epochs[index].load(Ordering::Acquire);
        if bucket_epoch <= now_second && now_second.saturating_sub(bucket_epoch) < RATE_WINDOW_SECS
        {
            recent_amount =
                recent_amount.saturating_add(buckets.amounts[index].load(Ordering::Relaxed));
        }
    }

    recent_amount as f64 / elapsed_window as f64
}

fn pubsub_elapsed_window(now_second: u64) -> u64 {
    let first_second = FIRST_PUBSUB_EPOCH_SECOND.load(Ordering::Acquire);
    if first_second == 0 {
        1
    } else {
        now_second
            .saturating_sub(first_second)
            .saturating_add(1)
            .min(RATE_WINDOW_SECS)
            .max(1)
    }
}

fn update_peak(peak: &AtomicU64, candidate: u64) {
    let mut current = peak.load(Ordering::Relaxed);
    while candidate > current {
        match peak.compare_exchange(current, candidate, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

#[derive(Debug)]
pub struct PubSubConsumerGuard;

impl Drop for PubSubConsumerGuard {
    fn drop(&mut self) {
        PUBSUB_ACTIVE_CONSUMERS.fetch_sub(1, Ordering::Relaxed);
    }
}

pub fn track_pubsub_consumer() -> PubSubConsumerGuard {
    let active = PUBSUB_ACTIVE_CONSUMERS.fetch_add(1, Ordering::Relaxed).saturating_add(1);
    update_peak(&PUBSUB_ACTIVE_CONSUMERS_PEAK, active);
    PubSubConsumerGuard
}

pub fn record_pubsub_messages_published(message_count: u64, byte_count: u64) {
    if message_count == 0 && byte_count == 0 {
        return;
    }

    let now_second = epoch_second();
    mark_first_pubsub_activity(now_second);

    if message_count > 0 {
        PUBSUB_MESSAGES_PUBLISHED_TOTAL.fetch_add(message_count, Ordering::Relaxed);
        let bucket_total =
            observe_rate_amount(publish_message_rate_buckets(), now_second, message_count);
        update_peak(&PUBSUB_MESSAGES_PUBLISHED_PEAK_PER_SECOND, bucket_total);
    }
    if byte_count > 0 {
        PUBSUB_BYTES_PUBLISHED_TOTAL.fetch_add(byte_count, Ordering::Relaxed);
        observe_rate_amount(publish_byte_rate_buckets(), now_second, byte_count);
    }
}

pub fn record_pubsub_messages_consumed(message_count: u64, byte_count: u64) {
    if message_count == 0 && byte_count == 0 {
        return;
    }

    let now_second = epoch_second();
    mark_first_pubsub_activity(now_second);

    if message_count > 0 {
        PUBSUB_MESSAGES_CONSUMED_TOTAL.fetch_add(message_count, Ordering::Relaxed);
        let bucket_total =
            observe_rate_amount(consume_message_rate_buckets(), now_second, message_count);
        update_peak(&PUBSUB_MESSAGES_CONSUMED_PEAK_PER_SECOND, bucket_total);
    }
    if byte_count > 0 {
        PUBSUB_BYTES_CONSUMED_TOTAL.fetch_add(byte_count, Ordering::Relaxed);
        observe_rate_amount(consume_byte_rate_buckets(), now_second, byte_count);
    }
}

pub fn record_subscription_changes_delivered(change_count: u64) {
    if change_count == 0 {
        return;
    }

    let now_second = epoch_second();
    mark_first_pubsub_activity(now_second);
    SUBSCRIPTION_CHANGES_DELIVERED_TOTAL.fetch_add(change_count, Ordering::Relaxed);
    observe_rate_amount(subscription_delivery_rate_buckets(), now_second, change_count);
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PubSubMetricsSnapshot {
    pub pubsub_messages_published_total: u64,
    pub pubsub_messages_published_per_second: f64,
    pub pubsub_messages_published_peak_per_second: u64,
    pub pubsub_bytes_published_total: u64,
    pub pubsub_kb_published_per_second: f64,
    pub pubsub_messages_consumed_total: u64,
    pub pubsub_messages_consumed_per_second: f64,
    pub pubsub_messages_consumed_peak_per_second: u64,
    pub pubsub_bytes_consumed_total: u64,
    pub pubsub_kb_consumed_per_second: f64,
    pub pubsub_active_consumers: u64,
    pub pubsub_active_consumers_peak: u64,
    pub subscription_changes_delivered_total: u64,
    pub subscription_changes_delivered_per_second: f64,
}

impl PubSubMetricsSnapshot {
    pub fn as_pairs(&self) -> Vec<(String, String)> {
        vec![
            (
                "pubsub_messages_published_total".to_string(),
                self.pubsub_messages_published_total.to_string(),
            ),
            (
                "pubsub_messages_published_per_second".to_string(),
                format!("{:.2}", self.pubsub_messages_published_per_second),
            ),
            (
                "pubsub_messages_published_peak_per_second".to_string(),
                self.pubsub_messages_published_peak_per_second.to_string(),
            ),
            (
                "pubsub_bytes_published_total".to_string(),
                self.pubsub_bytes_published_total.to_string(),
            ),
            (
                "pubsub_kb_published_per_second".to_string(),
                format!("{:.2}", self.pubsub_kb_published_per_second),
            ),
            (
                "pubsub_messages_consumed_total".to_string(),
                self.pubsub_messages_consumed_total.to_string(),
            ),
            (
                "pubsub_messages_consumed_per_second".to_string(),
                format!("{:.2}", self.pubsub_messages_consumed_per_second),
            ),
            (
                "pubsub_messages_consumed_peak_per_second".to_string(),
                self.pubsub_messages_consumed_peak_per_second.to_string(),
            ),
            (
                "pubsub_bytes_consumed_total".to_string(),
                self.pubsub_bytes_consumed_total.to_string(),
            ),
            (
                "pubsub_kb_consumed_per_second".to_string(),
                format!("{:.2}", self.pubsub_kb_consumed_per_second),
            ),
            ("pubsub_active_consumers".to_string(), self.pubsub_active_consumers.to_string()),
            (
                "pubsub_active_consumers_peak".to_string(),
                self.pubsub_active_consumers_peak.to_string(),
            ),
            (
                "subscription_changes_delivered_total".to_string(),
                self.subscription_changes_delivered_total.to_string(),
            ),
            (
                "subscription_changes_delivered_per_second".to_string(),
                format!("{:.2}", self.subscription_changes_delivered_per_second),
            ),
        ]
    }
}

pub fn pubsub_metrics_snapshot() -> PubSubMetricsSnapshot {
    let now_second = epoch_second();
    let elapsed_window = pubsub_elapsed_window(now_second);

    PubSubMetricsSnapshot {
        pubsub_messages_published_total: PUBSUB_MESSAGES_PUBLISHED_TOTAL.load(Ordering::Relaxed),
        pubsub_messages_published_per_second: snapshot_rate(
            publish_message_rate_buckets(),
            now_second,
            elapsed_window,
        ),
        pubsub_messages_published_peak_per_second: PUBSUB_MESSAGES_PUBLISHED_PEAK_PER_SECOND
            .load(Ordering::Relaxed),
        pubsub_bytes_published_total: PUBSUB_BYTES_PUBLISHED_TOTAL.load(Ordering::Relaxed),
        pubsub_kb_published_per_second: snapshot_rate(
            publish_byte_rate_buckets(),
            now_second,
            elapsed_window,
        ) / 1024.0,
        pubsub_messages_consumed_total: PUBSUB_MESSAGES_CONSUMED_TOTAL.load(Ordering::Relaxed),
        pubsub_messages_consumed_per_second: snapshot_rate(
            consume_message_rate_buckets(),
            now_second,
            elapsed_window,
        ),
        pubsub_messages_consumed_peak_per_second: PUBSUB_MESSAGES_CONSUMED_PEAK_PER_SECOND
            .load(Ordering::Relaxed),
        pubsub_bytes_consumed_total: PUBSUB_BYTES_CONSUMED_TOTAL.load(Ordering::Relaxed),
        pubsub_kb_consumed_per_second: snapshot_rate(
            consume_byte_rate_buckets(),
            now_second,
            elapsed_window,
        ) / 1024.0,
        pubsub_active_consumers: PUBSUB_ACTIVE_CONSUMERS.load(Ordering::Relaxed),
        pubsub_active_consumers_peak: PUBSUB_ACTIVE_CONSUMERS_PEAK.load(Ordering::Relaxed),
        subscription_changes_delivered_total: SUBSCRIPTION_CHANGES_DELIVERED_TOTAL
            .load(Ordering::Relaxed),
        subscription_changes_delivered_per_second: snapshot_rate(
            subscription_delivery_rate_buckets(),
            now_second,
            elapsed_window,
        ),
    }
}

#[cfg(test)]
fn reset_pubsub_metrics_for_tests() {
    PUBSUB_MESSAGES_PUBLISHED_TOTAL.store(0, Ordering::Relaxed);
    PUBSUB_BYTES_PUBLISHED_TOTAL.store(0, Ordering::Relaxed);
    PUBSUB_MESSAGES_CONSUMED_TOTAL.store(0, Ordering::Relaxed);
    PUBSUB_BYTES_CONSUMED_TOTAL.store(0, Ordering::Relaxed);
    PUBSUB_ACTIVE_CONSUMERS.store(0, Ordering::Relaxed);
    PUBSUB_ACTIVE_CONSUMERS_PEAK.store(0, Ordering::Relaxed);
    PUBSUB_MESSAGES_PUBLISHED_PEAK_PER_SECOND.store(0, Ordering::Relaxed);
    PUBSUB_MESSAGES_CONSUMED_PEAK_PER_SECOND.store(0, Ordering::Relaxed);
    SUBSCRIPTION_CHANGES_DELIVERED_TOTAL.store(0, Ordering::Relaxed);
    FIRST_PUBSUB_EPOCH_SECOND.store(0, Ordering::Relaxed);

    for buckets in [
        publish_message_rate_buckets(),
        publish_byte_rate_buckets(),
        consume_message_rate_buckets(),
        consume_byte_rate_buckets(),
        subscription_delivery_rate_buckets(),
    ] {
        for index in 0..RATE_BUCKET_COUNT {
            buckets.epochs[index].store(0, Ordering::Relaxed);
            buckets.amounts[index].store(0, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pubsub_metrics_snapshot_tracks_counts_rates_and_active_consumers() {
        reset_pubsub_metrics_for_tests();

        {
            let _consumer_guard = track_pubsub_consumer();
            record_pubsub_messages_published(3, 3_072);
            record_pubsub_messages_consumed(2, 1_024);
            record_subscription_changes_delivered(5);

            let snapshot = pubsub_metrics_snapshot();
            assert_eq!(snapshot.pubsub_active_consumers, 1);
            assert_eq!(snapshot.pubsub_active_consumers_peak, 1);
            assert_eq!(snapshot.pubsub_messages_published_total, 3);
            assert_eq!(snapshot.pubsub_bytes_published_total, 3_072);
            assert_eq!(snapshot.pubsub_messages_consumed_total, 2);
            assert_eq!(snapshot.pubsub_bytes_consumed_total, 1_024);
            assert_eq!(snapshot.pubsub_messages_published_peak_per_second, 3);
            assert_eq!(snapshot.pubsub_messages_consumed_peak_per_second, 2);
            assert_eq!(snapshot.subscription_changes_delivered_total, 5);
            assert!(snapshot.pubsub_messages_published_per_second > 0.0);
            assert!(snapshot.pubsub_messages_consumed_per_second > 0.0);
            assert!(snapshot.pubsub_kb_published_per_second > 0.0);
            assert!(snapshot.pubsub_kb_consumed_per_second > 0.0);
            assert!(snapshot.subscription_changes_delivered_per_second > 0.0);
        }

        assert_eq!(pubsub_metrics_snapshot().pubsub_active_consumers, 0);
    }
}
