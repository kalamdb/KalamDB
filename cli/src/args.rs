use std::{path::PathBuf, time::Duration};

use clap::{Args, Parser, Subcommand, ValueEnum};
use kalam_cli::OutputFormat;

#[path = "args/parsers.rs"]
mod parsers;
#[path = "args/workflow.rs"]
mod workflow;

use parsers::parse_watch_interval;
pub use workflow::{
    DbArgs, DbCommand, DeployArgs, DevArgs, DevCommand, DownArgs, FunctionsArgs, FunctionsCommand,
    InitArgs, InstancesArgs, LinkArgs, LogsArgs, MigrationCommand, SchemaArgs, SchemaCommand,
    StatusArgs, UpArgs,
};

// Build information - Create a static version string at compile time

// Macro to create the version string at compile time
macro_rules! version_string {
    () => {
        concat!(
            env!("CARGO_PKG_VERSION"),
            "\nCommit: ",
            env!("GIT_COMMIT_HASH"),
            " (",
            env!("GIT_BRANCH"),
            ")\nBuilt: ",
            env!("BUILD_DATE")
        )
    };
}

const AFTER_HELP: &str = "\
EVERYDAY
  dev        Develop your application with schema and procedure watching
  up         Start a local database
  down       Stop a local database; keep its data
  status     Show server status for this folder or selected target
  instances  List local servers and saved cloud connections
  logs       View database logs

GET STARTED
  init       Add KalamDB to an application or use a template
  login      Authenticate
  link       Connect a project to a remote environment

DATABASE & DEPLOYMENT
  db         Migrations, reset, and development fixtures
  schema     Generate types from the local schema
  deploy     Apply database and procedure changes

ADVANCED & MAINTENANCE
  functions  Inspect and manage procedures
  token      Service tokens
  invite     OIDC invitations
  doctor     Diagnose configuration and connectivity
  update     Update the CLI

SQL
  kalam                    Open the SQL shell
  kalam -c \"SELECT ...\"    Execute SQL
  kalam --file query.sql   Execute a SQL file

TARGETS
  --global, -g             Shared local database
  --env NAME               Named project environment
  --agent                  Deterministic events for AI coding agents
  --watch-schema           Watch information_schema.tables and run a command
";

/// KalamDB CLI for projects, SQL, development, and deployment
#[derive(Parser, Debug)]
#[command(name = "kalam")]
#[command(author = "KalamDB Team")]
#[command(version = version_string!())]
#[command(about = "KalamDB CLI for projects, SQL, development, and deployment", long_about = None)]
#[command(after_help = AFTER_HELP)]
pub struct Cli {
    /// Command to run (for example: login, logout, whoami, doctor, update)
    #[command(subcommand)]
    pub subcommand: Option<CliCommand>,

    /// Server URL (e.g., http://localhost:2900)
    #[arg(short = 'u', long = "url", global = true)]
    pub url: Option<String>,

    /// Host address (alternative to URL)
    #[arg(short = 'H', long = "host", global = true)]
    pub host: Option<String>,

    /// Port number (default: 2900 when used with --host or `kalam up`)
    #[arg(short = 'p', long = "port", global = true)]
    pub port: Option<u16>,

    /// Named project environment (`dev`, `staging`, ...)
    #[arg(long = "env", global = true)]
    pub env: Option<String>,

    /// Use the current user's shared local database (`~/.kalam/servers/default`)
    #[arg(short = 'g', long = "global", global = true)]
    pub global: bool,

    /// Deterministic, non-interactive mode optimized for AI coding agents and automation
    #[arg(long = "agent", global = true)]
    pub agent: bool,

    /// JWT authentication token (avoid in shared shells; may appear in process list/history)
    #[arg(long = "token", global = true)]
    pub token: Option<String>,

    /// HTTP Basic Auth user identifier
    #[arg(long = "user", global = true)]
    pub user: Option<String>,

    /// HTTP Basic Auth password (if flag is present without value, prompts interactively;
    /// avoid passing inline secrets in shared shells)
    #[arg(long = "password", num_args = 0..=1, default_missing_value = "", global = true)]
    pub password: Option<String>,

    /// Instance name for saved credentials or a named local server
    #[arg(long = "instance", default_value = "local", global = true)]
    pub instance: String,

    /// Execute SQL from file and exit
    #[arg(short = 'f', long = "file")]
    pub file: Option<PathBuf>,

    /// Execute a SQL statement or shared CLI command and exit
    #[arg(short = 'c', long = "command", num_args = 1.., conflicts_with = "file")]
    pub command: Option<Vec<String>>,

    /// Output format
    #[arg(long = "format", default_value = "table", global = true)]
    pub format: OutputFormat,

    /// Enable JSON output (shorthand for --format=json)
    #[arg(long = "json", conflicts_with = "format", global = true)]
    pub json: bool,

    /// Enable CSV output (shorthand for --format=csv)
    #[arg(long = "csv", conflicts_with = "format", global = true)]
    pub csv: bool,

    /// Disable colored output
    #[arg(long = "no-color", global = true)]
    pub no_color: bool,

    /// Disable spinners/animations
    #[arg(long = "no-spinner", global = true)]
    pub no_spinner: bool,

    /// Loading indicator threshold in ms (0 to always show)
    #[arg(long = "loading-threshold-ms", global = true, hide = true)]
    pub loading_threshold_ms: Option<u64>,

    /// Configuration file path
    #[arg(long = "config", default_value = "~/.kalam/config.toml", global = true)]
    pub config: PathBuf,

    /// Enable verbose logging
    #[arg(short = 'v', long = "verbose", global = true)]
    pub verbose: bool,

    /// HTTP request timeout in seconds (default: 30)
    #[arg(
        long = "timeout",
        value_name = "SECONDS",
        default_value_t = 30,
        global = true,
        hide = true
    )]
    pub timeout: u64,

    /// Connection timeout in seconds (TCP + TLS handshake, default: 10)
    #[arg(
        long = "connection-timeout",
        value_name = "SECONDS",
        default_value_t = 10,
        global = true,
        hide = true
    )]
    pub connection_timeout: u64,

    /// Receive timeout in seconds (default: 30)
    #[arg(
        long = "receive-timeout",
        value_name = "SECONDS",
        default_value_t = 30,
        global = true,
        hide = true
    )]
    pub receive_timeout: u64,

    /// WebSocket authentication timeout in seconds (default: 5)
    #[arg(
        long = "auth-timeout",
        value_name = "SECONDS",
        default_value_t = 5,
        global = true,
        hide = true
    )]
    pub auth_timeout: u64,

    /// Save credentials (JWT token) after successful login
    /// When used with --user/--password, stores the JWT token for future sessions
    #[arg(long = "save-credentials")]
    pub save_credentials: bool,

    /// List local servers and saved cloud connections (alias for instances)
    #[arg(long = "list-instances")]
    pub list_instances: bool,

    // Subscription management commands
    /// Subscribe to a table or live query
    #[arg(long = "subscribe")]
    pub subscribe: Option<String>,

    /// Subscription timeout in seconds (0 = no timeout, default: 0)
    /// After receiving initial data, subscription will exit after this duration
    #[arg(
        long = "subscription-timeout",
        value_name = "SECONDS",
        default_value_t = 0
    )]
    pub subscription_timeout: u64,

    /// Initial data timeout in seconds (0 = no timeout, default: 30)
    /// Maximum time to wait for initial data batch after subscribing
    #[arg(
        long = "initial-data-timeout",
        value_name = "SECONDS",
        default_value_t = 30
    )]
    pub initial_data_timeout: u64,

    /// Use fast timeout preset (optimized for local development)
    #[arg(long = "fast-timeouts", global = true, hide = true)]
    pub fast_timeouts: bool,

    /// Use relaxed timeout preset (optimized for high-latency networks)
    #[arg(long = "relaxed-timeouts", global = true, hide = true)]
    pub relaxed_timeouts: bool,

    /// Watch schema metadata and run a command when `information_schema.tables` changes
    #[arg(
        long = "watch-schema",
        conflicts_with_all = [
            "file",
            "command",
            "list_instances",
            "subscribe",
            "list_subscriptions",
            "consume"
        ]
    )]
    pub watch_schema: bool,

    /// Namespace to watch for schema changes; repeat to watch multiple namespaces
    #[arg(long = "namespace", requires = "watch_schema")]
    pub watch_namespace: Vec<String>,

    /// Table to watch for schema changes; repeat to watch multiple tables
    #[arg(long = "table", requires = "watch_schema")]
    pub watch_table: Vec<String>,

    /// Shell command to run after schema changes are detected
    #[arg(long = "run", requires = "watch_schema")]
    pub watch_run: Option<String>,

    /// Run the command once immediately before polling for schema changes
    #[arg(long = "run-on-start", requires = "watch_schema")]
    pub watch_run_on_start: bool,

    /// Poll interval for schema watch mode (examples: 5s, 500ms, 1m)
    #[arg(
        long = "interval",
        requires = "watch_schema",
        value_parser = parse_watch_interval,
        default_value = "5s"
    )]
    pub watch_interval: Duration,

    /// List active subscriptions
    #[arg(long = "list-subscriptions")]
    pub list_subscriptions: bool,

    // Topic consumption commands
    /// Start consumer mode (consume messages from a topic)
    #[arg(long = "consume")]
    pub consume: bool,

    /// Topic name for consume mode
    #[arg(long = "topic", requires = "consume")]
    pub topic: Option<String>,

    /// Consumer group ID for consume mode
    #[arg(long = "group")]
    pub group: Option<String>,

    /// Starting offset position: earliest, latest, or numeric offset
    #[arg(long = "from")]
    pub from: Option<String>,

    /// Maximum number of messages to consume before exiting
    #[arg(long = "consume-limit")]
    pub consume_limit: Option<usize>,

    /// Timeout in seconds for consume mode (exit if idle)
    #[arg(long = "consume-timeout")]
    pub consume_timeout: Option<u64>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum CliCommand {
    /// Update this kalam binary from verified release assets and checksums
    Update(UpdateArgs),

    /// Print version information
    Version,

    /// Run local, server, and authentication diagnostics
    Doctor(DoctorArgs),

    /// Login and save credentials for an instance
    Login(LoginArgs),

    /// Delete saved credentials for an instance
    Logout(LogoutArgs),

    /// Show the currently authenticated user
    Whoami,

    /// Create an OIDC email invite for a future login
    Invite(InviteArgs),

    /// Manage service tokens
    Token(TokenArgs),

    /// Initialize or scaffold a KalamDB project workflow
    Init(InitArgs),

    /// Link this project to an environment namespace
    Link(LinkArgs),

    /// Generate types from the local schema
    Schema(SchemaArgs),

    /// Run database migration operations for the linked project
    Db(DbArgs),

    /// Run the local development environment with schema and procedure watching
    #[command(about = "Run the local development environment with schema and procedure watching")]
    Dev(DevArgs),

    /// Start a local database in the background
    Up(UpArgs),

    /// Stop a managed local database and keep its data
    Down(DownArgs),

    /// List local servers and saved cloud connections
    #[command(visible_aliases = ["servers", "list"])]
    Instances(InstancesArgs),

    /// Show server status for this folder or selected project environment
    Status(StatusArgs),

    /// View managed database logs
    Logs(LogsArgs),

    /// Apply migrations and activate procedures
    Deploy(DeployArgs),

    /// Build, inspect, and roll back project procedures
    Functions(FunctionsArgs),
}

#[derive(Args, Debug, Clone)]
pub struct UpdateArgs {
    /// Install a specific version instead of the latest release
    #[arg(long = "version", value_name = "VERSION", conflicts_with_all = ["pre_release", "stable"])]
    pub version: Option<String>,

    /// Use the newest published pre-release (rc, beta, alpha)
    #[arg(long = "pre-release", conflicts_with = "stable")]
    pub pre_release: bool,

    /// Use the latest GitHub stable release. Allows leaving a newer installed pre-release
    #[arg(long = "stable")]
    pub stable: bool,

    /// Show the resolved update without replacing the binary
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Reinstall the CLI and managed kalamdb-server even when the requested version matches
    #[arg(long = "force")]
    pub force: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DoctorArgs {
    /// Exit non-zero when any diagnostic check fails
    #[arg(long = "strict")]
    pub strict: bool,
}

#[derive(Args, Debug, Clone)]
pub struct LoginArgs {
    /// Do not save credentials after login
    #[arg(long = "no-save")]
    pub no_save: bool,

    /// Use local username/password authentication explicitly
    #[arg(long = "local", conflicts_with = "oidc")]
    pub local: bool,

    /// Use the configured OIDC provider instead of local username/password authentication
    #[arg(long = "oidc")]
    pub oidc: bool,

    /// Use OIDC device-code login instead of opening a local browser callback
    #[arg(long = "no-browser", requires = "oidc")]
    pub no_browser: bool,

    /// Override the loopback redirect URI used by OIDC browser login
    #[arg(long = "oidc-redirect-uri", requires = "oidc", value_name = "URI")]
    pub oidc_redirect_uri: Option<String>,

    /// Force KalamDB-brokered OIDC device-code login
    #[arg(long = "brokered", requires = "no_browser")]
    pub brokered: bool,
}

#[derive(Args, Debug, Clone)]
pub struct LogoutArgs {
    /// Delete credentials for every saved instance
    #[arg(long = "all")]
    pub all: bool,
}

#[derive(Args, Debug, Clone)]
pub struct InviteArgs {
    /// Email address that may accept the OIDC invite
    #[arg(long = "email")]
    pub email: String,

    /// Role assigned after the invited user authenticates
    #[arg(long = "role", value_enum, default_value_t = TokenRole::User)]
    pub role: TokenRole,

    /// Number of days before the invite expires
    #[arg(long = "expires-in-days", default_value_t = 7)]
    pub expires_in_days: i64,
}

#[derive(Args, Debug, Clone)]
pub struct TokenArgs {
    #[command(subcommand)]
    pub command: TokenCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum TokenCommand {
    /// Create a service account and print a fresh access/refresh token pair
    Create(TokenCreateArgs),
}

#[derive(Args, Debug, Clone)]
pub struct TokenCreateArgs {
    /// Token/service account name
    #[arg(long = "name")]
    pub name: String,

    /// Role for the generated account
    #[arg(long = "role", value_enum, default_value_t = TokenRole::Service)]
    pub role: TokenRole,

    /// Save the generated token pair as a local credential instance with the same name
    #[arg(long = "save")]
    pub save: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TokenRole {
    User,
    Service,
    Dba,
    System,
}

impl TokenRole {
    pub fn as_sql(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Service => "service",
            Self::Dba => "dba",
            Self::System => "system",
        }
    }
}

pub fn version_report() -> &'static str {
    version_string!()
}

impl Cli {
    pub fn command_text(&self) -> Option<String> {
        self.command.as_ref().map(|parts| parts.join(" "))
    }

    pub fn listen_port(&self) -> u16 {
        self.port.unwrap_or(2900)
    }

    pub fn explicit_instance(&self) -> Option<&str> {
        let trimmed = self.instance.trim();
        if trimmed.is_empty() || trimmed == "local" {
            None
        } else {
            Some(trimmed)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use clap::Parser;

    use super::{parse_watch_interval, Cli, CliCommand, DevCommand, TokenCommand, TokenRole};

    #[test]
    fn parse_watch_interval_defaults_to_seconds() {
        assert_eq!(parse_watch_interval("5").unwrap(), Duration::from_secs(5));
    }

    #[test]
    fn parse_watch_interval_supports_suffixes() {
        assert_eq!(parse_watch_interval("250ms").unwrap(), Duration::from_millis(250));
        assert_eq!(parse_watch_interval("2s").unwrap(), Duration::from_secs(2));
        assert_eq!(parse_watch_interval("3m").unwrap(), Duration::from_secs(180));
        assert_eq!(parse_watch_interval("1h").unwrap(), Duration::from_secs(3600));
    }

    #[test]
    fn parse_watch_interval_rejects_zero() {
        assert!(parse_watch_interval("0s").is_err());
    }

    #[test]
    fn parse_watch_interval_handles_default_five_seconds_literal() {
        assert_eq!(parse_watch_interval("5s").unwrap(), Duration::from_secs(5));
    }

    #[test]
    fn short_connection_and_execution_flags_parse() {
        let cli = Cli::try_parse_from([
            "kalam",
            "-u",
            "http://127.0.0.1:2900",
            "-c",
            "SELECT 1",
            "-v",
        ])
        .expect("short flags should parse");

        assert_eq!(cli.url.as_deref(), Some("http://127.0.0.1:2900"));
        assert_eq!(cli.command_text().as_deref(), Some("SELECT 1"));
        assert!(cli.verbose);
    }

    #[test]
    fn command_flag_accepts_multiple_tokens() {
        let cli = Cli::try_parse_from(["kalam", "--command", "cluster", "list", "groups"])
            .expect("multi-token command should parse");

        assert_eq!(cli.command_text().as_deref(), Some("cluster list groups"));
    }

    #[test]
    fn short_host_port_and_file_flags_parse() {
        let cli = Cli::try_parse_from([
            "kalam",
            "-H",
            "127.0.0.1",
            "-p",
            "2900",
            "-f",
            "./queries.sql",
        ])
        .expect("short flags should parse");

        assert_eq!(cli.host.as_deref(), Some("127.0.0.1"));
        assert_eq!(cli.port, Some(2900));
        assert_eq!(cli.file.as_deref(), Some(Path::new("./queries.sql")));
    }

    #[test]
    fn version_subcommand_parses() {
        let cli = Cli::try_parse_from(["kalam", "version"]).expect("version should parse");

        assert!(matches!(cli.subcommand, Some(CliCommand::Version)));
    }

    #[test]
    fn login_subcommand_accepts_instance_and_url_after_command() {
        let cli = Cli::try_parse_from([
            "kalam",
            "login",
            "--instance",
            "prod",
            "--url",
            "https://db.example.com",
            "--user",
            "root",
            "--password",
            "secret",
        ])
        .expect("login should parse");

        assert!(matches!(cli.subcommand, Some(CliCommand::Login(_))));
        assert_eq!(cli.instance, "prod");
        assert_eq!(cli.url.as_deref(), Some("https://db.example.com"));
        assert_eq!(cli.user.as_deref(), Some("root"));
        assert_eq!(cli.password.as_deref(), Some("secret"));
    }

    #[test]
    fn token_create_subcommand_parses() {
        let cli = Cli::try_parse_from([
            "kalam", "token", "create", "--name", "ci-prod", "--role", "dba", "--save",
        ])
        .expect("token create should parse");

        let Some(CliCommand::Token(args)) = cli.subcommand else {
            panic!("expected token command");
        };
        let TokenCommand::Create(create) = args.command;
        assert_eq!(create.name, "ci-prod");
        assert_eq!(create.role, TokenRole::Dba);
        assert!(create.save);
    }

    #[test]
    fn invite_subcommand_parses() {
        let cli = Cli::try_parse_from([
            "kalam",
            "invite",
            "--email",
            "alice@example.com",
            "--role",
            "dba",
            "--expires-in-days",
            "14",
        ])
        .expect("invite should parse");

        let Some(CliCommand::Invite(args)) = cli.subcommand else {
            panic!("expected invite command");
        };
        assert_eq!(args.email, "alice@example.com");
        assert_eq!(args.role, TokenRole::Dba);
        assert_eq!(args.expires_in_days, 14);
    }

    #[test]
    fn utility_subcommands_parse() {
        for args in [
            &["doctor"][..],
            &["logout"][..],
            &["whoami"][..],
            &["update", "--dry-run"][..],
        ] {
            Cli::try_parse_from(std::iter::once("kalam").chain(args.iter().copied()))
                .expect("utility command should parse");
        }
    }

    #[test]
    fn dev_lifecycle_subcommands_parse() {
        let start = Cli::try_parse_from(["kalam", "dev", "start", "--agent", "--force"])
            .expect("dev start should parse");
        let Some(CliCommand::Dev(args)) = start.subcommand else {
            panic!("expected dev command");
        };
        assert!(start.agent);
        assert!(args.force);
        assert!(matches!(args.command, Some(DevCommand::Start)));

        let before_flag = Cli::try_parse_from(["kalam", "dev", "--agent", "status"])
            .expect("dev --agent status should parse");
        let Some(CliCommand::Dev(args)) = before_flag.subcommand else {
            panic!("expected dev command");
        };
        assert!(before_flag.agent);
        assert!(matches!(args.command, Some(DevCommand::Status)));

        let logs = Cli::try_parse_from(["kalam", "dev", "logs", "--follow", "-n", "20"])
            .expect("dev logs should parse");
        let Some(CliCommand::Dev(args)) = logs.subcommand else {
            panic!("expected dev command");
        };
        match args.command {
            Some(DevCommand::Logs(logs_args)) => {
                assert!(logs_args.follow);
                assert_eq!(logs_args.lines, 20);
            },
            other => panic!("expected logs command, got {other:?}"),
        }

        let stop = Cli::try_parse_from(["kalam", "dev", "stop"]).expect("dev stop should parse");
        let Some(CliCommand::Dev(args)) = stop.subcommand else {
            panic!("expected dev command");
        };
        assert!(matches!(args.command, Some(DevCommand::Stop)));
    }

    #[test]
    fn lifecycle_commands_and_global_flag_parse() {
        for alias in ["instances", "servers", "list"] {
            assert!(Cli::try_parse_from(["kalam", alias, "--local"]).is_ok());
            assert!(Cli::try_parse_from(["kalam", alias, "--cloud", "--check"]).is_ok());
        }
        assert!(Cli::try_parse_from(["kalam", "instances", "--local", "--cloud"]).is_err());

        assert!(Cli::try_parse_from(["kalam", "servers"]).is_ok());
        let up = Cli::try_parse_from(["kalam", "up", "-g"]).expect("up -g should parse");
        assert!(up.global);
        assert!(matches!(up.subcommand, Some(CliCommand::Up(_))));

        let down = Cli::try_parse_from(["kalam", "down", "--global"]).expect("down should parse");
        assert!(down.global);
        assert!(matches!(down.subcommand, Some(CliCommand::Down(_))));

        let logs =
            Cli::try_parse_from(["kalam", "logs", "--tail"]).expect("logs --tail should parse");
        let Some(CliCommand::Logs(args)) = logs.subcommand else {
            panic!("expected logs command");
        };
        assert!(args.follow);

        let status = Cli::try_parse_from(["kalam", "status", "--env", "staging"])
            .expect("status --env should parse");
        assert_eq!(status.env.as_deref(), Some("staging"));
        assert!(matches!(status.subcommand, Some(CliCommand::Status(_))));
    }

    #[test]
    fn global_and_env_are_accepted_separately() {
        Cli::try_parse_from(["kalam", "up", "--env", "dev"]).expect("up --env parses");
        Cli::try_parse_from(["kalam", "dev", "--exec", "npm test"]).expect("dev --exec parses");
    }

    #[test]
    fn removed_legacy_flags_are_rejected() {
        assert!(Cli::try_parse_from(["kalam", "--show-credentials"]).is_err());
        assert!(Cli::try_parse_from(["kalam", "--update-credentials"]).is_err());
        assert!(Cli::try_parse_from(["kalam", "--delete-credentials"]).is_err());
        assert!(Cli::try_parse_from(["kalam", "dev", "--progress"]).is_err());
        assert!(Cli::try_parse_from(["kalam", "init", "--schema-mode", "remote"]).is_err());
        assert!(Cli::try_parse_from(["kalam", "schema", "pull"]).is_err());
        assert!(Cli::try_parse_from(["kalam", "migration", "status"]).is_err());
    }
}
