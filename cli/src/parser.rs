//! Command parser for SQL and backslash commands
//!
//! **Implements T087**: CommandParser for SQL + backslash commands
//!
//! Parses user input to distinguish between SQL statements and CLI meta-commands.

use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
};

use clap::{Args, CommandFactory, Parser as ClapParser, Subcommand, ValueEnum};
use console::{pad_str, style, Alignment};
use shlex::split as split_shell_words;

use crate::error::{CLIError, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum FlushTarget {
    All,
    Table(String),
}

/// Parsed command
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// SQL statement
    Sql(String),

    /// Execute a single SQL statement as another user
    ExecuteAs {
        user: String,
        sql: String,
    },

    /// Meta-commands (backslash commands)
    Quit,
    Help,
    Flush(FlushTarget),
    ClusterSnapshot,
    ClusterPurge {
        upto: u64,
    },
    ClusterTriggerElection,
    ClusterTransferLeader {
        node_id: u64,
    },
    ClusterStepdown,
    ClusterClear,
    ClusterList,
    ClusterListGroups,
    ClusterJoin {
        node_id: u64,
        rpc_addr: String,
        api_addr: String,
    },
    ClusterRebalance,
    Health,
    ListTables,
    Describe(String),
    SetFormat(String),
    Subscribe(String),
    RefreshTables,
    ShowCredentials,
    UpdateCredentials {
        username: String,
        password: String,
    },
    DeleteCredentials,
    Info,
    Sessions,
    /// Show system statistics (from system.stats)
    Stats,
    /// Open interactive history menu
    History,
    /// Consume messages from a topic
    Consume {
        topic: String,
        group: Option<String>,
        from: Option<String>,
        limit: Option<usize>,
        timeout: Option<u64>,
    },
    Unknown(String),

    /// Export a table to a ZIP archive
    ExportTable {
        table: String,
        user_id: Option<String>,
        output: Option<String>,
    },
    /// Import a table from a ZIP archive
    ImportTable {
        table: String,
        file: String,
        user_id: Option<String>,
    },
}

#[derive(Debug)]
struct MetaCommandCatalog {
    meta_completions: Vec<String>,
    known_commands: HashSet<String>,
    external_commands: HashSet<String>,
    descriptions: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy)]
struct HelpEntry {
    command_key: &'static str,
    syntax: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct HelpSection {
    title: &'static str,
    entries: &'static [HelpEntry],
    notes: &'static [&'static str],
    examples: &'static [&'static str],
}

static META_COMMAND_CATALOG: OnceLock<MetaCommandCatalog> = OnceLock::new();

const HELP_EMPTY_LINES: &[&str] = &[];

const HELP_META_COMMANDS: &[HelpEntry] = &[
    HelpEntry {
        command_key: "help",
        syntax: "\\help, \\?",
    },
    HelpEntry {
        command_key: "quit",
        syntax: "\\quit, \\q",
    },
    HelpEntry {
        command_key: "info",
        syntax: "\\info, \\session",
    },
    HelpEntry {
        command_key: "history",
        syntax: "\\history, \\h",
    },
    HelpEntry {
        command_key: "health",
        syntax: "\\health",
    },
    HelpEntry {
        command_key: "tables",
        syntax: "\\dt, \\tables",
    },
    HelpEntry {
        command_key: "describe",
        syntax: "\\d, \\describe <table>",
    },
    HelpEntry {
        command_key: "as",
        syntax: "\\as <user_id> <SQL>",
    },
    HelpEntry {
        command_key: "format",
        syntax: "\\format <table|json|csv>",
    },
    HelpEntry {
        command_key: "refresh-tables",
        syntax: "\\refresh-tables, \\refresh",
    },
    HelpEntry {
        command_key: "stats",
        syntax: "\\stats, \\metrics",
    },
    HelpEntry {
        command_key: "sessions",
        syntax: "\\sessions",
    },
    HelpEntry {
        command_key: "flush",
        syntax: "\\flush [all|table <table>]",
    },
    HelpEntry {
        command_key: "cluster",
        syntax: "\\cluster <subcommand>",
    },
    HelpEntry {
        command_key: "consume",
        syntax: "\\consume <topic>",
    },
    HelpEntry {
        command_key: "export",
        syntax: "\\export <namespace.table> [--user-id <id>] [--output <file.zip>]",
    },
    HelpEntry {
        command_key: "import",
        syntax: "\\import <namespace.table> <file.zip> [--user-id <id>]",
    },
];

const HELP_LIVE_QUERIES: &[HelpEntry] = &[
    HelpEntry {
        command_key: "subscribe",
        syntax: "\\live <SELECT ...>",
    },
    HelpEntry {
        command_key: "subscribe",
        syntax: "\\subscribe <SELECT ...>",
    },
];

const HELP_CLUSTER_COMMANDS: &[HelpEntry] = &[
    HelpEntry {
        command_key: "cluster list",
        syntax: "\\cluster list",
    },
    HelpEntry {
        command_key: "cluster list groups",
        syntax: "\\cluster list groups",
    },
    HelpEntry {
        command_key: "cluster snapshot",
        syntax: "\\cluster snapshot",
    },
    HelpEntry {
        command_key: "cluster purge",
        syntax: "\\cluster purge --upto <index>",
    },
    HelpEntry {
        command_key: "cluster trigger-election",
        syntax: "\\cluster trigger-election",
    },
    HelpEntry {
        command_key: "cluster transfer-leader",
        syntax: "\\cluster transfer-leader <node_id>",
    },
    HelpEntry {
        command_key: "cluster rebalance",
        syntax: "\\cluster rebalance",
    },
    HelpEntry {
        command_key: "cluster stepdown",
        syntax: "\\cluster stepdown",
    },
    HelpEntry {
        command_key: "cluster clear",
        syntax: "\\cluster clear",
    },
    HelpEntry {
        command_key: "cluster join",
        syntax: "\\cluster join <id> <rpc> <api>",
    },
];

const HELP_CREDENTIALS: &[HelpEntry] = &[
    HelpEntry {
        command_key: "show-credentials",
        syntax: "\\show-credentials, \\credentials",
    },
    HelpEntry {
        command_key: "update-credentials",
        syntax: "\\update-credentials <user> <password>",
    },
    HelpEntry {
        command_key: "delete-credentials",
        syntax: "\\delete-credentials",
    },
];

const HELP_TOPIC_CONSUMPTION: &[HelpEntry] = &[
    HelpEntry {
        command_key: "consume",
        syntax: "\\consume app.events",
    },
    HelpEntry {
        command_key: "consume",
        syntax: "\\consume app.events --group my-group",
    },
    HelpEntry {
        command_key: "consume",
        syntax: "\\consume app.events --from earliest --limit 10",
    },
];

const HELP_SECTIONS: &[HelpSection] = &[
    HelpSection {
        title: "Meta Commands",
        entries: HELP_META_COMMANDS,
        notes: HELP_EMPTY_LINES,
        examples: HELP_EMPTY_LINES,
    },
    HelpSection {
        title: "Live Queries",
        entries: HELP_LIVE_QUERIES,
        notes: &["system.* tables are not subscribable."],
        examples: &["\\live SELECT * FROM chat.messages;"],
    },
    HelpSection {
        title: "Cluster Commands",
        entries: HELP_CLUSTER_COMMANDS,
        notes: HELP_EMPTY_LINES,
        examples: HELP_EMPTY_LINES,
    },
    HelpSection {
        title: "Credentials",
        entries: HELP_CREDENTIALS,
        notes: HELP_EMPTY_LINES,
        examples: HELP_EMPTY_LINES,
    },
    HelpSection {
        title: "Topic Consumption",
        entries: HELP_TOPIC_CONSUMPTION,
        notes: &["CLI args: kalam --consume --topic app.events --group my-group"],
        examples: HELP_EMPTY_LINES,
    },
];

#[derive(Debug, ClapParser)]
#[command(
    name = "kalam-meta",
    no_binary_name = true,
    disable_help_flag = true,
    disable_version_flag = true,
    disable_help_subcommand = true
)]
struct MetaCli {
    #[command(subcommand)]
    command: MetaCommand,
}

#[derive(Debug, Subcommand)]
enum MetaCommand {
    #[command(name = "quit", alias = "q", about = "Exit CLI")]
    Quit,
    #[command(name = "help", alias = "?", about = "Show this help")]
    Help,
    #[command(name = "sessions", about = "Show active sessions")]
    Sessions,
    #[command(name = "stats", alias = "metrics", about = "Show system stats")]
    Stats,
    #[command(
        name = "flush",
        about = "Run STORAGE FLUSH using the current namespace"
    )]
    Flush(FlushArgs),
    #[command(name = "cluster", about = "Cluster operations")]
    Cluster(ClusterArgs),
    #[command(name = "health", about = "Run public health probes")]
    Health,
    #[command(name = "tables", alias = "dt", about = "List tables")]
    ListTables,
    #[command(name = "describe", alias = "d", about = "Describe a table")]
    Describe(TrailingValueArgs),
    #[command(name = "format", about = "Change output format")]
    Format(FormatArgs),
    #[command(name = "subscribe", aliases = ["watch", "live"], about = "Start a live query")]
    Subscribe(TrailingValueArgs),
    #[command(
        name = "refresh-tables",
        alias = "refresh",
        about = "Refresh autocomplete caches"
    )]
    RefreshTables,
    #[command(
        name = "show-credentials",
        alias = "credentials",
        about = "Show stored credentials"
    )]
    ShowCredentials,
    #[command(name = "update-credentials", about = "Update stored credentials")]
    UpdateCredentials(UpdateCredentialsArgs),
    #[command(name = "delete-credentials", about = "Delete stored credentials")]
    DeleteCredentials,
    #[command(name = "info", alias = "session", about = "Show session details")]
    Info,
    #[command(name = "history", alias = "h", about = "Browse command history")]
    History,
    #[command(name = "consume", about = "Consume topic messages")]
    Consume(ConsumeArgs),
    #[command(name = "export", about = "Export a table to a ZIP archive")]
    Export(ExportTableArgs),
    #[command(name = "import", about = "Import a table from a ZIP archive")]
    Import(ImportTableArgs),
}

#[derive(Debug, Args)]
struct FlushArgs {
    #[command(subcommand)]
    target: Option<FlushCommand>,
}

#[derive(Debug, Subcommand)]
enum FlushCommand {
    #[command(name = "all", about = "Flush all tables in the current namespace")]
    All,
    #[command(name = "table", about = "Flush one table in the current namespace")]
    Table(TrailingValueArgs),
}

#[derive(Debug, Args)]
struct ClusterArgs {
    #[command(subcommand)]
    command: ClusterCommand,
}

#[derive(Debug, Subcommand)]
enum ClusterCommand {
    #[command(name = "snapshot", about = "Trigger a snapshot")]
    Snapshot,
    #[command(name = "purge", about = "Purge raft logs")]
    Purge(ClusterPurgeArgs),
    #[command(name = "trigger-election", about = "Trigger an election")]
    TriggerElection,
    #[command(name = "trigger", about = "Trigger cluster maintenance operations")]
    Trigger(ClusterTriggerArgs),
    #[command(name = "transfer-leader", about = "Transfer leadership")]
    TransferLeader(ClusterTransferLeaderArgs),
    #[command(name = "transfer", about = "Transfer cluster leadership")]
    Transfer(ClusterTransferArgs),
    #[command(name = "rebalance", about = "Rebalance leaders")]
    Rebalance,
    #[command(name = "stepdown", alias = "step-down", about = "Leader stepdown")]
    Stepdown,
    #[command(name = "clear", about = "Clear old snapshots")]
    Clear,
    #[command(name = "list", alias = "ls", about = "List cluster nodes")]
    List(ClusterListArgs),
    #[command(name = "join", about = "Join a node at runtime")]
    Join(ClusterJoinArgs),
}

#[derive(Debug, Args)]
struct ClusterPurgeArgs {
    #[arg(long = "upto", conflicts_with = "index")]
    upto: Option<u64>,

    #[arg(value_name = "INDEX")]
    index: Option<u64>,
}

#[derive(Debug, Args)]
struct ClusterTriggerArgs {
    #[command(subcommand)]
    command: ClusterTriggerCommand,
}

#[derive(Debug, Subcommand)]
enum ClusterTriggerCommand {
    #[command(name = "election", about = "Trigger an election")]
    Election,
}

#[derive(Debug, Args)]
struct ClusterTransferLeaderArgs {
    node_id: u64,
}

#[derive(Debug, Args)]
struct ClusterTransferArgs {
    #[command(subcommand)]
    command: ClusterTransferCommand,
}

#[derive(Debug, Subcommand)]
enum ClusterTransferCommand {
    #[command(name = "leader", about = "Transfer leadership")]
    Leader(ClusterTransferLeaderArgs),
}

#[derive(Debug, Args)]
struct ClusterListArgs {
    #[command(subcommand)]
    target: Option<ClusterListTarget>,
}

#[derive(Debug, Subcommand)]
enum ClusterListTarget {
    #[command(name = "groups", about = "List raft groups")]
    Groups,
}

#[derive(Debug, Args)]
struct ClusterJoinArgs {
    node_id: u64,
    rpc_addr: String,
    api_addr: String,
}

#[derive(Debug, Args)]
struct FormatArgs {
    #[arg(value_enum)]
    format: MetaOutputFormat,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum MetaOutputFormat {
    Table,
    Json,
    Csv,
}

impl MetaOutputFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Json => "json",
            Self::Csv => "csv",
        }
    }
}

#[derive(Debug, Args)]
struct UpdateCredentialsArgs {
    username: String,
    password: String,
}

#[derive(Debug, Args)]
struct ConsumeArgs {
    topic: String,

    #[arg(long = "group")]
    group: Option<String>,

    #[arg(long = "from")]
    from: Option<String>,

    #[arg(long = "limit")]
    limit: Option<usize>,

    #[arg(long = "timeout")]
    timeout: Option<u64>,
}

#[derive(Debug, Args)]
struct ExportTableArgs {
    #[arg(value_name = "TABLE")]
    table: String,
    #[arg(long = "user-id")]
    user_id: Option<String>,
    #[arg(long = "output", short = 'o')]
    output: Option<String>,
}

#[derive(Debug, Args)]
struct ImportTableArgs {
    #[arg(value_name = "TABLE")]
    table: String,
    #[arg(value_name = "FILE")]
    file: String,
    #[arg(long = "user-id")]
    user_id: Option<String>,
}

#[derive(Debug, Args)]
struct TrailingValueArgs {
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    values: Vec<String>,
}

impl TrailingValueArgs {
    fn joined(self) -> String {
        self.values.join(" ")
    }
}

impl MetaCommand {
    fn into_command(self) -> Result<Command> {
        match self {
            Self::Quit => Ok(Command::Quit),
            Self::Help => Ok(Command::Help),
            Self::Sessions => Ok(Command::Sessions),
            Self::Stats => Ok(Command::Stats),
            Self::Flush(args) => args.into_command(),
            Self::Cluster(args) => args.into_command(),
            Self::Health => Ok(Command::Health),
            Self::ListTables => Ok(Command::ListTables),
            Self::Describe(args) => Ok(Command::Describe(args.joined())),
            Self::Format(args) => Ok(Command::SetFormat(args.format.as_str().to_string())),
            Self::Subscribe(args) => Ok(Command::Subscribe(args.joined())),
            Self::RefreshTables => Ok(Command::RefreshTables),
            Self::ShowCredentials => Ok(Command::ShowCredentials),
            Self::UpdateCredentials(args) => Ok(Command::UpdateCredentials {
                username: args.username,
                password: args.password,
            }),
            Self::DeleteCredentials => Ok(Command::DeleteCredentials),
            Self::Info => Ok(Command::Info),
            Self::History => Ok(Command::History),
            Self::Consume(args) => Ok(Command::Consume {
                topic: args.topic,
                group: args.group,
                from: args.from,
                limit: args.limit,
                timeout: args.timeout,
            }),
            Self::Export(args) => Ok(Command::ExportTable {
                table: args.table,
                user_id: args.user_id,
                output: args.output,
            }),
            Self::Import(args) => Ok(Command::ImportTable {
                table: args.table,
                file: args.file,
                user_id: args.user_id,
            }),
        }
    }
}

impl FlushArgs {
    fn into_command(self) -> Result<Command> {
        match self.target {
            None | Some(FlushCommand::All) => Ok(Command::Flush(FlushTarget::All)),
            Some(FlushCommand::Table(args)) => {
                Ok(Command::Flush(FlushTarget::Table(args.joined())))
            },
        }
    }
}

impl ClusterArgs {
    fn into_command(self) -> Result<Command> {
        match self.command {
            ClusterCommand::Snapshot => Ok(Command::ClusterSnapshot),
            ClusterCommand::Purge(args) => args.into_command(),
            ClusterCommand::TriggerElection => Ok(Command::ClusterTriggerElection),
            ClusterCommand::Trigger(args) => args.into_command(),
            ClusterCommand::TransferLeader(args) => Ok(Command::ClusterTransferLeader {
                node_id: args.node_id,
            }),
            ClusterCommand::Transfer(args) => args.into_command(),
            ClusterCommand::Rebalance => Ok(Command::ClusterRebalance),
            ClusterCommand::Stepdown => Ok(Command::ClusterStepdown),
            ClusterCommand::Clear => Ok(Command::ClusterClear),
            ClusterCommand::List(args) => args.into_command(),
            ClusterCommand::Join(args) => Ok(Command::ClusterJoin {
                node_id: args.node_id,
                rpc_addr: args.rpc_addr,
                api_addr: args.api_addr,
            }),
        }
    }
}

impl ClusterPurgeArgs {
    fn into_command(self) -> Result<Command> {
        let Some(upto) = self.upto.or(self.index) else {
            return Err(CLIError::ParseError(
                "\\cluster purge requires --upto <index> or a numeric index".into(),
            ));
        };
        Ok(Command::ClusterPurge { upto })
    }
}

impl ClusterTriggerArgs {
    fn into_command(self) -> Result<Command> {
        match self.command {
            ClusterTriggerCommand::Election => Ok(Command::ClusterTriggerElection),
        }
    }
}

impl ClusterTransferArgs {
    fn into_command(self) -> Result<Command> {
        match self.command {
            ClusterTransferCommand::Leader(args) => Ok(Command::ClusterTransferLeader {
                node_id: args.node_id,
            }),
        }
    }
}

impl ClusterListArgs {
    fn into_command(self) -> Result<Command> {
        match self.target {
            None => Ok(Command::ClusterList),
            Some(ClusterListTarget::Groups) => Ok(Command::ClusterListGroups),
        }
    }
}

fn meta_command_catalog() -> &'static MetaCommandCatalog {
    META_COMMAND_CATALOG.get_or_init(build_meta_command_catalog)
}

fn build_meta_command_catalog() -> MetaCommandCatalog {
    let mut root = MetaCli::command();
    root.build();

    let mut meta_completions = Vec::new();
    let mut known_commands = HashSet::new();
    let mut external_commands = HashSet::new();
    let mut descriptions = HashMap::new();

    for subcommand in root.get_subcommands() {
        collect_command_descriptions(None, subcommand, &mut descriptions);
        register_command_name(
            subcommand.get_name(),
            &mut meta_completions,
            &mut known_commands,
            &mut external_commands,
        );

        for alias in subcommand.get_all_aliases() {
            register_command_name(
                alias,
                &mut meta_completions,
                &mut known_commands,
                &mut external_commands,
            );
        }
    }

    register_custom_command(
        "as",
        "Wrap a statement as EXECUTE AS '<user_id>'",
        &mut meta_completions,
        &mut known_commands,
        &mut external_commands,
        &mut descriptions,
    );

    MetaCommandCatalog {
        meta_completions,
        known_commands,
        external_commands,
        descriptions,
    }
}

fn collect_command_descriptions(
    parent_path: Option<&str>,
    command: &clap::Command,
    descriptions: &mut HashMap<String, String>,
) {
    let path = match parent_path {
        Some(parent_path) => format!("{parent_path} {}", command.get_name()),
        None => command.get_name().to_string(),
    };

    if let Some(about) = command.get_about() {
        descriptions.insert(path.clone(), about.to_string());
    }

    for subcommand in command.get_subcommands() {
        collect_command_descriptions(Some(&path), subcommand, descriptions);
    }
}

fn register_command_name(
    name: &str,
    meta_completions: &mut Vec<String>,
    known_commands: &mut HashSet<String>,
    external_commands: &mut HashSet<String>,
) {
    if known_commands.insert(name.to_string()) {
        meta_completions.push(format!("\\{name}"));
    }
    external_commands.insert(name.to_string());
}

fn register_custom_command(
    name: &str,
    description: &str,
    meta_completions: &mut Vec<String>,
    known_commands: &mut HashSet<String>,
    external_commands: &mut HashSet<String>,
    descriptions: &mut HashMap<String, String>,
) {
    register_command_name(name, meta_completions, known_commands, external_commands);
    descriptions.insert(name.to_string(), description.to_string());
}

fn is_known_meta_command(name: &str) -> bool {
    meta_command_catalog().known_commands.contains(name)
}

fn is_external_command(name: &str) -> bool {
    meta_command_catalog().external_commands.contains(name)
}

pub(crate) fn meta_command_completions() -> &'static [String] {
    meta_command_catalog().meta_completions.as_slice()
}

fn meta_command_description(command_key: &str) -> Option<&str> {
    meta_command_catalog()
        .descriptions
        .get(command_key)
        .map(|description| description.as_str())
}

fn format_help_heading(text: &str, color_enabled: bool) -> String {
    if color_enabled {
        style(text).yellow().bold().to_string()
    } else {
        text.to_string()
    }
}

fn format_help_command(text: &str, color_enabled: bool) -> String {
    let padded = pad_str(text, 38, Alignment::Left, None).into_owned();
    if color_enabled {
        style(padded).cyan().to_string()
    } else {
        padded
    }
}

fn format_help_example(text: &str, color_enabled: bool) -> String {
    if color_enabled {
        style(text).green().to_string()
    } else {
        text.to_string()
    }
}

fn format_help_note(text: &str, color_enabled: bool) -> String {
    if color_enabled {
        style(text).dim().to_string()
    } else {
        text.to_string()
    }
}

pub(crate) fn render_meta_command_help(color_enabled: bool) -> String {
    let mut help = String::new();

    help.push('\n');
    if color_enabled {
        help.push_str(&style("Kalam CLI Help").blue().bold().to_string());
    } else {
        help.push_str("Kalam CLI Help");
    }
    help.push_str("\n\n");

    help.push_str(&format!("{}\n", format_help_heading("Basics", color_enabled)));
    help.push_str("  Write SQL and end with ';' to run it\n");
    help.push_str("  Press Tab for SQL, table, namespace, and command completion\n");
    help.push_str("  Press Up on an empty prompt to open command history\n\n");

    for section in HELP_SECTIONS {
        help.push_str(&format!("{}\n", format_help_heading(section.title, color_enabled)));
        for entry in section.entries {
            let description = meta_command_description(entry.command_key)
                .unwrap_or("Missing help metadata for meta command");
            help.push_str(&format!(
                "  {} {}\n",
                format_help_command(entry.syntax, color_enabled),
                description
            ));
        }

        for example in section.examples {
            help.push_str(&format!("  {}\n", format_help_example(example, color_enabled)));
        }

        for note in section.notes {
            help.push_str(&format!("  {}\n", format_help_note(note, color_enabled)));
        }

        help.push('\n');
    }

    help.push_str(&format!("{}\n", format_help_heading("Backup and Export SQL", color_enabled)));
    help.push_str("  Run these as normal SQL statements:\n");
    for example in [
        "BACKUP DATABASE TO '/tmp/kalamdb-backup.tar.gz';",
        "EXPORT USER DATA;",
        "SHOW EXPORT;",
    ] {
        help.push_str(&format!("  {}\n", format_help_example(example, color_enabled)));
    }
    help.push_str(&format!(
        "  {}\n\n",
        format_help_note(
            "SHOW EXPORT returns a download_url for the finished user export.",
            color_enabled,
        )
    ));

    help.push_str(&format!("{}\n", format_help_heading("Examples", color_enabled)));
    for example in [
        "USE NAMESPACE chat;",
        "SELECT * FROM system.tables LIMIT 5;",
        "\\flush table messages",
        "\\describe chat.messages;",
        "\\as user_123 SELECT * FROM user.orders LIMIT 5;",
        "\\cluster list",
    ] {
        help.push_str(&format!("  {}\n", format_help_example(example, color_enabled)));
    }
    help.push('\n');

    help
}

/// Command parser
pub struct CommandParser;

impl CommandParser {
    /// Create a new parser
    pub fn new() -> Self {
        Self
    }

    /// Parse a command line
    pub fn parse(&self, line: &str) -> Result<Command> {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            return Err(CLIError::ParseError("Empty command".into()));
        }

        // Check for backslash commands
        if trimmed.starts_with('\\') {
            return self.parse_meta_command(trimmed);
        }

        // Otherwise, treat as SQL
        Ok(Command::Sql(trimmed.to_string()))
    }

    pub fn parse_external_command(&self, line: &str) -> Result<Command> {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            return Err(CLIError::ParseError("Empty command".into()));
        }

        if trimmed.starts_with('\\') {
            return self.parse_meta_command(trimmed);
        }

        let tokens = Self::tokenize_command_line(trimmed)?;
        let Some(command_name) = tokens.first().map(String::as_str) else {
            return Err(CLIError::ParseError("Invalid command".into()));
        };

        if !is_external_command(command_name) {
            return Ok(Command::Sql(trimmed.to_string()));
        }

        if command_name == "as" {
            let bridged = format!("\\{}", trimmed);
            return Self::parse_execute_as_command(&bridged);
        }

        let cli = MetaCli::try_parse_from(tokens).map_err(Self::clap_parse_error)?;
        cli.command.into_command()
    }

    /// Parse meta-commands (backslash commands)
    fn parse_meta_command(&self, line: &str) -> Result<Command> {
        let Some(meta_line) = line.strip_prefix('\\') else {
            return Err(CLIError::ParseError("Invalid command".into()));
        };

        let tokens = Self::tokenize_command_line(meta_line)?;
        let Some(command_name) = tokens.first().map(String::as_str) else {
            return Err(CLIError::ParseError("Invalid command".into()));
        };

        if !is_known_meta_command(command_name) {
            return Ok(Command::Unknown(format!("\\{command_name}")));
        }

        if command_name == "as" {
            return Self::parse_execute_as_command(line);
        }

        let cli = MetaCli::try_parse_from(tokens).map_err(Self::clap_parse_error)?;
        cli.command.into_command()
    }

    fn parse_execute_as_command(line: &str) -> Result<Command> {
        let remainder = line["\\as".len()..].trim_start();
        if remainder.is_empty() {
            return Err(CLIError::ParseError("\\as requires a target user and SQL query".into()));
        }

        let user_end = remainder.find(char::is_whitespace).ok_or_else(|| {
            CLIError::ParseError("\\as requires a target user and SQL query".into())
        })?;

        let user = remainder[..user_end].trim();
        let sql = remainder[user_end..].trim_start();

        if user.is_empty() || sql.is_empty() {
            return Err(CLIError::ParseError("\\as requires a target user and SQL query".into()));
        }

        Ok(Command::ExecuteAs {
            user: user.to_string(),
            sql: sql.to_string(),
        })
    }

    fn tokenize_command_line(line: &str) -> Result<Vec<String>> {
        split_shell_words(line)
            .ok_or_else(|| CLIError::ParseError("Invalid quoting in command".into()))
    }

    fn clap_parse_error(error: clap::Error) -> CLIError {
        CLIError::ParseError(error.to_string())
    }
}

impl Default for CommandParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sql() {
        let parser = CommandParser::new();
        let cmd = parser.parse("SELECT * FROM users").unwrap();
        assert_eq!(cmd, Command::Sql("SELECT * FROM users".to_string()));
    }

    #[test]
    fn test_parse_quit() {
        let parser = CommandParser::new();
        assert_eq!(parser.parse("\\quit").unwrap(), Command::Quit);
        assert_eq!(parser.parse("\\q").unwrap(), Command::Quit);
    }

    #[test]
    fn test_parse_help() {
        let parser = CommandParser::new();
        assert_eq!(parser.parse("\\help").unwrap(), Command::Help);
        assert_eq!(parser.parse("\\?").unwrap(), Command::Help);
    }

    #[test]
    fn test_parse_flush_shortcuts() {
        let parser = CommandParser::new();

        assert_eq!(parser.parse("\\flush").unwrap(), Command::Flush(FlushTarget::All));
        assert_eq!(parser.parse("\\flush all").unwrap(), Command::Flush(FlushTarget::All));
        assert_eq!(
            parser.parse("\\flush table messages").unwrap(),
            Command::Flush(FlushTarget::Table("messages".to_string()))
        );
    }

    #[test]
    fn test_parse_flush_table_requires_target() {
        let parser = CommandParser::new();
        assert!(parser.parse("\\flush table").is_err());
    }

    #[test]
    fn test_parse_describe() {
        let parser = CommandParser::new();
        let cmd = parser.parse("\\describe users").unwrap();
        assert_eq!(cmd, Command::Describe("users".to_string()));
    }

    #[test]
    fn test_parse_meta_command_preserves_quoted_values_for_clap() {
        let parser = CommandParser::new();

        assert_eq!(
            parser.parse("\\describe \"chat users\"").unwrap(),
            Command::Describe("chat users".to_string())
        );
        assert_eq!(
            parser.parse("\\update-credentials admin \"secret phrase\"").unwrap(),
            Command::UpdateCredentials {
                username: "admin".to_string(),
                password: "secret phrase".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_live_alias() {
        let parser = CommandParser::new();
        let cmd = parser.parse("\\live SELECT * FROM chat.messages").unwrap();
        assert_eq!(cmd, Command::Subscribe("SELECT * FROM chat.messages".to_string()));
    }

    #[test]
    fn test_parse_execute_as_shortcut() {
        let parser = CommandParser::new();
        let cmd = parser.parse("\\as alice SELECT * FROM chat.messages WHERE body = 'hi  there'");
        assert_eq!(
            cmd.unwrap(),
            Command::ExecuteAs {
                user: "alice".to_string(),
                sql: "SELECT * FROM chat.messages WHERE body = 'hi  there'".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_unknown() {
        let parser = CommandParser::new();
        let cmd = parser.parse("\\unknown").unwrap();
        assert_eq!(cmd, Command::Unknown("\\unknown".to_string()));
    }

    #[test]
    fn test_parse_stats() {
        let parser = CommandParser::new();
        assert_eq!(parser.parse("\\stats").unwrap(), Command::Stats);
        assert_eq!(parser.parse("\\metrics").unwrap(), Command::Stats);
    }

    #[test]
    fn test_parse_sessions() {
        let parser = CommandParser::new();
        assert_eq!(parser.parse("\\sessions").unwrap(), Command::Sessions);
    }

    #[test]
    fn test_parse_history() {
        let parser = CommandParser::new();
        assert_eq!(parser.parse("\\history").unwrap(), Command::History);
        assert_eq!(parser.parse("\\h").unwrap(), Command::History);
    }

    #[test]
    fn test_empty_command() {
        let parser = CommandParser::new();
        assert!(parser.parse("").is_err());
        assert!(parser.parse("   ").is_err());
    }

    #[test]
    fn test_parse_cluster_join() {
        let parser = CommandParser::new();
        let cmd = parser.parse("\\cluster join 2 10.0.0.2:2910 http://10.0.0.2:2900").unwrap();
        assert_eq!(
            cmd,
            Command::ClusterJoin {
                node_id: 2,
                rpc_addr: "10.0.0.2:2910".to_string(),
                api_addr: "http://10.0.0.2:2900".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_cluster_rebalance() {
        let parser = CommandParser::new();
        assert_eq!(parser.parse("\\cluster rebalance").unwrap(), Command::ClusterRebalance);
    }

    #[test]
    fn test_parse_meta_command_aliases() {
        let parser = CommandParser::new();

        assert_eq!(parser.parse("\\health").unwrap(), Command::Health);
        assert_eq!(parser.parse("\\dt").unwrap(), Command::ListTables);
        assert_eq!(parser.parse("\\tables").unwrap(), Command::ListTables);
        assert_eq!(parser.parse("\\info").unwrap(), Command::Info);
        assert_eq!(parser.parse("\\session").unwrap(), Command::Info);
        assert_eq!(parser.parse("\\show-credentials").unwrap(), Command::ShowCredentials);
        assert_eq!(parser.parse("\\credentials").unwrap(), Command::ShowCredentials);
        assert_eq!(parser.parse("\\delete-credentials").unwrap(), Command::DeleteCredentials);
        assert_eq!(parser.parse("\\refresh-tables").unwrap(), Command::RefreshTables);
        assert_eq!(parser.parse("\\refresh").unwrap(), Command::RefreshTables);
    }

    #[test]
    fn test_parse_format_subscribe_and_credential_update_commands() {
        let parser = CommandParser::new();

        assert_eq!(parser.parse("\\format json").unwrap(), Command::SetFormat("json".to_string()));
        assert_eq!(
            parser.parse("\\subscribe SELECT * FROM app.messages").unwrap(),
            Command::Subscribe("SELECT * FROM app.messages".to_string())
        );
        assert_eq!(
            parser.parse("\\watch SELECT * FROM app.messages").unwrap(),
            Command::Subscribe("SELECT * FROM app.messages".to_string())
        );
        assert_eq!(
            parser.parse("\\update-credentials admin kalamdb123").unwrap(),
            Command::UpdateCredentials {
                username: "admin".to_string(),
                password: "kalamdb123".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_consume_command_with_all_options() {
        let parser = CommandParser::new();

        assert_eq!(
            parser
                .parse(
                    "\\consume app.events --group workers --from earliest --limit 5 --timeout 12"
                )
                .unwrap(),
            Command::Consume {
                topic: "app.events".to_string(),
                group: Some("workers".to_string()),
                from: Some("earliest".to_string()),
                limit: Some(5),
                timeout: Some(12),
            }
        );
    }

    #[test]
    fn test_parse_consume_command_accepts_clap_option_ordering() {
        let parser = CommandParser::new();

        assert_eq!(
            parser
                .parse("\\consume app.events --timeout 12 --limit 5 --from latest --group workers",)
                .unwrap(),
            Command::Consume {
                topic: "app.events".to_string(),
                group: Some("workers".to_string()),
                from: Some("latest".to_string()),
                limit: Some(5),
                timeout: Some(12),
            }
        );
    }

    #[test]
    fn test_parse_consume_command_rejects_invalid_arguments() {
        let parser = CommandParser::new();

        assert!(parser.parse("\\consume").is_err());
        assert!(parser.parse("\\consume app.events --limit nope").is_err());
        assert!(parser.parse("\\consume app.events --timeout nope").is_err());
        assert!(parser.parse("\\consume app.events --bogus 1").is_err());
    }

    #[test]
    fn test_parse_cluster_command_aliases() {
        let parser = CommandParser::new();

        assert_eq!(parser.parse("\\cluster snapshot").unwrap(), Command::ClusterSnapshot);
        assert_eq!(
            parser.parse("\\cluster purge --upto 42").unwrap(),
            Command::ClusterPurge { upto: 42 }
        );
        assert_eq!(parser.parse("\\cluster purge 42").unwrap(), Command::ClusterPurge { upto: 42 });
        assert_eq!(
            parser.parse("\\cluster trigger-election").unwrap(),
            Command::ClusterTriggerElection
        );
        assert_eq!(
            parser.parse("\\cluster trigger election").unwrap(),
            Command::ClusterTriggerElection
        );
        assert_eq!(
            parser.parse("\\cluster transfer-leader 7").unwrap(),
            Command::ClusterTransferLeader { node_id: 7 }
        );
        assert_eq!(
            parser.parse("\\cluster transfer leader 7").unwrap(),
            Command::ClusterTransferLeader { node_id: 7 }
        );
        assert_eq!(parser.parse("\\cluster stepdown").unwrap(), Command::ClusterStepdown);
        assert_eq!(parser.parse("\\cluster step-down").unwrap(), Command::ClusterStepdown);
        assert_eq!(parser.parse("\\cluster clear").unwrap(), Command::ClusterClear);
        assert_eq!(parser.parse("\\cluster list").unwrap(), Command::ClusterList);
        assert_eq!(parser.parse("\\cluster ls").unwrap(), Command::ClusterList);
        assert_eq!(parser.parse("\\cluster list groups").unwrap(), Command::ClusterListGroups);
    }

    #[test]
    fn test_parse_cluster_leave_is_rejected() {
        let parser = CommandParser::new();
        assert!(parser.parse("\\cluster leave").is_err());
    }

    #[test]
    fn meta_command_completions_are_registered_with_clap_parser() {
        for completion in meta_command_completions() {
            let name = completion.trim_start_matches('\\');
            assert!(
                is_known_meta_command(name),
                "{completion} must be registered as a clap meta command"
            );
        }
    }

    #[test]
    fn meta_cli_command_definition_passes_clap_debug_asserts() {
        MetaCli::command().debug_assert();
    }

    #[test]
    fn render_meta_command_help_uses_shared_descriptions() {
        let help = render_meta_command_help(false);

        assert!(help.contains("Kalam CLI Help"));
        assert!(help.contains("\\cluster list"));
        assert!(help.contains("List cluster nodes"));
        assert!(help.contains("\\show-credentials, \\credentials"));
        assert!(help.contains("Show stored credentials"));
    }

    #[test]
    fn test_parse_format_uses_clap_value_validation() {
        let parser = CommandParser::new();

        assert_eq!(parser.parse("\\format csv").unwrap(), Command::SetFormat("csv".to_string()));
        assert!(parser.parse("\\format xml").is_err());
    }

    #[test]
    fn test_parse_external_command_uses_shared_clap_model() {
        let parser = CommandParser::new();

        assert_eq!(parser.parse_external_command("cluster list").unwrap(), Command::ClusterList);
        assert_eq!(
            parser
                .parse_external_command("consume app.events --group workers --limit 2")
                .unwrap(),
            Command::Consume {
                topic: "app.events".to_string(),
                group: Some("workers".to_string()),
                from: None,
                limit: Some(2),
                timeout: None,
            }
        );
        assert_eq!(
            parser.parse_external_command("SELECT 1").unwrap(),
            Command::Sql("SELECT 1".to_string())
        );
    }

    #[test]
    fn test_parse_external_command_preserves_quoted_values_for_clap() {
        let parser = CommandParser::new();

        assert_eq!(
            parser.parse_external_command("describe \"chat users\"").unwrap(),
            Command::Describe("chat users".to_string())
        );
        assert_eq!(
            parser
                .parse_external_command("update-credentials admin \"secret phrase\"")
                .unwrap(),
            Command::UpdateCredentials {
                username: "admin".to_string(),
                password: "secret phrase".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_meta_command_rejects_invalid_quoting() {
        let parser = CommandParser::new();

        assert!(parser.parse("\\describe \"unterminated").is_err());
        assert!(parser.parse_external_command("describe \"unterminated").is_err());
    }

    #[test]
    fn test_parse_external_execute_as_preserves_sql_tail() {
        let parser = CommandParser::new();

        assert_eq!(
            parser
                .parse_external_command(
                    "as alice SELECT * FROM chat.messages WHERE body = 'hi  there'"
                )
                .unwrap(),
            Command::ExecuteAs {
                user: "alice".to_string(),
                sql: "SELECT * FROM chat.messages WHERE body = 'hi  there'".to_string(),
            }
        );
    }
}
