//! Testable process startup policy for the KalamDB server binary.

use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use kalamdb_configs::ServerConfig;
use kalamdb_core::metrics::{BUILD_DATE, SERVER_VERSION};
use kalamdb_observability::initialize_activity_now;
use kalamdb_postgres_wire::{http_port_conflict_message, rpc_port_conflict_message};
use log::info;

use crate::{
    http_server::effective_max_blocking_threads,
    lifecycle::{bootstrap, run},
    logging,
    startup::configure_auth_runtime,
};

const INSECURE_JWT_SECRETS: &[&str] = &[
    "CHANGE_ME_IN_PRODUCTION",
    "kalamdb-dev-secret-key-change-in-production",
    "your-secret-key-at-least-32-chars-change-me-in-production",
    "test",
    "secret",
    "password",
];

#[derive(Debug)]
enum StartupCommand {
    Run { config_path: PathBuf },
    Version,
    Help,
}

/// Run the KalamDB process for the supplied command-line arguments.
pub fn run_process<I, S>(args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let config_path = match parse_startup_command(args)? {
        StartupCommand::Version => {
            println!("KalamDB Server v{} | Build: {}", SERVER_VERSION, BUILD_DATE);
            return Ok(());
        },
        StartupCommand::Help => {
            print_help();
            return Ok(());
        },
        StartupCommand::Run { config_path } => config_path,
    };

    let config = load_server_config(&config_path)?;
    let worker_threads = resolve_tokio_worker_threads(&config);
    let max_blocking_threads =
        effective_max_blocking_threads(config.performance.worker_max_blocking_threads);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .thread_name("kalamdb-tokio")
        .worker_threads(worker_threads)
        .max_blocking_threads(max_blocking_threads)
        .enable_all()
        .build()
        .context("failed to build Tokio runtime")?;

    let result = runtime.block_on(start_server(config, worker_threads, max_blocking_threads));
    // A timed-out blocking storage operation cannot be cancelled by Tokio. Do not let such an
    // orphan keep process termination waiting forever after the owned async tasks were drained.
    runtime.shutdown_background();
    result
}

fn resolve_config_path() -> PathBuf {
    let cwd_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("server.toml");
    if cwd_path.exists() {
        return cwd_path;
    }

    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|directory| directory.join("server.toml")))
        .unwrap_or_else(|| PathBuf::from("server.toml"))
}

fn parse_startup_command<I, S>(args: I) -> Result<StartupCommand>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = args.into_iter();
    let _executable = args.next();

    match args.next().as_ref().map(AsRef::as_ref) {
        None => Ok(StartupCommand::Run {
            config_path: resolve_config_path(),
        }),
        Some("--version") | Some("-V") | Some("version") => Ok(StartupCommand::Version),
        Some("--help") | Some("-h") | Some("help") => Ok(StartupCommand::Help),
        Some(argument) if argument.starts_with('-') => Err(anyhow!(
            "Unknown option '{}'. Use --help to show supported arguments.",
            argument
        )),
        Some(config_path) => Ok(StartupCommand::Run {
            config_path: PathBuf::from(config_path),
        }),
    }
}

fn print_help() {
    println!("Usage: kalamdb-server [CONFIG_PATH]");
    println!();
    println!("Options:");
    println!("  -h, --help       Show this help message");
    println!("  -V, --version    Show version information");
}

fn load_server_config(config_path: &Path) -> Result<ServerConfig> {
    if !config_path.exists() {
        return Err(anyhow!("config file not found: {}", config_path.display()));
    }

    let mut config = ServerConfig::from_file(config_path)
        .with_context(|| format!("failed to load {}", config_path.display()))?;
    eprintln!(
        "✅ Loaded config from: {}",
        std::fs::canonicalize(config_path)
            .unwrap_or_else(|_| config_path.to_path_buf())
            .display()
    );
    config.apply_env_overrides().context("failed to apply environment overrides")?;
    config.finalize().context("invalid configuration after overrides")?;

    if config.should_warn_on_non_local_http_wildcard_cors() {
        eprintln!(
            "⚠️  SECURITY WARNING: non-local HTTP exposure allows every browser origin; replace \
             security.cors.allowed_origins = [\"*\"] with explicit origins"
        );
    }

    validate_startup_addresses(&config)?;
    Ok(config)
}

fn validate_startup_addresses(config: &ServerConfig) -> Result<()> {
    let http_addr = format!("{}:{}", config.server.host, config.server.port);
    let http_addrs = resolve_bind_addrs(&http_addr, "HTTP")?;

    let rpc_addrs = config
        .cluster
        .as_ref()
        .map(|cluster| resolve_bind_addrs(&cluster.rpc_addr, "Raft RPC"))
        .transpose()?;
    if let (Some(cluster), Some(rpc_addrs)) = (&config.cluster, &rpc_addrs) {
        ensure_no_address_conflict(
            "HTTP",
            &http_addr,
            &http_addrs,
            "Raft RPC",
            &cluster.rpc_addr,
            rpc_addrs,
        )?;
    }

    if config.postgres_wire.enabled {
        let wire_addr = format!("{}:{}", config.postgres_wire.host, config.postgres_wire.port);
        let wire_addrs = resolve_bind_addrs(&wire_addr, "PostgreSQL wire")?;
        if let Some(message) =
            http_port_conflict_message(&config.postgres_wire, &wire_addrs, &http_addrs, &http_addr)
        {
            return Err(anyhow!(message));
        }
        if let (Some(cluster), Some(rpc_addrs)) = (&config.cluster, &rpc_addrs) {
            if let Some(message) = rpc_port_conflict_message(
                &config.postgres_wire,
                &wire_addrs,
                rpc_addrs,
                &cluster.rpc_addr,
            ) {
                return Err(anyhow!(message));
            }
        }
    }

    Ok(())
}

fn resolve_bind_addrs(address: &str, label: &str) -> Result<HashSet<SocketAddr>> {
    let addresses: HashSet<_> = address
        .to_socket_addrs()
        .with_context(|| format!("invalid {label} address '{address}'"))?
        .collect();
    if addresses.is_empty() {
        return Err(anyhow!(
            "invalid {label} address '{address}': resolved to no socket addresses"
        ));
    }
    Ok(addresses)
}

fn ensure_no_address_conflict(
    left_label: &str,
    left_address: &str,
    left: &HashSet<SocketAddr>,
    right_label: &str,
    right_address: &str,
    right: &HashSet<SocketAddr>,
) -> Result<()> {
    if address_sets_conflict(left, right) {
        return Err(anyhow!(
            "invalid configuration: {left_label} '{left_address}' conflicts with {right_label} \
             '{right_address}'; configure distinct ports"
        ));
    }
    Ok(())
}

fn address_sets_conflict(left: &HashSet<SocketAddr>, right: &HashSet<SocketAddr>) -> bool {
    left.iter().any(|left| {
        right.iter().any(|right| {
            left.port() == right.port()
                && same_ip_family(left.ip(), right.ip())
                && (left.ip() == right.ip()
                    || left.ip().is_unspecified()
                    || right.ip().is_unspecified())
        })
    })
}

fn same_ip_family(left: IpAddr, right: IpAddr) -> bool {
    matches!((left, right), (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_)))
}

fn resolve_tokio_worker_threads(config: &ServerConfig) -> usize {
    std::env::var("KALAMDB_TOKIO_WORKER_THREADS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .or_else(|| {
            (config.performance.tokio_worker_threads > 0)
                .then_some(config.performance.tokio_worker_threads)
        })
        .unwrap_or_else(|| num_cpus::get().min(4))
}

fn validate_jwt_secret(host: &str, secret: &str) -> Result<()> {
    let insecure = secret.len() < 32 || INSECURE_JWT_SECRETS.contains(&secret);
    if !insecure {
        return Ok(());
    }

    let localhost = matches!(host, "127.0.0.1" | "localhost" | "::1");
    if localhost {
        eprintln!(
            "⚠️  Insecure JWT secret allowed for localhost development only; configure at least \
             32 random characters before non-local deployment"
        );
        return Ok(());
    }

    Err(anyhow!(
        "refusing to start with an insecure JWT secret on non-local address '{host}'"
    ))
}

async fn start_server(
    config: ServerConfig,
    worker_threads: usize,
    max_blocking_threads: usize,
) -> Result<()> {
    let main_start = std::time::Instant::now();
    configure_auth_runtime(&config)?;
    validate_jwt_secret(&config.server.host, &config.auth.jwt_secret)?;

    let log_extension = if config.logging.format.eq_ignore_ascii_case("json") {
        "jsonl"
    } else {
        "log"
    };
    let server_log_path = format!("{}/server.{log_extension}", config.logging.logs_path);
    logging::init_logging(
        &config.logging.level,
        &server_log_path,
        config.logging.log_to_console,
        Some(&config.logging.targets),
        &config.logging.format,
        &config.logging.otlp,
    )
    .with_context(|| format!("failed to initialize logging at '{server_log_path}'"))?;
    let _telemetry = TelemetryGuard;

    initialize_activity_now();
    info!(
        "Tokio runtime configured: worker_threads={}, max_blocking_threads={}",
        worker_threads, max_blocking_threads
    );
    info!("KalamDB Server v{:<10} | Build: {}", SERVER_VERSION, BUILD_DATE);

    let (components, app_context) = bootstrap(&config).await?;
    run(&config, components, app_context, main_start).await
}

struct TelemetryGuard;

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        logging::shutdown_telemetry();
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_startup_command, validate_jwt_secret, StartupCommand};

    #[test]
    fn parses_version_flag_before_config_resolution() {
        let command = parse_startup_command(["kalamdb-server", "--version"]).unwrap();
        assert!(matches!(command, StartupCommand::Version));
    }

    #[test]
    fn preserves_positional_config_path() {
        let command = parse_startup_command(["kalamdb-server", "ci-server.toml"]).unwrap();
        assert!(matches!(
            command,
            StartupCommand::Run { config_path }
                if config_path == std::path::PathBuf::from("ci-server.toml")
        ));
    }

    #[test]
    fn rejects_unknown_flags() {
        let error = parse_startup_command(["kalamdb-server", "--bogus"]).unwrap_err();
        assert!(error.to_string().contains("Unknown option '--bogus'"));
    }

    #[test]
    fn rejects_insecure_jwt_secret_on_non_local_bind() {
        let error = validate_jwt_secret("0.0.0.0", "secret").unwrap_err();

        assert!(error.to_string().contains("insecure JWT secret"));
    }
}
