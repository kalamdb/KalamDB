#[cfg(unix)]
use std::io::IsTerminal;
use std::time::Duration;

use kalam_client::{
    SeqId, SqlSubscriptionDescriptor, SqlSubscriptionStatus, SubscriptionConfig,
    SubscriptionOptions,
};
#[cfg(unix)]
use tokio::io::AsyncReadExt;

use super::CLISession;
use crate::error::{CLIError, Result};

#[cfg(unix)]
struct TerminalRawModeGuard {
    original: libc::termios,
}

#[cfg(unix)]
impl TerminalRawModeGuard {
    fn new() -> std::io::Result<Self> {
        unsafe {
            let fd = libc::STDIN_FILENO;
            let mut term = std::mem::MaybeUninit::<libc::termios>::uninit();
            if libc::tcgetattr(fd, term.as_mut_ptr()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let mut term = term.assume_init();

            let original = term;
            term.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
            term.c_iflag &= !(libc::IXON | libc::ICRNL);
            term.c_cc[libc::VMIN] = 1;
            term.c_cc[libc::VTIME] = 0;

            if libc::tcsetattr(fd, libc::TCSANOW, &term) != 0 {
                return Err(std::io::Error::last_os_error());
            }

            Ok(Self { original })
        }
    }
}

#[cfg(unix)]
impl Drop for TerminalRawModeGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original);
        }
    }
}

impl CLISession {
    pub(in crate::session) fn extract_subscription_config(
        response: &kalam_client::QueryResponse,
    ) -> Result<Option<(SubscriptionConfig, Option<String>)>> {
        if response.results.is_empty() {
            return Ok(None);
        }

        let result = &response.results[0];
        if result.rows.as_ref().is_none_or(|r| r.is_empty()) {
            return Ok(None);
        }

        let row_map = match result.row_as_map(0) {
            Some(m) => m,
            None => return Ok(None),
        };

        let Some(subscription_value) = row_map.get("subscription") else {
            return Ok(None);
        };

        let Some(status_value) = row_map.get("status") else {
            return Ok(None);
        };

        let status =
            serde_json::from_value::<SqlSubscriptionStatus>(status_value.clone().into_inner())
                .map_err(|_| {
                    CLIError::ParseError("Subscription status has an invalid format".into())
                })?;

        if status != SqlSubscriptionStatus::SubscriptionRequired {
            return Ok(None);
        }

        let message = row_map.get("message").and_then(|v| v.as_str()).map(|s| s.to_string());

        let ws_url = row_map.get("ws_url").and_then(|v| v.as_str()).map(|s| s.to_string());

        let subscription = serde_json::from_value::<SqlSubscriptionDescriptor>(
            subscription_value.clone().into_inner(),
        )
        .map_err(|_| CLIError::ParseError("Subscription metadata must be a JSON object".into()))?;

        if subscription.sql.is_empty() {
            return Err(CLIError::ParseError(
                "Subscription metadata does not include SQL query".into(),
            ));
        }

        let mut config = SubscriptionConfig::new(subscription.id, subscription.sql);

        if let Some(url) = ws_url {
            config.ws_url = Some(url);
        }

        if let Some(options) = subscription.options {
            config.options = Some(options);
        }

        Ok(Some((config, message)))
    }

    pub(in crate::session) fn extract_subscribe_options(
        sql: &str,
    ) -> (String, Option<SubscriptionOptions>) {
        let sql = sql.trim().trim_end_matches(';').trim();
        let sql_upper = sql.to_uppercase();
        let options_idx = sql_upper.rfind(" OPTIONS ").or_else(|| sql_upper.rfind(" OPTIONS("));

        let Some(idx) = options_idx else {
            return (sql.to_string(), Some(SubscriptionOptions::default()));
        };

        let clean_sql = sql[..idx].trim().to_string();
        let options_str = sql[idx + " OPTIONS".len()..].trim();
        let options = Self::parse_subscribe_options(options_str);

        (clean_sql, options)
    }

    fn parse_subscribe_options(options_str: &str) -> Option<SubscriptionOptions> {
        let options_str = options_str.trim();

        if !options_str.starts_with('(') || !options_str.ends_with(')') {
            eprintln!("Warning: Invalid OPTIONS format, using defaults");
            return Some(SubscriptionOptions::default());
        }

        let inner = options_str[1..options_str.len() - 1].trim();
        let mut options = SubscriptionOptions::new();

        for pair in inner.split(',') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }

            let Some((key, value)) = pair.split_once('=') else {
                eprintln!("Warning: Invalid option '{}', expected key=value", pair);
                continue;
            };

            let key = key.trim().to_ascii_lowercase();
            let value = value.trim().trim_matches(['\'', '"']);

            if key == "last_rows" {
                if let Ok(last_rows) = value.parse::<u32>() {
                    options = options.with_last_rows(last_rows);
                } else {
                    eprintln!("Warning: Invalid last_rows value '{}', using default", value);
                }
            } else if key == "batch_size" {
                if let Ok(batch_size) = value.parse::<usize>() {
                    options = options.with_batch_size(batch_size);
                } else {
                    eprintln!("Warning: Invalid batch_size value '{}', using default", value);
                }
            } else if key == "from" || key == "from_seq_id" {
                if let Ok(seq_id) = value.parse::<i64>() {
                    options = options.with_from(SeqId::from(seq_id));
                } else {
                    eprintln!("Warning: Invalid from value '{}', using default", value);
                }
            } else {
                eprintln!("Warning: Unknown option '{}', ignoring", key);
            }
        }

        Some(options)
    }

    #[cfg(unix)]
    async fn wait_for_exit_key_for_subscription() {
        let mut stdin = tokio::io::stdin();
        let mut buf = [0u8; 1];

        loop {
            if stdin.read_exact(&mut buf).await.is_err() {
                break;
            }
            match buf[0] {
                3 | b'q' | b'Q' => break,
                _ => {},
            }
        }
    }

    pub(in crate::session) async fn run_subscription(
        &mut self,
        config: SubscriptionConfig,
    ) -> Result<()> {
        let sql_display = config.sql.clone();
        let ws_url_display = config.ws_url.clone();
        let requested_id = config.id.clone();

        if self.animations {
            eprintln!("Starting subscription for query: {}", sql_display);
            if let Some(ref ws_url) = ws_url_display {
                eprintln!("WebSocket endpoint: {}", ws_url);
            }
            eprintln!("Subscription ID: {}", requested_id);
            eprintln!("Press Ctrl+C (or 'q') to unsubscribe and return to CLI\n");
        }

        let mut subscription = self.client.live_events_with_config(config).await?;

        if self.animations {
            eprintln!("Subscription established (ID: {})", subscription.subscription_id());
        }

        #[cfg(unix)]
        if std::io::stdin().is_terminal() {
            if let Ok(_raw_guard) = TerminalRawModeGuard::new() {
                let mut exit_key = Box::pin(Self::wait_for_exit_key_for_subscription());

                loop {
                    if self.subscription_paused {
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        continue;
                    }

                    tokio::select! {
                        _ = exit_key.as_mut() => {
                            if self.color {
                                println!("\n\x1b[33m⚠ Unsubscribing...\x1b[0m");
                            } else {
                                println!("\n⚠ Unsubscribing...");
                            }

                            let close_res = tokio::time::timeout(Duration::from_secs(2), subscription.close()).await;
                            if close_res.is_err() {
                                eprintln!("Warning: Timed out while closing subscription; exiting anyway");
                            } else if let Ok(Err(e)) = close_res {
                                eprintln!("Warning: Failed to close subscription cleanly: {}", e);
                            }

                            if self.color {
                                println!("\x1b[32m✓ Unsubscribed\x1b[0m Back to CLI prompt");
                            } else {
                                println!("✓ Unsubscribed - Back to CLI prompt");
                            }
                            break;
                        }

                        event_result = subscription.next() => {
                            match event_result {
                                Some(Ok(event)) => {
                                    if matches!(event, kalam_client::ChangeEvent::Error { .. }) {
                                        self.display_change_event(&sql_display, &event);
                                        println!("\nSubscription failed - returning to CLI prompt");
                                        break;
                                    }
                                    self.display_change_event(&sql_display, &event);
                                },
                                Some(Err(e)) => {
                                    eprintln!("Subscription error: {}", e);
                                    break;
                                },
                                None => {
                                    println!("Subscription ended by server");
                                    break;
                                },
                            }
                        }
                    }
                }

                return Ok(());
            }
        }

        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);

        loop {
            if self.subscription_paused {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                continue;
            }

            tokio::select! {
                _ = &mut ctrl_c => {
                    if self.color {
                        println!("\n\x1b[33m⚠ Unsubscribing...\x1b[0m");
                    } else {
                        println!("\n⚠ Unsubscribing...");
                    }
                    let close_res = tokio::time::timeout(Duration::from_secs(2), subscription.close()).await;
                    if close_res.is_err() {
                        eprintln!("Warning: Timed out while closing subscription; exiting anyway");
                    } else if let Ok(Err(e)) = close_res {
                        eprintln!("Warning: Failed to close subscription cleanly: {}", e);
                    }
                    if self.color {
                        println!("\x1b[32m✓ Unsubscribed\x1b[0m Back to CLI prompt");
                    } else {
                        println!("✓ Unsubscribed - Back to CLI prompt");
                    }
                    break;
                }

                event_result = subscription.next() => {
                    match event_result {
                        Some(Ok(event)) => {
                            if matches!(event, kalam_client::ChangeEvent::Error { .. }) {
                                self.display_change_event(&sql_display, &event);
                                println!("\nSubscription failed - returning to CLI prompt");
                                break;
                            }
                            self.display_change_event(&sql_display, &event);
                        },
                        Some(Err(e)) => {
                            eprintln!("Subscription error: {}", e);
                            break;
                        },
                        None => {
                            println!("Subscription ended by server");
                            break;
                        },
                    }
                }
            }
        }

        Ok(())
    }

    pub(in crate::session) async fn run_subscription_with_timeout(
        &mut self,
        config: SubscriptionConfig,
        timeout: Option<std::time::Duration>,
    ) -> Result<()> {
        let sql_display = config.sql.clone();
        let ws_url_display = config.ws_url.clone();
        let requested_id = config.id.clone();

        if self.animations {
            eprintln!("Starting subscription for query: {}", sql_display);
            if let Some(ref ws_url) = ws_url_display {
                eprintln!("WebSocket endpoint: {}", ws_url);
            }
            eprintln!("Subscription ID: {}", requested_id);
            if let Some(timeout) = timeout {
                eprintln!("Timeout: {:?}", timeout);
            } else {
                eprintln!("Press Ctrl+C (or 'q') to unsubscribe and return to CLI");
            }
            eprintln!();
        }

        let mut subscription = self.client.live_events_with_config(config).await?;

        if self.animations {
            eprintln!("Subscription established (ID: {})", subscription.subscription_id());
        }

        #[cfg(unix)]
        if std::io::stdin().is_terminal() {
            if let Ok(_raw_guard) = TerminalRawModeGuard::new() {
                let mut exit_key = Box::pin(Self::wait_for_exit_key_for_subscription());
                let mut initial_data_complete = false;
                let timeout_deadline = timeout.map(|d| tokio::time::Instant::now() + d);

                loop {
                    if initial_data_complete {
                        if let Some(deadline) = timeout_deadline {
                            if tokio::time::Instant::now() >= deadline {
                                if self.animations {
                                    eprintln!("\n⏱ Subscription timeout reached");
                                }
                                let close_res = tokio::time::timeout(
                                    Duration::from_secs(2),
                                    subscription.close(),
                                )
                                .await;
                                if close_res.is_err() {
                                    eprintln!(
                                        "Warning: Timed out while closing subscription; exiting \
                                         anyway"
                                    );
                                } else if let Ok(Err(e)) = close_res {
                                    eprintln!(
                                        "Warning: Failed to close subscription cleanly: {}",
                                        e
                                    );
                                }
                                break;
                            }
                        }
                    }

                    if self.subscription_paused {
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        continue;
                    }

                    let poll_timeout = if timeout.is_some() {
                        tokio::time::Duration::from_millis(100)
                    } else {
                        tokio::time::Duration::from_secs(3600)
                    };

                    tokio::select! {
                        _ = exit_key.as_mut() => {
                            if self.color {
                                println!("\n\x1b[33m⚠ Unsubscribing...\x1b[0m");
                            } else {
                                println!("\n⚠ Unsubscribing...");
                            }
                            let close_res = tokio::time::timeout(Duration::from_secs(2), subscription.close()).await;
                            if close_res.is_err() {
                                eprintln!("Warning: Timed out while closing subscription; exiting anyway");
                            } else if let Ok(Err(e)) = close_res {
                                eprintln!("Warning: Failed to close subscription cleanly: {}", e);
                            }
                            if self.color {
                                println!("\x1b[32m✓ Unsubscribed\x1b[0m Back to CLI prompt");
                            } else {
                                println!("✓ Unsubscribed - Back to CLI prompt");
                            }
                            break;
                        }

                        _ = tokio::time::sleep(poll_timeout) => {
                            continue;
                        }

                        event_result = subscription.next() => {
                            match event_result {
                                Some(Ok(event)) => {
                                    if matches!(event, kalam_client::ChangeEvent::Error { .. }) {
                                        self.display_change_event(&sql_display, &event);
                                        println!("\nSubscription failed - returning to CLI prompt");
                                        break;
                                    }

                                    match &event {
                                        kalam_client::ChangeEvent::InitialDataBatch { batch_control, .. }
                                        | kalam_client::ChangeEvent::Ack { batch_control, .. } => {
                                            if !batch_control.has_more {
                                                initial_data_complete = true;
                                            }
                                        },
                                        _ => {},
                                    }

                                    self.display_change_event(&sql_display, &event);
                                },
                                Some(Err(e)) => {
                                    eprintln!("Subscription error: {}", e);
                                    break;
                                },
                                None => {
                                    println!("Subscription ended by server");
                                    break;
                                },
                            }
                        }
                    }
                }

                return Ok(());
            }
        }

        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);
        let mut initial_data_complete = false;
        let timeout_deadline = timeout.map(|d| tokio::time::Instant::now() + d);

        loop {
            if initial_data_complete {
                if let Some(deadline) = timeout_deadline {
                    if tokio::time::Instant::now() >= deadline {
                        if self.animations {
                            eprintln!("\n⏱ Subscription timeout reached");
                        }
                        let close_res =
                            tokio::time::timeout(Duration::from_secs(2), subscription.close())
                                .await;
                        if close_res.is_err() {
                            eprintln!(
                                "Warning: Timed out while closing subscription; exiting anyway"
                            );
                        } else if let Ok(Err(e)) = close_res {
                            eprintln!("Warning: Failed to close subscription cleanly: {}", e);
                        }
                        break;
                    }
                }
            }

            if self.subscription_paused {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                continue;
            }

            let poll_timeout = if timeout.is_some() {
                tokio::time::Duration::from_millis(100)
            } else {
                tokio::time::Duration::from_secs(3600)
            };

            tokio::select! {
                _ = &mut ctrl_c => {
                    if self.color {
                        println!("\n\x1b[33m⚠ Unsubscribing...\x1b[0m");
                    } else {
                        println!("\n⚠ Unsubscribing...");
                    }
                    if let Err(e) = subscription.close().await {
                        eprintln!("Warning: Failed to close subscription cleanly: {}", e);
                    }
                    if self.color {
                        println!("\x1b[32m✓ Unsubscribed\x1b[0m Back to CLI prompt");
                    } else {
                        println!("✓ Unsubscribed - Back to CLI prompt");
                    }
                    break;
                }

                _ = tokio::time::sleep(poll_timeout) => {
                    continue;
                }

                event_result = subscription.next() => {
                    match event_result {
                        Some(Ok(event)) => {
                            if matches!(event, kalam_client::ChangeEvent::Error { .. }) {
                                self.display_change_event(&sql_display, &event);
                                println!("\nSubscription failed - returning to CLI prompt");
                                break;
                            }

                            match &event {
                                kalam_client::ChangeEvent::InitialDataBatch { batch_control, .. }
                                | kalam_client::ChangeEvent::Ack { batch_control, .. } => {
                                    if !batch_control.has_more {
                                        initial_data_complete = true;
                                    }
                                },
                                _ => {},
                            }

                            self.display_change_event(&sql_display, &event);
                        },
                        Some(Err(e)) => {
                            eprintln!("Subscription error: {}", e);
                            break;
                        },
                        None => {
                            println!("Subscription ended by server");
                            break;
                        },
                    }
                }
            }
        }

        Ok(())
    }

    fn display_change_event(&self, _subscription_sql: &str, event: &kalam_client::ChangeEvent) {
        use chrono::Local;
        let timestamp = Local::now().format("%H:%M:%S%.3f");

        match event {
            kalam_client::ChangeEvent::Ack {
                subscription_id,
                total_rows,
                batch_control,
                schema,
            } => {
                if self.color {
                    println!(
                        "\x1b[36m[{}] ✓ SUBSCRIBED\x1b[0m [{}] {} total rows, batch {} {}, {} \
                         columns",
                        timestamp,
                        subscription_id,
                        total_rows,
                        batch_control.batch_num + 1,
                        if batch_control.has_more {
                            "(loading...)"
                        } else {
                            "(ready)"
                        },
                        schema.len()
                    );
                } else {
                    println!(
                        "[{}] ✓ SUBSCRIBED [{}] {} total rows, batch {} {}, {} columns",
                        timestamp,
                        subscription_id,
                        total_rows,
                        batch_control.batch_num + 1,
                        if batch_control.has_more {
                            "(loading...)"
                        } else {
                            "(ready)"
                        },
                        schema.len()
                    );
                }
            },
            kalam_client::ChangeEvent::InitialDataBatch {
                subscription_id,
                rows,
                batch_control,
            } => {
                let count = rows.len();
                if self.color {
                    println!(
                        "\x1b[34m[{}] BATCH {}\x1b[0m [{}] {} rows {}",
                        timestamp,
                        batch_control.batch_num + 1,
                        subscription_id,
                        count,
                        if batch_control.has_more {
                            "(more pending)"
                        } else {
                            "(complete)"
                        }
                    );
                } else {
                    println!(
                        "[{}] BATCH {} [{}] {} rows {}",
                        timestamp,
                        batch_control.batch_num + 1,
                        subscription_id,
                        count,
                        if batch_control.has_more {
                            "(more pending)"
                        } else {
                            "(complete)"
                        }
                    );
                }

                for row in rows {
                    let formatted = Self::format_row(row);
                    if self.color {
                        println!("  \x1b[90m{}\x1b[0m", formatted);
                    } else {
                        println!("  {}", formatted);
                    }
                }
            },
            kalam_client::ChangeEvent::Insert {
                subscription_id,
                rows,
            } => {
                if rows.is_empty() {
                    if self.color {
                        println!(
                            "\x1b[32m[{}] INSERT\x1b[0m [{}] (no row payload)",
                            timestamp, subscription_id
                        );
                    } else {
                        println!("[{}] INSERT [{}] (no row payload)", timestamp, subscription_id);
                    }
                } else {
                    for row in rows {
                        let row_str = Self::format_row(row);
                        if self.color {
                            println!(
                                "\x1b[32m[{}] INSERT\x1b[0m [{}] {}",
                                timestamp, subscription_id, row_str
                            );
                        } else {
                            println!("[{}] INSERT [{}] {}", timestamp, subscription_id, row_str);
                        }
                    }
                }
            },
            kalam_client::ChangeEvent::Update {
                subscription_id,
                rows,
                old_rows,
            } => {
                let max_len = rows.len().max(old_rows.len());
                if max_len == 0 {
                    if self.color {
                        println!(
                            "\x1b[33m[{}] UPDATE\x1b[0m [{}] (no row payload)",
                            timestamp, subscription_id
                        );
                    } else {
                        println!("[{}] UPDATE [{}] (no row payload)", timestamp, subscription_id);
                    }
                } else {
                    for idx in 0..max_len {
                        let new_str = rows
                            .get(idx)
                            .map(Self::format_row)
                            .unwrap_or_else(|| "<missing>".to_string());
                        let old_str = old_rows
                            .get(idx)
                            .map(Self::format_row)
                            .unwrap_or_else(|| "<missing>".to_string());
                        if self.color {
                            println!(
                                "\x1b[33m[{}] UPDATE\x1b[0m [{}] {} ⇒ {}",
                                timestamp, subscription_id, old_str, new_str
                            );
                        } else {
                            println!(
                                "[{}] UPDATE [{}] {} => {}",
                                timestamp, subscription_id, old_str, new_str
                            );
                        }
                    }
                }
            },
            kalam_client::ChangeEvent::Delete {
                subscription_id,
                old_rows,
            } => {
                if old_rows.is_empty() {
                    if self.color {
                        println!(
                            "\x1b[31m[{}] DELETE\x1b[0m [{}] (no row payload)",
                            timestamp, subscription_id
                        );
                    } else {
                        println!("[{}] DELETE [{}] (no row payload)", timestamp, subscription_id);
                    }
                } else {
                    for row in old_rows {
                        let row_str = Self::format_row(row);
                        if self.color {
                            println!(
                                "\x1b[31m[{}] DELETE\x1b[0m [{}] {}",
                                timestamp, subscription_id, row_str
                            );
                        } else {
                            println!("[{}] DELETE [{}] {}", timestamp, subscription_id, row_str);
                        }
                    }
                }
            },
            kalam_client::ChangeEvent::Error {
                subscription_id,
                code,
                message,
            } => {
                if self.color {
                    eprintln!(
                        "\x1b[31m[{}] ERROR\x1b[0m [{}] {}: {}",
                        timestamp, subscription_id, code, message
                    );
                } else {
                    eprintln!("[{}] ERROR [{}] {}: {}", timestamp, subscription_id, code, message);
                }
            },
            kalam_client::ChangeEvent::Unknown { raw } => {
                if self.color {
                    eprintln!("\x1b[90m[{}] DEBUG: Unrecognized message type\x1b[0m", timestamp);
                } else {
                    eprintln!("[{}] DEBUG: Unrecognized message type", timestamp);
                }
                #[cfg(debug_assertions)]
                eprintln!("  Payload: {}", serde_json::to_string(raw).unwrap_or_default());
                #[cfg(not(debug_assertions))]
                let _ = raw;
            },
        }
    }

    #[allow(dead_code)]
    fn format_json(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::String(s) => format!("\"{}\"", s),
            serde_json::Value::Null => "null".to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Number(n) => n.to_string(),
            _ => serde_json::to_string(value).unwrap_or_else(|_| value.to_string()),
        }
    }

    pub(in crate::session) fn format_row(row: &kalam_client::RowData) -> String {
        serde_json::to_string(row).unwrap_or_else(|_| format!("{:?}", row))
    }

    pub async fn subscribe(&mut self, query: &str) -> Result<()> {
        self.subscribe_with_timeout(query, None).await
    }

    pub async fn subscribe_with_timeout(
        &mut self,
        query: &str,
        timeout: Option<std::time::Duration>,
    ) -> Result<()> {
        let sub_id = format!(
            "sub_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos()
        );
        let config = self.build_subscription_config(query, sub_id)?;
        self.run_subscription_with_timeout(config, timeout).await
    }

    pub async fn list_subscriptions(&mut self) -> Result<()> {
        println!("Subscription management:");
        println!("  • Subscriptions run in blocking mode per CLI session");
        println!("  • Use Ctrl+C to cancel active subscriptions");
        println!("  • Each CLI instance can have at most one active subscription");
        println!("  • No persistent subscription registry is currently implemented");
        Ok(())
    }
}
