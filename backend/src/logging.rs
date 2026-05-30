// Logging module — powered by tracing-subscriber
//
// Uses tracing-subscriber for structured spans & events.
// A compatibility bridge (`tracing_log::LogTracer`) captures all existing
// `log::*` macro calls and routes them through the tracing subscriber so
// span context is preserved end-to-end.

#[cfg(feature = "otel")]
use std::sync::{Mutex, OnceLock};
#[cfg(feature = "otel")]
use std::time::Duration;
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    path::Path,
};

use kalamdb_configs::config::types::OtlpSettings;
#[cfg(feature = "otel")]
use opentelemetry::trace::TracerProvider as _;
#[cfg(feature = "otel")]
use opentelemetry_otlp::WithExportConfig;
#[cfg(feature = "otel")]
use opentelemetry_sdk::trace::SdkTracerProvider;
#[cfg(feature = "otel")]
use opentelemetry_sdk::Resource;
#[cfg(feature = "otel")]
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::{
    fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer,
};

#[cfg(feature = "otel")]
static OTEL_TRACER_PROVIDER: OnceLock<Mutex<Option<SdkTracerProvider>>> = OnceLock::new();

#[cfg(feature = "otel")]
fn tracer_provider_slot() -> &'static Mutex<Option<SdkTracerProvider>> {
    OTEL_TRACER_PROVIDER.get_or_init(|| Mutex::new(None))
}

#[cfg(feature = "otel")]
fn is_otlp_noisy_target(target: &str) -> bool {
    // Drop exporter/system transport chatter while keeping application spans/events.
    let noisy_prefixes = [
        "h2",
        "hyper",
        "tower",
        "tonic",
        "openraft",
        "kalamdb_raft",
        "opentelemetry",
        "opentelemetry_sdk",
        "opentelemetry_otlp",
        "mio",
        "tokio_util",
        "want",
    ];

    noisy_prefixes
        .iter()
        .any(|prefix| target == *prefix || target.starts_with(&format!("{}::", prefix)))
}

/// Log format type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Compact text format: timestamp LEVEL target - message
    Compact,
    /// JSON Lines format for structured logging
    Json,
}

impl LogFormat {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "json" | "jsonl" => LogFormat::Json,
            _ => LogFormat::Compact,
        }
    }
}

#[cfg(any(feature = "otel", test))]
fn level_verbosity(level: &str) -> Option<u8> {
    match level.to_ascii_lowercase().as_str() {
        "off" => Some(0),
        "error" => Some(1),
        "warn" | "warning" => Some(2),
        "info" => Some(3),
        "debug" => Some(4),
        "trace" => Some(5),
        _ => None,
    }
}

#[cfg(any(feature = "otel", test))]
fn event_verbosity(level: &tracing::Level) -> u8 {
    match *level {
        tracing::Level::ERROR => 1,
        tracing::Level::WARN => 2,
        tracing::Level::INFO => 3,
        tracing::Level::DEBUG => 4,
        tracing::Level::TRACE => 5,
    }
}

#[cfg(any(feature = "otel", test))]
fn target_level_override<'a>(
    target: &str,
    target_levels: Option<&'a HashMap<String, String>>,
) -> Option<&'a str> {
    target_levels.and_then(|levels| {
        levels
            .iter()
            .filter(|(configured_target, _)| {
                target == configured_target.as_str()
                    || target
                        .strip_prefix(configured_target.as_str())
                        .is_some_and(|suffix| suffix.starts_with("::"))
            })
            .max_by_key(|(configured_target, _)| configured_target.len())
            .map(|(_, level)| level.as_str())
    })
}

#[cfg(any(feature = "otel", test))]
fn target_level_enabled(
    target: &str,
    event_level: &tracing::Level,
    default_level: &str,
    target_levels: Option<&HashMap<String, String>>,
) -> bool {
    let configured_level = target_level_override(target, target_levels).unwrap_or(default_level);
    let Some(configured_verbosity) = level_verbosity(configured_level) else {
        return false;
    };

    configured_verbosity != 0 && event_verbosity(event_level) <= configured_verbosity
}

/// Build the `EnvFilter` from the base level, hardcoded noisy-crate
/// overrides, and optional per-target overrides from config.
fn build_env_filter(
    level: &str,
    target_levels: Option<&HashMap<String, String>>,
) -> anyhow::Result<EnvFilter> {
    // Base directive — set the default level
    let mut directives = vec![level.to_string()];

    // Suppress noisy third-party crates
    let noisy: &[(&str, &str)] = &[
        ("actix_server", "warn"),
        ("actix_web", "warn"),
        ("h2", "warn"),
        ("sqlparser", "warn"),
        ("datafusion", "warn"),
        ("datafusion_optimizer", "warn"),
        ("datafusion_datasource", "warn"),
        ("arrow", "warn"),
        ("parquet", "warn"),
        ("object_store", "info"),
        ("openraft", "error"),
        ("openraft::replication", "off"),
        ("tracing", "warn"),
        ("tracing_actix_web::middleware", "error"),
        // Reduce verbose Raft logs
        ("kalamdb_core::applier::raft", "warn"),
    ];
    for (target, lvl) in noisy {
        directives.push(format!("{}={}", target, lvl));
    }

    // Per-target overrides from server.toml
    if let Some(map) = target_levels {
        for (target, lvl) in map.iter() {
            directives.push(format!("{}={}", target, lvl));
        }
    }

    let filter_str = directives.join(",");
    EnvFilter::try_new(&filter_str)
        .map_err(|e| anyhow::anyhow!("Invalid tracing filter '{}': {}", filter_str, e))
}

/// Initialize logging based on configuration.
///
/// Sets up `tracing-subscriber` with:
///  - Colored console layer (when `log_to_console` is true)
///  - File layer (compact text or JSON lines)
///  - `tracing_log::LogTracer` bridge so that all `log::*` calls are captured
///  - Span events on CLOSE (prints elapsed time for each span)
pub fn init_logging(
    level: &str,
    file_path: &str,
    log_to_console: bool,
    target_levels: Option<&HashMap<String, String>>,
    format: &str,
    otlp: &OtlpSettings,
) -> anyhow::Result<()> {
    let log_format = LogFormat::from_str(format);

    // Bridge `log` crate -> tracing as early as possible so existing subscribers
    // in tests still receive `log::*` records, and so later `try_init()` failure
    // does not silently disable the bridge.
    tracing_log::LogTracer::init().ok();

    // Create logs directory if it doesn't exist
    if let Some(parent) = Path::new(file_path).parent() {
        fs::create_dir_all(parent)?;
    }

    // Open log file in append mode
    let log_file = OpenOptions::new().create(true).append(true).open(file_path)?;

    // -- Console layer (optional) --
    let console_layer = if log_to_console {
        Some(
            tracing_subscriber::fmt::layer()
                .with_ansi(true)
                .with_target(true)
                .with_level(true)
                .with_thread_names(true)
                .with_thread_ids(false)
                .compact()
                .with_span_events(FmtSpan::NONE) // Change to CLOSE to show span timing
                .with_filter(build_env_filter(level, target_levels)?),
        )
    } else {
        None
    };

    // -- File layer --
    let file_layer = if log_format == LogFormat::Json {
        // JSON lines — includes span fields automatically
        let layer = tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .with_writer(log_file)
            .with_target(true)
            .with_thread_names(true)
            .with_span_events(FmtSpan::NONE) // Change to CLOSE to show span timing
            // Keep the current span for request/job correlation, but avoid
            // serializing the full span stack on every log line.
            .with_current_span(true)
            .with_filter(build_env_filter(level, target_levels)?);
        // We need to box because the json() layer has a different type
        layer.boxed()
    } else {
        let layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(log_file)
            .with_target(true)
            .with_level(true)
            .with_thread_names(true)
            .with_thread_ids(false)
            .compact()
            .with_span_events(FmtSpan::NONE) // Change to CLOSE to show span timing
            .with_filter(build_env_filter(level, target_levels)?);
        layer.boxed()
    };

    // Compose and install as global subscriber.
    // Use try_init() to handle cases where subscriber is already initialized
    // (e.g., in testing or when running multiple times).
    #[cfg(feature = "otel")]
    let init_result = if otlp.enabled {
        let tracer_provider = build_otlp_provider(otlp)?;
        let tracer = tracer_provider.tracer("kalamdb-server");
        let otlp_target_levels = target_levels.cloned();
        let otlp_default_level = level.to_string();

        // Respect the configured level/target overrides, but still strip known
        // transport/exporter chatter from trace export.
        let otlp_filter = filter_fn(move |metadata| {
            if is_otlp_noisy_target(metadata.target()) {
                return false;
            }

            target_level_enabled(
                metadata.target(),
                metadata.level(),
                &otlp_default_level,
                otlp_target_levels.as_ref(),
            )
        });

        let otlp_layer =
            tracing_opentelemetry::layer().with_tracer(tracer).with_filter(otlp_filter);

        let result = tracing_subscriber::registry()
            .with(console_layer)
            .with(file_layer)
            .with(otlp_layer)
            .try_init();
        (result, Some(tracer_provider))
    } else {
        let result = tracing_subscriber::registry().with(console_layer).with(file_layer).try_init();
        (result, None)
    };

    #[cfg(not(feature = "otel"))]
    let init_result = {
        let _ = &otlp; // suppress unused warning
        let result = tracing_subscriber::registry().with(console_layer).with(file_layer).try_init();
        (result, None::<()>)
    };

    match init_result.0 {
        Ok(_) => {
            #[cfg(feature = "otel")]
            if let Some(provider) = init_result.1 {
                if let Ok(mut guard) = tracer_provider_slot().lock() {
                    *guard = Some(provider);
                }
            }

            tracing::trace!(
                "Logging initialized: level={}, console={}, file={}",
                level,
                log_to_console,
                file_path
            );
        },
        Err(e) => {
            #[cfg(feature = "otel")]
            if let Some(provider) = init_result.1 {
                let _ = provider.shutdown();
            }
            // Subscriber already initialized - this can happen in test contexts
            // Continue with existing subscriber, but file logging won't be available
            eprintln!("⚠️  Note: Tracing subscriber already initialized: {}", e);
            eprintln!(
                "   Using existing logging configuration (file logging may not be available)."
            );
        },
    }

    Ok(())
}

#[allow(dead_code)]
/// Initialize simple logging for development (console only)
pub fn init_simple_logging() -> anyhow::Result<()> {
    tracing_log::LogTracer::init().ok();

    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(true)
        .with_span_events(FmtSpan::CLOSE)
        .init();

    Ok(())
}

/// Flush and shutdown OTLP tracer provider, if installed.
pub fn shutdown_telemetry() {
    #[cfg(feature = "otel")]
    if let Ok(mut guard) = tracer_provider_slot().lock() {
        if let Some(provider) = guard.take() {
            let _ = provider.shutdown();
        }
    }
}

#[cfg(feature = "otel")]
fn build_otlp_provider(otlp: &OtlpSettings) -> anyhow::Result<SdkTracerProvider> {
    let protocol = otlp.protocol.to_ascii_lowercase();
    let timeout = Duration::from_millis(otlp.timeout_ms.max(1));

    let exporter = match protocol.as_str() {
        "grpc" => opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(otlp.endpoint.clone())
            .with_timeout(timeout)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build OTLP gRPC span exporter: {}", e))?,
        "http" => opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(normalize_http_endpoint(&otlp.endpoint))
            .with_timeout(timeout)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build OTLP HTTP span exporter: {}", e))?,
        _ => {
            return Err(anyhow::anyhow!(
                "Unsupported OTLP protocol '{}'. Use 'grpc' or 'http'.",
                otlp.protocol
            ));
        },
    };

    let resource = Resource::builder().with_service_name(otlp.service_name.clone()).build();

    let tracer_provider = SdkTracerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(exporter)
        .build();

    Ok(tracer_provider)
}

#[cfg(feature = "otel")]
fn normalize_http_endpoint(endpoint: &str) -> String {
    if endpoint.ends_with("/v1/traces") {
        endpoint.to_string()
    } else {
        format!("{}/v1/traces", endpoint.trim_end_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use kalamdb_commons::helpers::security::redact_sensitive_sql;

    use super::{target_level_enabled, target_level_override};

    #[test]
    fn target_level_override_prefers_longest_matching_prefix() {
        let levels = HashMap::from([
            ("kalamdb_core".to_string(), "info".to_string()),
            ("kalamdb_core::sql::executor".to_string(), "debug".to_string()),
        ]);

        let override_level =
            target_level_override("kalamdb_core::sql::executor::sql_executor", Some(&levels));

        assert_eq!(override_level, Some("debug"));
    }

    #[test]
    fn target_level_enabled_honors_target_override_and_default() {
        let levels =
            HashMap::from([("kalamdb_core::sql::executor".to_string(), "debug".to_string())]);

        assert!(target_level_enabled(
            "kalamdb_core::sql::executor::sql_executor",
            &tracing::Level::DEBUG,
            "info",
            Some(&levels),
        ));
        assert!(!target_level_enabled(
            "kalamdb_publisher::service",
            &tracing::Level::DEBUG,
            "info",
            Some(&levels),
        ));
    }

    #[test]
    fn test_redact_sensitive_sql_passwords() {
        // ALTER USER with SET PASSWORD
        let sql = "ALTER USER 'alice' SET PASSWORD 'SuperSecret123!'";
        let redacted = redact_sensitive_sql(sql);
        assert!(!redacted.contains("SuperSecret123"));
        assert!(redacted.contains("[REDACTED]"));

        // CREATE USER with PASSWORD
        let sql = "CREATE USER bob PASSWORD 'mypassword'";
        let redacted = redact_sensitive_sql(sql);
        assert!(!redacted.contains("mypassword"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn test_redact_sensitive_sql_preserves_safe_queries() {
        let sql = "SELECT * FROM users WHERE name = 'alice'";
        let redacted = redact_sensitive_sql(sql);
        assert_eq!(sql, redacted);
    }
}
