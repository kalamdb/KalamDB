use std::{future::Future, pin::Pin, time::Instant};

use colored::Colorize;
use kalam_client::SubscriptionConfig;

use super::{CLISession, OutputFormat};
use crate::{
    error::{CLIError, Result},
    parser::{render_meta_command_help, Command, FlushTarget},
};

type SessionCommandFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + 'a>>;

trait SessionCommandHandler {
    fn run<'a>(self, session: &'a mut CLISession) -> SessionCommandFuture<'a>;
}

impl SessionCommandHandler for Command {
    fn run<'a>(self, session: &'a mut CLISession) -> SessionCommandFuture<'a> {
        Box::pin(async move { session.execute_command_inner(self).await })
    }
}

impl CLISession {
    /// Execute a parsed command
    ///
    /// **Implements T094**: Backslash command handling
    pub(super) async fn execute_command(&mut self, command: Command) -> Result<()> {
        command.run(self).await
    }

    async fn execute_command_inner(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Sql(sql) => {
                self.execute(&sql).await?;
            },
            Command::Quit => {
                println!("Goodbye!");
                std::process::exit(0);
            },
            Command::Help => {
                self.show_help();
            },
            Command::Flush(target) => {
                let flush_sql = self.build_flush_query(&target)?;

                match &target {
                    FlushTarget::All => println!(
                        "Storage flushing all tables in namespace '{}'...",
                        self.effective_namespace()
                    ),
                    FlushTarget::Table(table) => println!(
                        "Storage flushing table '{}' in namespace context '{}'...",
                        table,
                        self.effective_namespace()
                    ),
                }

                match self.execute(&flush_sql).await {
                    Ok(_) => println!("Storage flush completed successfully"),
                    Err(e) => eprintln!("Storage flush failed: {}", e),
                }
            },
            Command::ClusterSnapshot => {
                self.show_cluster_group_action("CLUSTER SNAPSHOT").await?;
            },
            Command::ClusterPurge { upto } => {
                self.show_cluster_group_action(&format!("CLUSTER PURGE --UPTO {}", upto))
                    .await?;
            },
            Command::ClusterTriggerElection => {
                self.show_cluster_group_action("CLUSTER TRIGGER ELECTION").await?;
            },
            Command::ClusterTransferLeader { node_id } => {
                self.show_cluster_group_action(&format!("CLUSTER TRANSFER-LEADER {}", node_id))
                    .await?;
            },
            Command::ClusterRebalance => {
                self.show_cluster_group_action("CLUSTER REBALANCE").await?;
            },
            Command::ClusterStepdown => {
                self.show_cluster_group_action("CLUSTER STEPDOWN").await?;
            },
            Command::ClusterClear => {
                self.show_cluster_clear().await?;
            },
            Command::ClusterList => {
                self.show_cluster_list().await?;
            },
            Command::ClusterListGroups => {
                self.show_cluster_list_groups().await?;
            },
            Command::ClusterJoin {
                node_id,
                rpc_addr,
                api_addr,
            } => {
                self.show_cluster_join(node_id, &rpc_addr, &api_addr).await?;
            },
            Command::Health => match self.health_check().await {
                Ok(_) => {},
                Err(e) => eprintln!("Health check failed: {}", e),
            },
            Command::ListTables => {
                self.execute(
                    "SELECT namespace_id AS namespace, table_name, table_type FROM system.tables \
                     ORDER BY namespace_id, table_name",
                )
                .await?;
            },
            Command::Describe(table) => {
                let query = Self::build_describe_query(&table)?;
                self.execute(&query).await?;
            },
            Command::ExecuteAs { user, sql } => {
                let query = Self::build_execute_as_query(&user, &sql)?;
                self.execute(&query).await?;
            },
            Command::SetFormat(format) => match format.to_lowercase().as_str() {
                "table" => {
                    self.set_format(OutputFormat::Table);
                    println!("Output format set to: table");
                },
                "json" => {
                    self.set_format(OutputFormat::Json);
                    println!("Output format set to: json");
                },
                "csv" => {
                    self.set_format(OutputFormat::Csv);
                    println!("Output format set to: csv");
                },
                _ => {
                    eprintln!("Unknown format: {}. Use: table, json, or csv", format);
                },
            },
            Command::Subscribe(query) => {
                let sub_id = format!(
                    "sub_{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                );
                let config = self.build_subscription_config(&query, sub_id)?;
                self.run_subscription(config).await?;
            },
            Command::RefreshTables => {
                println!("Table names refreshed");
            },
            Command::ShowCredentials => {
                self.show_credentials();
            },
            Command::UpdateCredentials { username, password } => {
                self.update_credentials(username, password).await?;
            },
            Command::DeleteCredentials => {
                self.delete_credentials()?;
            },
            Command::Info => {
                self.show_session_info().await;
            },
            Command::Sessions => {
                self.execute(
                    "SELECT * FROM system.sessions ORDER BY last_seen_at DESC, session_id",
                )
                .await?;
            },
            Command::Stats => {
                self.execute(
                    "SELECT metric_name, metric_value FROM system.stats ORDER BY metric_name \
                     LIMIT 5000",
                )
                .await?;
            },
            Command::History => {
                eprintln!("History command should be handled in interactive mode");
            },
            Command::Consume {
                topic,
                group,
                from,
                limit,
                timeout,
            } => {
                self.cmd_consume(&topic, group.as_deref(), from.as_deref(), limit, timeout)
                    .await?;
            },
            Command::Unknown(cmd) => {
                eprintln!("Unknown command: {}. Type \\help for help.", cmd);
            },
            Command::ExportTable {
                table,
                user_id,
                output,
            } => {
                let user_id = user_id
                    .as_deref()
                    .map(crate::workflow::project::identifiers::parse_user_id)
                    .transpose()?;
                self.cmd_export_table(&table, user_id.as_ref(), output.as_deref()).await?;
            },
            Command::ImportTable {
                table,
                file,
                user_id,
            } => {
                let user_id = user_id
                    .as_deref()
                    .map(crate::workflow::project::identifiers::parse_user_id)
                    .transpose()?;
                self.cmd_import_table(&table, &file, user_id.as_ref()).await?;
            },
        }
        Ok(())
    }

    fn build_describe_query(target: &str) -> Result<String> {
        let (namespace, table_name) = Self::parse_describe_target(target)?;
        let columns = "table_schema AS namespace, table_name, column_name, data_type, \
                       is_nullable, column_default, ordinal_position AS position";
        let escaped_table = Self::escape_sql_literal(&table_name);

        if let Some(namespace) = namespace {
            Ok(format!(
                "SELECT {columns} FROM information_schema.columns WHERE table_schema = '{namespace}' \
                 AND table_name = '{table_name}' ORDER BY ordinal_position",
                namespace = Self::escape_sql_literal(&namespace),
                table_name = escaped_table,
            ))
        } else {
            Ok(format!(
                "SELECT {columns} FROM information_schema.columns WHERE table_name = '{table_name}' \
                 ORDER BY table_schema, ordinal_position",
                table_name = escaped_table,
            ))
        }
    }

    fn build_execute_as_query(user: &str, sql: &str) -> Result<String> {
        let normalized_user = Self::normalize_execute_as_user(user)?;
        let inner_sql = sql.trim().trim_end_matches(';').trim_end();

        if inner_sql.is_empty() {
            return Err(CLIError::ParseError(
                "\\as requires a non-empty SQL statement".to_string(),
            ));
        }

        Ok(format!(
            "EXECUTE AS '{}' ({})",
            Self::escape_sql_literal(&normalized_user),
            inner_sql,
        ))
    }

    fn build_flush_query(&self, target: &FlushTarget) -> Result<String> {
        match target {
            FlushTarget::All => Ok("STORAGE FLUSH ALL".to_string()),
            FlushTarget::Table(table) => {
                Self::build_flush_table_query(table, Some(self.effective_namespace()))
            },
        }
    }

    pub(super) fn build_subscription_config(
        &self,
        query: &str,
        subscription_id: String,
    ) -> Result<SubscriptionConfig> {
        let (clean_sql, options) = Self::extract_subscribe_options(query);
        let qualified_sql = Self::qualify_subscription_sql(&clean_sql, self.effective_namespace())?;
        let mut config = SubscriptionConfig::new(subscription_id, qualified_sql);
        config.options = options;
        Ok(config)
    }

    fn qualify_subscription_sql(sql: &str, default_namespace: &str) -> Result<String> {
        let trimmed = sql.trim().trim_end_matches(';').trim();
        if trimmed.is_empty() {
            return Err(CLIError::ParseError("\\live requires a SELECT query".to_string()));
        }

        let Some(from_idx) = Self::find_subscription_from_clause(trimmed) else {
            return Ok(trimmed.to_string());
        };

        let relation_start = trimmed[from_idx..]
            .char_indices()
            .find_map(|(offset, ch)| (!ch.is_whitespace()).then_some(from_idx + offset));
        let Some(relation_start) = relation_start else {
            return Ok(trimmed.to_string());
        };

        let relation_end = Self::find_subscription_relation_end(trimmed, relation_start);
        let relation = trimmed[relation_start..relation_end].trim();

        let parts = Self::split_identifier_parts(relation).map_err(|_| {
            CLIError::ParseError(
                "\\live expects SELECT ... FROM <table> or <namespace.table>".to_string(),
            )
        })?;

        if parts.len() != 1 {
            return Ok(trimmed.to_string());
        }

        let qualified_relation =
            format!("{}.{}", Self::quote_identifier(default_namespace), relation);

        Ok(format!(
            "{}{}{}",
            &trimmed[..relation_start],
            qualified_relation,
            &trimmed[relation_end..]
        ))
    }

    fn find_subscription_from_clause(sql: &str) -> Option<usize> {
        let mut in_single_quotes = false;
        let mut in_double_quotes = false;

        for (index, ch) in sql.char_indices() {
            match ch {
                '\'' if !in_double_quotes => in_single_quotes = !in_single_quotes,
                '"' if !in_single_quotes => in_double_quotes = !in_double_quotes,
                _ => {},
            }

            if in_single_quotes || in_double_quotes {
                continue;
            }

            let end = index + 4;
            if end > sql.len() || !sql[index..end].eq_ignore_ascii_case("from") {
                continue;
            }

            let prev = sql[..index].chars().last();
            let next = sql[end..].chars().next();
            let prev_is_ident = prev.is_some_and(Self::is_subscription_identifier_char);
            let next_is_ident = next.is_some_and(Self::is_subscription_identifier_char);

            if !prev_is_ident && !next_is_ident {
                return Some(end);
            }
        }

        None
    }

    fn find_subscription_relation_end(sql: &str, start: usize) -> usize {
        let mut in_double_quotes = false;

        for (offset, ch) in sql[start..].char_indices() {
            match ch {
                '"' => in_double_quotes = !in_double_quotes,
                _ if !in_double_quotes && ch.is_whitespace() => return start + offset,
                _ => {},
            }
        }

        sql.len()
    }

    fn is_subscription_identifier_char(ch: char) -> bool {
        ch.is_ascii_alphanumeric() || ch == '_'
    }

    fn build_flush_table_query(target: &str, default_namespace: Option<&str>) -> Result<String> {
        let trimmed = target.trim().trim_end_matches(';').trim();
        if trimmed.is_empty() {
            return Err(CLIError::ParseError("\\flush table requires a table name".to_string()));
        }

        let parts = Self::split_identifier_parts(trimmed)?;
        match parts.as_slice() {
            [table_name] => {
                let namespace = default_namespace.unwrap_or("default");
                Ok(format!(
                    "STORAGE FLUSH TABLE {}.{}",
                    Self::quote_identifier(namespace),
                    Self::quote_identifier(table_name),
                ))
            },
            [namespace, table_name] => Ok(format!(
                "STORAGE FLUSH TABLE {}.{}",
                Self::quote_identifier(namespace),
                Self::quote_identifier(table_name),
            )),
            _ => Err(CLIError::ParseError(
                "\\flush table expects <table> or <namespace.table>".to_string(),
            )),
        }
    }

    fn normalize_execute_as_user(user: &str) -> Result<String> {
        let trimmed = user.trim();
        if trimmed.is_empty() {
            return Err(CLIError::ParseError("\\as requires a target user".to_string()));
        }

        let normalized = if trimmed.len() >= 2
            && ((trimmed.starts_with('\'') && trimmed.ends_with('\''))
                || (trimmed.starts_with('"') && trimmed.ends_with('"')))
        {
            &trimmed[1..trimmed.len() - 1]
        } else {
            trimmed
        };

        if normalized.is_empty() {
            return Err(CLIError::ParseError("\\as requires a target user".to_string()));
        }

        Ok(normalized.to_string())
    }

    pub(super) fn parse_describe_target(target: &str) -> Result<(Option<String>, String)> {
        let trimmed = target.trim().trim_end_matches(';').trim();
        if trimmed.is_empty() {
            return Err(CLIError::ParseError("\\describe requires a table name".to_string()));
        }

        let parts = Self::split_identifier_parts(trimmed)?;
        match parts.as_slice() {
            [table_name] => Ok((None, table_name.clone())),
            [namespace, table_name] => Ok((Some(namespace.clone()), table_name.clone())),
            _ => Err(CLIError::ParseError(
                "\\describe expects <table> or <namespace.table>".to_string(),
            )),
        }
    }

    fn split_identifier_parts(target: &str) -> Result<Vec<String>> {
        let mut parts = Vec::new();
        let mut current = String::new();
        let mut chars = target.chars().peekable();
        let mut in_quotes = false;

        while let Some(ch) = chars.next() {
            match ch {
                '"' => {
                    if in_quotes && chars.peek() == Some(&'"') {
                        current.push('"');
                        chars.next();
                    } else {
                        in_quotes = !in_quotes;
                    }
                },
                '.' if !in_quotes => {
                    let part = current.trim();
                    if part.is_empty() {
                        return Err(CLIError::ParseError(
                            "\\describe received an invalid identifier".to_string(),
                        ));
                    }
                    parts.push(part.to_string());
                    current.clear();
                },
                _ => current.push(ch),
            }
        }

        if in_quotes {
            return Err(CLIError::ParseError(
                "\\describe received an unterminated quoted identifier".to_string(),
            ));
        }

        let tail = current.trim();
        if tail.is_empty() {
            return Err(CLIError::ParseError(
                "\\describe received an invalid identifier".to_string(),
            ));
        }
        parts.push(tail.to_string());

        Ok(parts)
    }

    fn escape_sql_literal(value: &str) -> String {
        value.replace('\'', "''")
    }

    fn quote_identifier(value: &str) -> String {
        format!("\"{}\"", value.replace('"', "\"\""))
    }

    fn show_help(&self) {
        print!("{}", render_meta_command_help(self.color));
    }

    /// Consume messages from a topic
    pub async fn cmd_consume(
        &mut self,
        topic: &str,
        group: Option<&str>,
        from: Option<&str>,
        limit: Option<usize>,
        timeout: Option<u64>,
    ) -> Result<()> {
        use kalam_client::consumer::AutoOffsetReset;
        use tokio::{
            signal,
            time::{sleep, Duration},
        };

        // Warn if no consumer group specified
        if group.is_none() {
            println!(
                "{}",
                "⚠️  Running without consumer group - offsets will not be saved".yellow()
            );
            println!("{}", "   Use --group NAME to persist progress".dimmed());
            println!();
        }

        // Build consumer
        let mut builder = self.client.consumer().topic(topic);

        if let Some(group_id) = group {
            builder = builder.group_id(group_id);
        }

        // Parse from offset
        if let Some(from_str) = from {
            let auto_offset = match from_str.to_lowercase().as_str() {
                "earliest" => AutoOffsetReset::Earliest,
                "latest" => AutoOffsetReset::Latest,
                offset_str => {
                    if let Ok(offset) = offset_str.parse::<u64>() {
                        AutoOffsetReset::Offset(offset)
                    } else {
                        return Err(CLIError::ParseError(format!(
                            "Invalid --from value: {}. Use 'earliest', 'latest', or a numeric \
                             offset",
                            from_str
                        )));
                    }
                },
            };
            builder = builder.auto_offset_reset(auto_offset);
        }

        let mut consumer = builder.build().map_err(|e| CLIError::LinkError(e))?;

        // Print header
        println!("{}", format!("Consuming from topic: {}", topic).bright_green().bold());
        if let Some(group_id) = group {
            println!("{}", format!("  Consumer group: {}", group_id).dimmed());
        }
        if let Some(from_str) = from {
            println!("{}", format!("  Starting from: {}", from_str).dimmed());
        }
        if let Some(limit_val) = limit {
            println!("{}", format!("  Limit: {} messages", limit_val).dimmed());
        }
        if let Some(timeout_val) = timeout {
            println!("{}", format!("  Timeout: {}s", timeout_val).dimmed());
        }
        println!("{}", "Press Ctrl+C to stop...".dimmed());
        println!();

        // CSV header for CSV format
        if matches!(self.format, OutputFormat::Csv) {
            println!("offset,operation,payload");
        }

        let start_time = Instant::now();
        let mut total_consumed = 0_usize;
        let mut last_offset = 0_u64;
        let mut error_count = 0;

        // Set up Ctrl+C handler
        let ctrl_c = signal::ctrl_c();
        tokio::pin!(ctrl_c);

        // Poll loop - library handles long polling (30s default)
        loop {
            // Check timeout
            if let Some(timeout_seconds) = timeout {
                if start_time.elapsed().as_secs() >= timeout_seconds {
                    println!();
                    println!("{}", "⏱️  Timeout reached".yellow());
                    break;
                }
            }

            // Check limit
            if let Some(limit_val) = limit {
                if total_consumed >= limit_val {
                    break;
                }
            }

            // Poll with long polling (30s) - library handles the HTTP request timeout
            // Use tokio::select to handle Ctrl+C during the poll
            let poll_result = tokio::select! {
                _ = &mut ctrl_c => {
                    println!();
                    println!("{}", "⚠️  Interrupted by user (Ctrl+C)".yellow());
                    break;
                }
                result = consumer.poll() => result
            };

            let records = match poll_result {
                Ok(records) => {
                    error_count = 0; // Reset error count on success
                    records
                },
                Err(e) => {
                    error_count += 1;

                    // Format detailed error message
                    let error_msg = format!("{}", e);
                    let detailed_error = if error_msg.contains("404") {
                        format!(
                            "❌ Topic '{}' not found or consume endpoint not available.\n   {}",
                            topic,
                            "Create the topic with: CREATE TOPIC <name> SOURCE TABLE \
                             <namespace>.<table>"
                                .dimmed()
                        )
                    } else if error_msg.contains("401") || error_msg.contains("403") {
                        format!(
                            "❌ Authentication failed or insufficient permissions.\n   Error: {}",
                            error_msg
                        )
                    } else if error_msg.contains("500") {
                        format!(
                            "❌ Server error while consuming from topic '{}'.\n   Error: {}",
                            topic, error_msg
                        )
                    } else {
                        format!("❌ Poll error: {}", error_msg)
                    };

                    eprintln!("{}", detailed_error.red());

                    // Exit after 3 consecutive errors instead of infinite retry
                    if error_count >= 3 {
                        eprintln!();
                        eprintln!("{}", "❌ Too many consecutive errors. Exiting.".red().bold());
                        break;
                    }

                    sleep(Duration::from_secs(1)).await;
                    continue;
                },
            };

            if records.is_empty() {
                // No messages, continue polling
                continue;
            }

            // Process and display records
            for record in records {
                last_offset = record.offset;

                // Format operation as string
                let op_str = match record.op {
                    kalam_client::consumer::TopicOp::Insert => "INSERT",
                    kalam_client::consumer::TopicOp::Update => "UPDATE",
                    kalam_client::consumer::TopicOp::Delete => "DELETE",
                };

                // Format and display record
                let formatted =
                    self.formatter.format_consumer_record(record.offset, op_str, &record.payload);
                println!("{}", formatted);

                // Mark as processed
                consumer.mark_processed(&record);

                total_consumed += 1;

                // Check limit after each record
                if let Some(limit_val) = limit {
                    if total_consumed >= limit_val {
                        break;
                    }
                }
            }
        }

        // Commit offsets before exit
        println!();
        if group.is_some() {
            print!("Committing offsets... ");
            match consumer.commit_sync().await {
                Ok(result) => {
                    println!(
                        "{}",
                        format!("✓ Committed offset: {}", result.acknowledged_offset).green()
                    );
                },
                Err(e) => {
                    println!("{}", format!("✗ Failed: {}", e).red());
                },
            }
        }

        // Print summary
        println!();
        println!(
            "{}",
            format!(
                "Consumed {} message(s). Last offset: {}. Duration: {:.2}s",
                total_consumed,
                last_offset,
                start_time.elapsed().as_secs_f64()
            )
            .bright_cyan()
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_execute_as_query_wraps_statement() {
        let query =
            CLISession::build_execute_as_query("alice", "SELECT * FROM user.orders;  ").unwrap();
        assert_eq!(query, "EXECUTE AS 'alice' (SELECT * FROM user.orders)");
    }

    #[test]
    fn test_build_execute_as_query_escapes_user_literal() {
        let query = CLISession::build_execute_as_query("o'brien", "SELECT 1").unwrap();
        assert_eq!(query, "EXECUTE AS 'o''brien' (SELECT 1)");
    }

    #[test]
    fn test_build_flush_table_query_uses_current_namespace_for_unqualified_table() {
        let query = CLISession::build_flush_table_query("messages", Some("chat")).unwrap();
        assert_eq!(query, "STORAGE FLUSH TABLE \"chat\".\"messages\"");
    }

    #[test]
    fn test_build_flush_table_query_preserves_explicit_namespace() {
        let query = CLISession::build_flush_table_query("billing.invoices", Some("chat")).unwrap();
        assert_eq!(query, "STORAGE FLUSH TABLE \"billing\".\"invoices\"");
    }

    #[test]
    fn test_qualify_subscription_sql_uses_current_namespace() {
        let query =
            CLISession::qualify_subscription_sql("SELECT * FROM messages WHERE id = 1", "chat")
                .unwrap();

        assert_eq!(query, "SELECT * FROM \"chat\".messages WHERE id = 1");
    }

    #[test]
    fn test_qualify_subscription_sql_preserves_explicit_namespace() {
        let query = CLISession::qualify_subscription_sql("SELECT ID FROM billing.messages", "chat")
            .unwrap();

        assert_eq!(query, "SELECT ID FROM billing.messages");
    }
}
