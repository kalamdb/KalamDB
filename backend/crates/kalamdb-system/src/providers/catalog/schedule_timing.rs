//! Shared schedule validation and next-run calculation for DDL and dispatch.
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use croner::Cron;

/// Advance a due schedule without replaying missed occurrences.
pub fn advance_schedule_run(
    cron: Option<&str>,
    interval_ms: Option<i64>,
    timezone: &str,
    scheduled_at: i64,
    now: i64,
) -> Result<i64, String> {
    if cron.is_none() {
        if let Some(ms) = interval_ms.filter(|ms| *ms >= 1000) {
            // Use a wider intermediate so long outages cannot overflow subtraction
            // or multiplication. Keep the original phase and skip the backlog in O(1).
            let elapsed = (i128::from(now) - i128::from(scheduled_at)).max(0);
            let steps = elapsed / i128::from(ms) + 1;
            return i64::try_from(i128::from(scheduled_at) + steps * i128::from(ms))
                .map_err(|_| "Interval overflow".into());
        }
    }
    next_schedule_run(cron, interval_ms, timezone, now)
}

pub fn next_schedule_run(
    cron: Option<&str>,
    interval_ms: Option<i64>,
    timezone: &str,
    after_ms: i64,
) -> Result<i64, String> {
    let timezone: Tz = timezone.parse().map_err(|_| format!("Unknown time zone '{timezone}'"))?;
    match (cron, interval_ms) {
        (None, Some(ms)) if ms >= 1000 => {
            after_ms.checked_add(ms).ok_or_else(|| "Interval overflow".into())
        },
        (Some(expression), None) => {
            if expression.split_whitespace().count() != 5 {
                return Err("CRON requires five fields: minute hour day month weekday".into());
            }
            let schedule: Cron = expression.parse().map_err(|e| format!("Invalid CRON: {e}"))?;
            let after = DateTime::<Utc>::from_timestamp_millis(after_ms)
                .ok_or("Invalid schedule time")?
                .with_timezone(&timezone);
            schedule
                .find_next_occurrence(&after, false)
                .map(|next| next.timestamp_millis())
                .map_err(|e| format!("No next CRON occurrence: {e}"))
        },
        _ => Err("Specify CRON or an INTERVAL of at least one second".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interval_cadence_survives_poll_jitter() {
        let mut due = 1000;
        for now in [1001, 2000, 3001, 4000, 5001] {
            assert!(due <= now, "deadline {due} missed at poll {now}");
            due = advance_schedule_run(None, Some(1000), "UTC", due, now).unwrap();
        }
        assert_eq!(due, 6000);
    }

    #[test]
    fn advancing_interval_skips_backlog_without_changing_phase() {
        assert_eq!(advance_schedule_run(None, Some(1000), "UTC", 1000, 9500).unwrap(), 10000);
        assert_eq!(advance_schedule_run(None, Some(1000), "UTC", 1000, 10000).unwrap(), 11000);
        assert!(advance_schedule_run(None, Some(1000), "UTC", i64::MAX - 10, i64::MAX).is_err());
        assert!(advance_schedule_run(None, Some(0), "UTC", 1000, 2000).is_err());
        assert_eq!(advance_schedule_run(None, Some(1000), "UTC", i64::MIN, 0).unwrap(), 192);
    }

    #[test]
    fn advancing_cron_selects_next_future_occurrence() {
        let now = DateTime::parse_from_rfc3339("2026-01-01T14:00:00Z").unwrap().timestamp_millis();
        let next = DateTime::parse_from_rfc3339("2026-01-02T14:00:00Z").unwrap().timestamp_millis();
        assert_eq!(
            advance_schedule_run(
                Some("0 9 * * *"),
                None,
                "America/New_York",
                now - 86_400_000,
                now
            )
            .unwrap(),
            next
        );
    }

    #[test]
    fn cron_uses_timezone_and_five_fields() {
        let start =
            DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap().timestamp_millis();
        let expected =
            DateTime::parse_from_rfc3339("2026-01-01T14:00:00Z").unwrap().timestamp_millis();
        assert_eq!(
            next_schedule_run(Some("0 9 * * *"), None, "America/New_York", start).unwrap(),
            expected
        );
        assert!(next_schedule_run(Some("0 0 9 * * *"), None, "UTC", start).is_err());
        assert!(next_schedule_run(Some("* * * * *"), None, "Unknown", start).is_err());
    }
    #[test]
    fn interval_skips_backlog_and_rejects_overflow() {
        assert_eq!(next_schedule_run(None, Some(1000), "UTC", 90000).unwrap(), 91000);
        assert!(next_schedule_run(None, Some(1), "UTC", 0).is_err());
        assert!(next_schedule_run(None, Some(1000), "UTC", i64::MAX).is_err());
    }
}
