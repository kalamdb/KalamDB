/// Time bucket granularity for log layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamTimeBucket {
    Minute,
    Hour,
    Day,
    Week,
    Month,
}

impl StreamTimeBucket {
    pub fn duration_ms(&self) -> u64 {
        match self {
            StreamTimeBucket::Minute => 60 * 1000,
            StreamTimeBucket::Hour => 60 * 60 * 1000,
            StreamTimeBucket::Day => 24 * 60 * 60 * 1000,
            StreamTimeBucket::Week => 7 * 24 * 60 * 60 * 1000,
            StreamTimeBucket::Month => 31 * 24 * 60 * 60 * 1000,
        }
    }
}

/// Resolve bucket granularity from TTL (seconds).
pub fn bucket_for_ttl(ttl_seconds: u64) -> StreamTimeBucket {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * 60;
    const DAY: u64 = 24 * HOUR;
    const WEEK: u64 = 7 * DAY;
    const MONTH: u64 = 30 * DAY;
    const SHORT_TTL: u64 = 15 * MINUTE;

    if ttl_seconds <= SHORT_TTL {
        StreamTimeBucket::Minute
    } else if ttl_seconds <= DAY {
        StreamTimeBucket::Hour
    } else if ttl_seconds <= WEEK {
        StreamTimeBucket::Day
    } else if ttl_seconds <= MONTH {
        StreamTimeBucket::Week
    } else {
        StreamTimeBucket::Month
    }
}

#[cfg(test)]
mod tests {
    use super::{bucket_for_ttl, StreamTimeBucket};

    #[test]
    fn test_short_ttl_uses_minute_buckets() {
        assert_eq!(bucket_for_ttl(10), StreamTimeBucket::Minute);
        assert_eq!(bucket_for_ttl(30), StreamTimeBucket::Minute);
        assert_eq!(bucket_for_ttl(15 * 60), StreamTimeBucket::Minute);
    }

    #[test]
    fn test_longer_ttl_uses_coarser_buckets() {
        assert_eq!(bucket_for_ttl(16 * 60), StreamTimeBucket::Hour);
        assert_eq!(bucket_for_ttl(2 * 24 * 60 * 60), StreamTimeBucket::Day);
        assert_eq!(bucket_for_ttl(10 * 24 * 60 * 60), StreamTimeBucket::Week);
        assert_eq!(bucket_for_ttl(45 * 24 * 60 * 60), StreamTimeBucket::Month);
    }
}
