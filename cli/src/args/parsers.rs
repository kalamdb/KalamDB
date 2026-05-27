use std::time::Duration;

use humantime::parse_duration;

pub(super) fn parse_watch_interval(value: &str) -> Result<Duration, String> {
    let trimmed = value.trim();
    let duration = if !trimmed.is_empty() && trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
        let seconds = trimmed.parse::<u64>().map_err(|err| err.to_string())?;
        Duration::from_secs(seconds)
    } else {
        parse_duration(trimmed).map_err(|err| err.to_string())?
    };

    if duration.is_zero() {
        return Err("interval must be greater than zero".into());
    }

    Ok(duration)
}
