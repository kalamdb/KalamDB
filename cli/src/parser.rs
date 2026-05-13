//! Command parser for SQL and backslash commands
//!
//! **Implements T087**: CommandParser for SQL + backslash commands
//!
//! Parses user input to distinguish between SQL statements and CLI meta-commands.

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
}

pub(crate) const META_COMMAND_COMPLETIONS: &[&str] = &[
    "\\quit",
    "\\q",
    "\\help",
    "\\?",
    "\\info",
    "\\session",
    "\\history",
    "\\h",
    "\\sessions",
    "\\stats",
    "\\metrics",
    "\\flush",
    "\\health",
    "\\dt",
    "\\tables",
    "\\d",
    "\\describe",
    "\\as",
    "\\format",
    "\\refresh-tables",
    "\\refresh",
    "\\subscribe",
    "\\live",
    "\\show-credentials",
    "\\credentials",
    "\\update-credentials",
    "\\delete-credentials",
    "\\cluster",
    "\\consume",
];

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

    /// Parse meta-commands (backslash commands)
    fn parse_meta_command(&self, line: &str) -> Result<Command> {
        if line.starts_with("\\as") {
            return Self::parse_execute_as_command(line);
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            return Err(CLIError::ParseError("Invalid command".into()));
        }

        let command = parts[0];
        let args = parts.get(1..).unwrap_or(&[]);

        match command {
            "\\quit" | "\\q" => Ok(Command::Quit),
            "\\help" | "\\?" => Ok(Command::Help),
            "\\sessions" => Ok(Command::Sessions),
            "\\stats" | "\\metrics" => Ok(Command::Stats),
            "\\flush" => Self::parse_flush_command(args),
            "\\cluster" => {
                if args.is_empty() {
                    Err(CLIError::ParseError(
                        "\\cluster requires: snapshot, purge, trigger-election, transfer-leader, \
                         rebalance, stepdown, clear, list, or join"
                            .into(),
                    ))
                } else {
                    let sub = args[0].to_ascii_lowercase();
                    match sub.as_str() {
                        "snapshot" => Ok(Command::ClusterSnapshot),
                        "purge" => {
                            let upto = args
                                .iter()
                                .skip(1)
                                .position(|arg| {
                                    arg.trim_start_matches('-').eq_ignore_ascii_case("upto")
                                })
                                .and_then(|pos| args.get(pos + 2))
                                .and_then(|v| v.parse::<u64>().ok())
                                .or_else(|| args.get(1).and_then(|v| v.parse::<u64>().ok()));

                            if let Some(upto) = upto {
                                Ok(Command::ClusterPurge { upto })
                            } else {
                                Err(CLIError::ParseError(
                                    "\\cluster purge requires --upto <index> or a numeric index"
                                        .into(),
                                ))
                            }
                        },
                        "trigger-election" => Ok(Command::ClusterTriggerElection),
                        "trigger" => {
                            if args.get(1).map(|v| v.eq_ignore_ascii_case("election")) == Some(true)
                            {
                                Ok(Command::ClusterTriggerElection)
                            } else {
                                Err(CLIError::ParseError(
                                    "\\cluster trigger requires: election".into(),
                                ))
                            }
                        },
                        "transfer-leader" => {
                            let node_id = args.get(1).and_then(|v| v.parse::<u64>().ok());
                            if let Some(node_id) = node_id {
                                Ok(Command::ClusterTransferLeader { node_id })
                            } else {
                                Err(CLIError::ParseError(
                                    "\\cluster transfer-leader requires a numeric node id".into(),
                                ))
                            }
                        },
                        "transfer" => {
                            if args.get(1).map(|v| v.eq_ignore_ascii_case("leader")) == Some(true) {
                                let node_id = args.get(2).and_then(|v| v.parse::<u64>().ok());
                                if let Some(node_id) = node_id {
                                    Ok(Command::ClusterTransferLeader { node_id })
                                } else {
                                    Err(CLIError::ParseError(
                                        "\\cluster transfer leader requires a numeric node id"
                                            .into(),
                                    ))
                                }
                            } else {
                                Err(CLIError::ParseError(
                                    "\\cluster transfer requires: leader <node_id>".into(),
                                ))
                            }
                        },
                        "rebalance" => Ok(Command::ClusterRebalance),
                        "stepdown" | "step-down" => Ok(Command::ClusterStepdown),
                        "clear" => Ok(Command::ClusterClear),
                        "list" | "ls" => {
                            if args.get(1).map(|v| v.eq_ignore_ascii_case("groups")) == Some(true) {
                                Ok(Command::ClusterListGroups)
                            } else {
                                Ok(Command::ClusterList)
                            }
                        },
                        "join" => {
                            if args.len() < 4 {
                                Err(CLIError::ParseError(
                                    "\\cluster join requires <node_id> <rpc_addr> <api_addr>"
                                        .into(),
                                ))
                            } else {
                                let node_id = args[1].parse::<u64>().map_err(|_| {
                                    CLIError::ParseError(
                                        "\\cluster join requires a numeric node id".into(),
                                    )
                                })?;
                                Ok(Command::ClusterJoin {
                                    node_id,
                                    rpc_addr: args[2].to_string(),
                                    api_addr: args[3].to_string(),
                                })
                            }
                        },
                        _ => Err(CLIError::ParseError(format!(
                            "Unknown cluster subcommand: {}",
                            args[0]
                        ))),
                    }
                }
            },
            "\\health" => Ok(Command::Health),
            "\\dt" | "\\tables" => Ok(Command::ListTables),
            "\\d" | "\\describe" => {
                if args.is_empty() {
                    Err(CLIError::ParseError("\\describe requires a table name".into()))
                } else {
                    Ok(Command::Describe(args.join(" ")))
                }
            },
            "\\format" => {
                if args.is_empty() {
                    Err(CLIError::ParseError("\\format requires: table, json, or csv".into()))
                } else {
                    Ok(Command::SetFormat(args[0].to_string()))
                }
            },
            "\\subscribe" | "\\watch" | "\\live" => {
                if args.is_empty() {
                    Err(CLIError::ParseError("\\subscribe requires a SQL query".into()))
                } else {
                    Ok(Command::Subscribe(args.join(" ")))
                }
            },
            "\\refresh-tables" | "\\refresh" => Ok(Command::RefreshTables),
            "\\show-credentials" | "\\credentials" => Ok(Command::ShowCredentials),
            "\\update-credentials" => {
                if args.len() < 2 {
                    Err(CLIError::ParseError(
                        "\\update-credentials requires user and password".into(),
                    ))
                } else {
                    Ok(Command::UpdateCredentials {
                        username: args[0].to_string(),
                        password: args[1].to_string(),
                    })
                }
            },
            "\\delete-credentials" => Ok(Command::DeleteCredentials),
            "\\info" | "\\session" => Ok(Command::Info),
            "\\history" | "\\h" => Ok(Command::History),
            "\\consume" => {
                if args.is_empty() {
                    return Err(CLIError::ParseError(
                        "\\consume requires a topic name. Usage: \\consume <topic> [--group NAME] \
                         [--from earliest|latest|OFFSET] [--limit N] [--timeout SECONDS]"
                            .into(),
                    ));
                }
                let topic = args[0].to_string();
                let mut group = None;
                let mut from = None;
                let mut limit = None;
                let mut timeout = None;

                let mut i = 1;
                while i < args.len() {
                    match args[i] {
                        "--group" => {
                            if i + 1 < args.len() {
                                group = Some(args[i + 1].to_string());
                                i += 2;
                            } else {
                                return Err(CLIError::ParseError(
                                    "--group requires a value".into(),
                                ));
                            }
                        },
                        "--from" => {
                            if i + 1 < args.len() {
                                from = Some(args[i + 1].to_string());
                                i += 2;
                            } else {
                                return Err(CLIError::ParseError("--from requires a value".into()));
                            }
                        },
                        "--limit" => {
                            if i + 1 < args.len() {
                                limit = args[i + 1].parse::<usize>().ok();
                                if limit.is_none() {
                                    return Err(CLIError::ParseError(
                                        "--limit requires a numeric value".into(),
                                    ));
                                }
                                i += 2;
                            } else {
                                return Err(CLIError::ParseError(
                                    "--limit requires a value".into(),
                                ));
                            }
                        },
                        "--timeout" => {
                            if i + 1 < args.len() {
                                timeout = args[i + 1].parse::<u64>().ok();
                                if timeout.is_none() {
                                    return Err(CLIError::ParseError(
                                        "--timeout requires a numeric value (seconds)".into(),
                                    ));
                                }
                                i += 2;
                            } else {
                                return Err(CLIError::ParseError(
                                    "--timeout requires a value".into(),
                                ));
                            }
                        },
                        _ => {
                            return Err(CLIError::ParseError(format!(
                                "Unknown option for \\consume: {}",
                                args[i]
                            )));
                        },
                    }
                }

                Ok(Command::Consume {
                    topic,
                    group,
                    from,
                    limit,
                    timeout,
                })
            },
            _ => Ok(Command::Unknown(command.to_string())),
        }
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

    fn parse_flush_command(args: &[&str]) -> Result<Command> {
        match args {
            [] => Ok(Command::Flush(FlushTarget::All)),
            [subcommand] if subcommand.eq_ignore_ascii_case("all") => {
                Ok(Command::Flush(FlushTarget::All))
            },
            [subcommand, table @ ..] if subcommand.eq_ignore_ascii_case("table") => {
                if table.is_empty() {
                    Err(CLIError::ParseError("\\flush table requires a table name".into()))
                } else {
                    Ok(Command::Flush(FlushTarget::Table(table.join(" "))))
                }
            },
            _ => Err(CLIError::ParseError(
                "\\flush supports: \\flush, \\flush all, or \\flush table <table>".into(),
            )),
        }
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
}
