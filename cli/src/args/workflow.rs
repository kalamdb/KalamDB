use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use kalam_cli::workflow::project::config::SchemaMode;

#[derive(Args, Debug, Clone, Default)]
pub struct InitArgs {
    /// Project name
    #[arg(long = "name")]
    pub name: Option<String>,

    /// Schema source mode (SQL files)
    #[arg(long = "schema-mode", value_enum)]
    pub schema_mode: Option<SchemaModeArg>,

    /// Comma-separated generated language targets (typescript, dart/flutter)
    #[arg(long = "languages", value_delimiter = ',')]
    pub languages: Option<Vec<String>>,

    /// Project template or repository example id (for example simple-live or chat-with-ai)
    #[arg(long = "template")]
    pub template: Option<String>,

    /// List embedded templates and repository examples, then exit
    #[arg(long = "list-templates")]
    pub list_templates: bool,

    /// JavaScript package manager for TypeScript projects (npm, pnpm, yarn, bun)
    #[arg(long = "package-manager", value_enum)]
    pub package_manager: Option<PackageManagerArg>,

    /// Non-interactive mode (use defaults for unspecified values)
    #[arg(long = "yes")]
    pub yes: bool,

    /// Local KalamDB server management during kalam dev (local starts server, remote uses existing
    /// URL)
    #[arg(long = "server-mode", value_enum)]
    pub server_mode: Option<ServerModeArg>,

    /// KalamDB server URL for remote server mode
    #[arg(long = "server-url")]
    pub server_url: Option<String>,

    /// Project directory to initialize (defaults to current directory)
    #[arg(long = "project-dir")]
    pub project_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PackageManagerArg {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl From<PackageManagerArg> for kalam_cli::workflow::project::ts::PackageManager {
    fn from(value: PackageManagerArg) -> Self {
        match value {
            PackageManagerArg::Npm => Self::Npm,
            PackageManagerArg::Pnpm => Self::Pnpm,
            PackageManagerArg::Yarn => Self::Yarn,
            PackageManagerArg::Bun => Self::Bun,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ServerModeArg {
    Local,
    Remote,
}

impl From<ServerModeArg> for kalam_cli::workflow::project::init::ServerMode {
    fn from(value: ServerModeArg) -> Self {
        match value {
            ServerModeArg::Local => kalam_cli::workflow::project::init::ServerMode::Local,
            ServerModeArg::Remote => kalam_cli::workflow::project::init::ServerMode::Remote,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SchemaModeArg {
    Sql,
}

impl From<SchemaModeArg> for SchemaMode {
    fn from(value: SchemaModeArg) -> Self {
        match value {
            SchemaModeArg::Sql => SchemaMode::Sql,
        }
    }
}

#[derive(Args, Debug, Clone, Default)]
pub struct LinkArgs {
    /// Namespace to associate with the linked environment
    #[arg(long = "namespace")]
    pub namespace: Option<String>,

    /// KalamDB server URL for the environment
    #[arg(long = "url")]
    pub url: Option<String>,

    /// Project directory containing kalam.toml
    #[arg(long = "project-dir", global = true)]
    pub project_dir: Option<PathBuf>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DevArgs {
    #[command(subcommand)]
    pub command: Option<DevCommand>,

    /// Project directory containing kalam.toml
    #[arg(long = "project-dir", global = true)]
    pub project_dir: Option<PathBuf>,

    /// Namespace override for the resolved environment
    #[arg(long = "namespace")]
    pub namespace: Option<String>,

    /// Retry a paused schema pipeline on startup
    #[arg(long = "force", global = true)]
    pub force: bool,

    /// Run a command against an isolated temporary database, then exit with its status
    #[arg(long = "exec")]
    pub exec: Option<String>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DevCommand {
    /// Start the development environment in the background
    Start,
    /// Show whether a background `kalam dev` session is running
    Status,
    /// Print or follow logs from a background `kalam dev` session
    Logs(DevLogsArgs),
    /// Stop a background `kalam dev` session
    Stop,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DevLogsArgs {
    /// Follow the log file until interrupted
    #[arg(short = 'F', long = "follow", visible_alias = "tail")]
    pub follow: bool,

    /// Number of trailing lines to print (0 prints the full file)
    #[arg(short = 'n', long = "lines", default_value_t = 200)]
    pub lines: usize,
}

#[derive(Args, Debug, Clone, Default)]
pub struct UpArgs {
    /// Project directory for a project-local database
    #[arg(long = "project-dir", global = true)]
    pub project_dir: Option<PathBuf>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DownArgs {
    /// Project directory for a project-local database
    #[arg(long = "project-dir", global = true)]
    pub project_dir: Option<PathBuf>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct LogsArgs {
    /// Read the local capture file instead of querying authenticated SQL logs
    #[arg(long)]
    pub local_file: bool,

    /// Project directory for a project-local database
    #[arg(long = "project-dir", global = true)]
    pub project_dir: Option<PathBuf>,

    /// Follow the log file until interrupted
    #[arg(short = 'F', long = "follow", visible_alias = "tail")]
    pub follow: bool,

    /// Number of trailing lines to print (0 prints the full file)
    #[arg(short = 'n', long = "lines", default_value_t = 200)]
    pub lines: usize,
}

#[derive(Args, Debug, Clone, Default)]
pub struct StatusArgs {
    /// Project directory containing kalam.toml
    #[arg(long = "project-dir", global = true)]
    pub project_dir: Option<PathBuf>,

    /// Namespace override for the resolved environment
    #[arg(long = "namespace")]
    pub namespace: Option<String>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DeployArgs {
    /// Project directory containing kalam.toml
    #[arg(long = "project-dir", global = true)]
    pub project_dir: Option<PathBuf>,

    /// Validate readiness without applying migrations or activating procedures
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

#[derive(Args, Debug, Clone)]
pub struct SchemaArgs {
    #[command(subcommand)]
    pub command: SchemaCommand,

    /// Project directory containing kalam.toml
    #[arg(long = "project-dir", global = true)]
    pub project_dir: Option<PathBuf>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum SchemaCommand {
    /// Generate SDK and schema artifacts from the project schema source
    Gen(SchemaGenerateArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct SchemaGenerateArgs {
    /// Limit generation to specific language targets (typescript, dart/flutter)
    #[arg(long = "languages", value_delimiter = ',')]
    pub languages: Option<Vec<String>>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum MigrationCommand {
    /// Create a new ordered migration from the current schema changes
    Create(MigrationCreateArgs),
    /// Show migration state for the current project
    Status(MigrationStatusArgs),
    /// Create a numbered migration from kalam/migrations/_draft.sql
    Seal(MigrationSealArgs),
    /// Retry a failed migration
    Retry(MigrationRetryArgs),
    /// Repair migration state manually
    Repair(MigrationRepairArgs),
}

#[derive(Args, Debug, Clone)]
pub struct MigrationCreateArgs {
    /// Migration name
    pub name: String,
}

#[derive(Args, Debug, Clone, Default)]
pub struct MigrationStatusArgs {}

#[derive(Args, Debug, Clone, Default)]
pub struct MigrationSealArgs {}

#[derive(Args, Debug, Clone)]
pub struct MigrationRetryArgs {
    /// Migration id or filename to retry
    pub migration_id: String,
}

#[derive(Args, Debug, Clone)]
pub struct MigrationRepairArgs {
    /// Migration id or filename to repair
    pub migration_id: String,

    /// Mark this migration as applied
    #[arg(long = "mark-applied")]
    pub mark_applied: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DbArgs {
    #[command(subcommand)]
    pub command: DbCommand,

    /// Project directory containing kalam.toml
    #[arg(long = "project-dir", global = true)]
    pub project_dir: Option<PathBuf>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DbCommand {
    /// Apply the committed migration history to the linked database
    Migrate(DbMigrateArgs),

    /// Rebuild the local database from committed migrations and development fixtures
    Reset(DbResetArgs),

    /// Rerun development fixtures from kalam/seed.sql
    Seed,

    /// Create and inspect schema migration history
    Migration(DbMigrationArgs),
}

#[derive(Args, Debug, Clone)]
pub struct DbMigrationArgs {
    #[command(subcommand)]
    pub command: MigrationCommand,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DbResetArgs {
    /// Skip confirmation prompts (local reset does not drop remote namespaces)
    #[arg(long)]
    pub yes: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DbMigrateArgs {}

#[derive(Args, Debug, Clone)]
pub struct FunctionsArgs {
    #[command(subcommand)]
    pub command: FunctionsCommand,

    /// Project directory containing kalam.toml
    #[arg(long = "project-dir", global = true)]
    pub project_dir: Option<PathBuf>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum FunctionsCommand {
    /// Generate function contracts, validate packages, and write the build manifest
    Build,
    /// Show the active function module, procedures, and runtime health
    Status,
    /// List function module revisions (active and ready)
    Revisions,
    /// Point the active revision at a previously activated hash (CAS, no rebuild)
    Rollback(FunctionsRollbackArgs),
    /// Print recent structured function errors
    Logs(FunctionsLogsArgs),
    /// Show resident isolates, memory reservations, and in-flight calls
    Runtime,
    /// Scaffold a project implementation that overrides an inline procedure
    Override(FunctionsOverrideArgs),
}

#[derive(Args, Debug, Clone)]
pub struct FunctionsRollbackArgs {
    pub revision: String,
}

#[derive(Args, Debug, Clone, Default)]
pub struct FunctionsLogsArgs {
    pub procedure: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct FunctionsOverrideArgs {
    /// Procedure to scaffold, as `schema.name`
    pub procedure: String,
}

#[derive(Args, Debug, Clone, Default)]
pub struct InstancesArgs {
    /// Show only managed local servers
    #[arg(long, conflicts_with = "cloud")]
    pub local: bool,
    /// Show only saved cloud connections
    #[arg(long)]
    pub cloud: bool,
    /// Check cloud endpoint reachability without sending credentials
    #[arg(long)]
    pub check: bool,
}
