use std::collections::HashMap;

#[derive(Default, Clone)]
pub(super) struct SqlLogCursor {
    pub timestamp: String,
    counts:        HashMap<String, usize>,
}

impl SqlLogCursor {
    pub fn observe(&mut self, timestamp: &str, key: String) -> usize {
        if self.timestamp != timestamp {
            self.timestamp = timestamp.to_owned();
            self.counts.clear();
        }
        let count = self.counts.entry(key).or_default();
        *count += 1;
        *count
    }

    pub fn contains(&self, timestamp: &str, key: &str, occurrence: usize) -> bool {
        timestamp < self.timestamp.as_str()
            || (timestamp == self.timestamp
                && occurrence <= self.counts.get(key).copied().unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_identical_events_and_new_events_at_the_same_timestamp() {
        let mut before = SqlLogCursor::default();
        before.observe("2026-09-16T00:00:00Z", "same event".into());
        let mut next = SqlLogCursor::default();
        assert!(before.contains(
            "2026-09-16T00:00:00Z",
            "same event",
            next.observe("2026-09-16T00:00:00Z", "same event".into())
        ));
        assert!(!before.contains(
            "2026-09-16T00:00:00Z",
            "same event",
            next.observe("2026-09-16T00:00:00Z", "same event".into())
        ));
        assert!(!before.contains(
            "2026-09-16T00:00:00Z",
            "different event",
            next.observe("2026-09-16T00:00:00Z", "different event".into())
        ));
    }
}
