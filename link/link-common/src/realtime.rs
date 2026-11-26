//! Transport-independent retry and subscription recovery policy.

use std::time::Duration;

use crate::{
    subscription::{event_progress, subscription_start_ready},
    ChangeEvent, ConnectionOptions, KalamLinkTimeouts, SubscriptionOptions, VersionId,
};

/// Retry budget for one outage. Reset only after the subscription set recovers.
#[derive(Default)]
pub struct RetryBackoff {
    attempts: u32,
}

impl RetryBackoff {
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    pub fn next_delay(&mut self, options: &ConnectionOptions) -> Option<Duration> {
        if !options.auto_reconnect
            || options.max_reconnect_attempts.is_some_and(|max| self.attempts >= max)
        {
            return None;
        }

        let delay = options
            .reconnect_delay_ms
            .saturating_mul(2_u64.saturating_pow(self.attempts))
            .min(options.max_reconnect_delay_ms);
        self.attempts = self.attempts.saturating_add(1);
        Some(Duration::from_millis(delay))
    }

    pub fn reset(&mut self) {
        self.attempts = 0;
    }
}

/// Set the replay boundary to the newest consumed or explicitly requested version.
pub fn resume_from(options: &mut SubscriptionOptions, consumed: Option<VersionId>) {
    options.from = options.from.max(consumed);
}

/// A server acknowledgement or ready batch completes a resume request.
pub fn resume_acknowledged(event: &ChangeEvent) -> bool {
    matches!(event, ChangeEvent::Ack { .. }) || subscription_start_ready(event)
}

/// The resume cursor advances on delivered rows, including snapshot batches.
pub fn delivered_version(event: &ChangeEvent) -> Option<VersionId> {
    event_progress(event).map(|progress| progress.seq_id)
}

/// Give a resumed subscription a bounded window to acknowledge its new request.
pub fn resume_timeout(timeouts: &KalamLinkTimeouts) -> Option<Duration> {
    let initial = timeouts.initial_data_timeout;
    if KalamLinkTimeouts::is_no_timeout(initial) {
        return None;
    }
    let subscribe = timeouts.subscribe_timeout;
    Some(if KalamLinkTimeouts::is_no_timeout(subscribe) {
        initial
    } else {
        initial.min(subscribe)
    })
}

/// Recovery requested by the server for an already registered subscription.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubscriptionRecovery {
    Replay,
    Snapshot,
}

impl SubscriptionRecovery {
    pub fn from_error(code: &str, message: &str) -> Option<Self> {
        let message = message.to_ascii_lowercase();
        if code.eq_ignore_ascii_case("CURSOR_EXPIRED")
            || message.contains("stale resume cursor")
            || message.contains("complete-event history expired")
            || message.contains("latest-state cursor expired")
        {
            Some(Self::Snapshot)
        } else if code.eq_ignore_ascii_case("DELIVERY_GAP") {
            Some(Self::Replay)
        } else {
            None
        }
    }

    pub fn apply(self, options: &mut SubscriptionOptions, consumed: &mut Option<VersionId>) {
        match self {
            Self::Replay => resume_from(options, *consumed),
            Self::Snapshot => {
                options.from = None;
                options.version_domain = None;
                *consumed = None;
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_budget_caps_delays_and_resets_after_recovery() {
        let mut options = ConnectionOptions::default()
            .with_reconnect_delay_ms(10)
            .with_max_reconnect_delay_ms(15)
            .with_max_reconnect_attempts(Some(3));
        let mut retry = RetryBackoff::default();
        assert_eq!(retry.next_delay(&options), Some(Duration::from_millis(10)));
        assert_eq!(retry.next_delay(&options), Some(Duration::from_millis(15)));
        assert_eq!(retry.next_delay(&options), Some(Duration::from_millis(15)));
        assert_eq!(retry.next_delay(&options), None);
        assert_eq!(retry.attempts(), 3);
        retry.reset();
        assert_eq!(retry.next_delay(&options), Some(Duration::from_millis(10)));
        options.auto_reconnect = false;
        assert_eq!(retry.next_delay(&options), None);
    }

    #[test]
    fn resume_uses_the_consumed_cursor_and_discards_it_for_a_fresh_snapshot() {
        let mut options = SubscriptionOptions::default();
        options.from = Some(VersionId::from(42));
        let mut consumed = Some(VersionId::from(41));
        SubscriptionRecovery::Replay.apply(&mut options, &mut consumed);
        assert_eq!(options.from, Some(VersionId::from(42)));
        consumed = Some(VersionId::from(43));
        resume_from(&mut options, consumed);
        assert_eq!(options.from, consumed);
        SubscriptionRecovery::Snapshot.apply(&mut options, &mut consumed);
        assert_eq!(options.from, None);
        assert_eq!(consumed, None);
    }

    #[test]
    fn server_history_expiry_requires_a_fresh_snapshot() {
        for message in [
            "complete-event history expired; resynchronize",
            "latest-state cursor expired; resynchronize",
        ] {
            assert_eq!(
                SubscriptionRecovery::from_error("UNSUPPORTED", message),
                Some(SubscriptionRecovery::Snapshot)
            );
        }
    }
}
