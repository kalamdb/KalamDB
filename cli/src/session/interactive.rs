use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    time::Duration,
};

use colored::Colorize;
use rustyline::{
    completion::Completer, error::ReadlineError, highlight::Highlighter, hint::Hinter,
    history::DefaultHistory, validate::Validator, Cmd, CompletionType, Config, EditMode, Editor,
    Helper, KeyEvent,
};

use super::CLISession;
use crate::{
    completer::{AutoCompleter, SQL_KEYWORDS, SQL_TYPES},
    error::Result,
    history::CommandHistory,
    history_menu::{HistoryMenu, HistoryMenuResult},
    parser::Command,
    CLI_VERSION,
};

const SYSTEM_TABLES: &[&str] = &[
    "users",
    "jobs",
    "namespaces",
    "storages",
    "live_queries",
    "tables",
    "audit_logs",
    "manifest",
    "stats",
    "settings",
    "server_logs",
    "cluster",
];

impl CLISession {
    fn primary_prompt(&self) -> String {
        #[cfg(target_os = "windows")]
        let use_colors_in_prompt = false;
        #[cfg(not(target_os = "windows"))]
        let use_colors_in_prompt = self.color;

        #[cfg(target_os = "windows")]
        let use_unicode = false;
        #[cfg(not(target_os = "windows"))]
        let use_unicode = true;

        let status = if use_colors_in_prompt {
            if self.connected {
                if use_unicode {
                    "●".green().bold().to_string()
                } else {
                    "*".green().bold().to_string()
                }
            } else if use_unicode {
                "○".yellow().bold().to_string()
            } else {
                "o".yellow().bold().to_string()
            }
        } else if self.connected {
            "*".to_string()
        } else {
            "o".to_string()
        };

        let brand = if use_colors_in_prompt {
            "KalamDB".bright_blue().bold().to_string()
        } else {
            "KalamDB".to_string()
        };

        let display_name =
            self.cluster_name.as_deref().or(self.instance.as_deref()).unwrap_or("local");

        let brand_with_profile = if use_colors_in_prompt {
            format!("{}{}", brand, format!("[{}]", display_name).dimmed())
        } else {
            format!("{}[{}]", brand, display_name)
        };

        let identity = if use_colors_in_prompt {
            format!("{}{}", self.username.cyan(), format!("@{}", self.server_host).dimmed())
        } else {
            format!("{}@{}", self.username, self.server_host)
        };

        let namespace = if use_colors_in_prompt {
            self.current_namespace_label().magenta().to_string()
        } else {
            self.current_namespace_label()
        };

        let update = update_prompt_label(self.update_available.as_ref(), use_colors_in_prompt);

        let arrow = if use_colors_in_prompt {
            if use_unicode {
                "❯".bright_blue().bold().to_string()
            } else {
                ">".bright_blue().bold().to_string()
            }
        } else {
            ">".to_string()
        };

        let mut parts = vec![status, brand_with_profile, identity, namespace];
        if let Some(update) = update {
            parts.push(update);
        }
        let body = parts.join(" ");
        format!("{} {} ", body, arrow)
    }

    fn continuation_prompt(&self) -> String {
        #[cfg(target_os = "windows")]
        let use_colors_in_prompt = false;
        #[cfg(not(target_os = "windows"))]
        let use_colors_in_prompt = self.color;

        #[cfg(target_os = "windows")]
        let use_unicode = false;
        #[cfg(not(target_os = "windows"))]
        let use_unicode = true;

        if use_colors_in_prompt {
            if use_unicode {
                format!("  {} {} ", "↳".dimmed(), "❯".bright_blue().bold())
            } else {
                format!("  {} {} ", "->".dimmed(), ">".bright_blue().bold())
            }
        } else {
            "  -> ".to_string()
        }
    }

    /// Run interactive readline loop with autocomplete.
    pub async fn run_interactive(&mut self) -> Result<()> {
        let mut completer = AutoCompleter::new();
        println!("{}", "Connecting and authenticating...".dimmed());

        if let Err(e) = self.refresh_tables(&mut completer).await {
            eprintln!();
            eprintln!("{} {}", "Connection failed:".red().bold(), e);
            eprintln!();
            eprintln!("{}", "Possible issues:".yellow().bold());
            eprintln!("  • Server is not running on {}", self.server_url);
            eprintln!("  • Authentication failed (check credentials)");
            eprintln!("  • Network connectivity issue");
            eprintln!();
            eprintln!("{}", "Try:".cyan().bold());
            eprintln!(
                "  • Check if server is running: curl {}/v1/api/healthcheck",
                self.server_url
            );
            eprintln!("  • Verify credentials with: kalam --user <user> --password <pass>");
            eprintln!("  • Use \\show-credentials to see stored credentials");
            eprintln!();
            std::process::exit(1);
        }

        println!("{}", "✓ Connected".green());

        if let Some(cluster_info) = self.fetch_cluster_info().await {
            self.adopt_cluster_metadata(&cluster_info);
        }

        self.update_available = crate::update_check::check_for_update(Duration::from_secs(2))
            .await
            .ok()
            .flatten();
        if let Some(update) = &self.update_available {
            let message = format!(
                "Update available: kalam {} -> {}. Run `kalam update`.",
                update.current_version, update.latest_version
            );
            if self.color {
                println!("{}", message.yellow().bold());
            } else {
                println!("{}", message);
            }
        }

        self.print_banner();

        let helper = CLIHelper::new(completer, self.color);
        let history_size = self.config.resolved_ui().history_size;
        let config = Config::builder()
            .max_history_size(history_size)?
            .completion_type(CompletionType::List)
            .completion_prompt_limit(100)
            .edit_mode(EditMode::Emacs)
            .auto_add_history(false)
            .build();

        let mut rl = Editor::<CLIHelper, DefaultHistory>::with_config(config)?;
        rl.set_helper(Some(helper));
        rl.bind_sequence(KeyEvent::from('\t'), Cmd::Complete);
        rl.bind_sequence(
            KeyEvent(rustyline::KeyCode::Up, rustyline::Modifiers::NONE),
            Cmd::AcceptLine,
        );

        let history = CommandHistory::new(history_size);

        if let Ok(history_entries) = history.load() {
            for entry in history_entries {
                let _ = rl.add_history_entry(&entry);
            }
        }

        let mut accumulated_command = String::new();
        let mut prefill_next = String::new();

        loop {
            let prompt = if accumulated_command.is_empty() {
                self.primary_prompt()
            } else {
                self.continuation_prompt()
            };

            let readline_result = if !prefill_next.is_empty() {
                let prefill = prefill_next.clone();
                prefill_next.clear();
                rl.readline_with_initial(&prompt, (&prefill, ""))
            } else {
                rl.readline(&prompt)
            };

            match readline_result {
                Ok(line) => {
                    let line = line.trim();

                    if line.is_empty() && accumulated_command.is_empty() && prefill_next.is_empty()
                    {
                        let history_entries = history.load().unwrap_or_default();

                        if !history_entries.is_empty() {
                            let mut menu = HistoryMenu::new(history_entries, self.color);
                            match menu.run("") {
                                Ok(HistoryMenuResult::Selected(selected_cmd)) => {
                                    let _ = history.deduplicate_and_move_to_end(&selected_cmd);
                                    prefill_next = selected_cmd;
                                },
                                Ok(HistoryMenuResult::Cancelled)
                                | Ok(HistoryMenuResult::Continue) => {},
                                Err(e) => {
                                    eprintln!("{}", format!("History menu error: {}", e).red());
                                },
                            }
                        }
                        continue;
                    }

                    if !accumulated_command.is_empty() {
                        accumulated_command.push('\n');
                    }
                    accumulated_command.push_str(line);

                    let is_complete = line.ends_with(';')
                        || accumulated_command.trim_start().starts_with('\\')
                        || (line.is_empty() && !accumulated_command.is_empty());

                    if !is_complete {
                        continue;
                    }

                    let final_command = accumulated_command.trim().to_string();
                    accumulated_command.clear();

                    if final_command.is_empty() {
                        continue;
                    }

                    if crate::history::should_persist_command(&final_command) {
                        let _ = rl.add_history_entry(&final_command);
                        let _ = history.append(&final_command);
                    }

                    match self.parser.parse(&final_command) {
                        Ok(command) => {
                            if matches!(command, Command::RefreshTables) {
                                if let Some(helper) = rl.helper_mut() {
                                    print!("{}", "Fetching tables... ".dimmed());
                                    std::io::Write::flush(&mut std::io::stdout()).ok();

                                    if let Err(e) = self.refresh_tables(&mut helper.completer).await
                                    {
                                        println!("{}", format!("✗ {}", e).red());
                                    } else {
                                        println!("{}", "✓".green());
                                    }
                                }
                                continue;
                            }

                            if matches!(command, Command::History) {
                                let history_entries = history.load().unwrap_or_default();

                                if history_entries.is_empty() {
                                    println!("{}", "No command history available".dimmed());
                                    continue;
                                }

                                let mut menu = HistoryMenu::new(history_entries, self.color);
                                match menu.run("") {
                                    Ok(HistoryMenuResult::Selected(selected_cmd)) => {
                                        let _ = history.deduplicate_and_move_to_end(&selected_cmd);
                                        accumulated_command = selected_cmd;
                                    },
                                    Ok(HistoryMenuResult::Cancelled)
                                    | Ok(HistoryMenuResult::Continue) => {},
                                    Err(e) => {
                                        eprintln!("{}", format!("History menu error: {}", e).red());
                                    },
                                }
                                continue;
                            }

                            if let Err(e) = self.execute_command(command).await {
                                eprintln!("{}", format!("✗ {}", e).red());
                            }
                        },
                        Err(e) => {
                            eprintln!("{}", format!("✗ {}", e).red());
                        },
                    }
                },
                Err(ReadlineError::Interrupted) => {
                    if !accumulated_command.is_empty() {
                        println!("\n{}", "Command cancelled".yellow());
                        accumulated_command.clear();
                    } else {
                        println!("{}", "Use \\quit or \\q to exit".dimmed());
                    }
                    continue;
                },
                Err(ReadlineError::Eof) => {
                    println!("\n{}", "Goodbye!".cyan());
                    break;
                },
                Err(err) => {
                    eprintln!("{}", format!("✗ {}", err).red());
                    break;
                },
            }
        }

        Ok(())
    }

    fn print_banner(&self) {
        println!();
        println!(
            "{}",
            "╔═══════════════════════════════════════════════════════════╗"
                .bright_blue()
                .bold()
        );
        println!(
            "{}",
            "║                                                           ║"
                .bright_blue()
                .bold()
        );
        println!(
            "{}{}{}",
            "║        ".bright_blue().bold(),
            "🗄️  Kalam CLI - Interactive Database Terminal".white().bold(),
            "       ║".bright_blue().bold()
        );
        println!(
            "{}",
            "║                                                           ║"
                .bright_blue()
                .bold()
        );
        println!(
            "{}",
            "╚═══════════════════════════════════════════════════════════╝"
                .bright_blue()
                .bold()
        );
        println!();
        println!("  {}  {}", "📡".dimmed(), format!("Connected to: {}", self.server_url).cyan());
        println!("  {}  {}", "👤".dimmed(), format!("User: {}", self.username).cyan());

        if let Some(ref version) = self.server_version {
            println!("  {}  {}", "🏷️ ".dimmed(), format!("Server version: {}", version).dimmed());
        }

        println!(
            "  {}  {}",
            "📚".dimmed(),
            format!("CLI version: {} (built: {})", CLI_VERSION, env!("BUILD_DATE")).dimmed()
        );

        println!(
            "  {}  Type {} for help, {} for session info, {} to exit",
            "💡".dimmed(),
            "\\help".cyan().bold(),
            "\\info".cyan().bold(),
            "\\quit".cyan().bold()
        );
        println!();
    }

    async fn refresh_tables(&mut self, completer: &mut AutoCompleter) -> Result<()> {
        let namespaces_res = if self.animations {
            let pb = Self::create_spinner("Fetching namespaces...");
            let resp = self
                .client
                .execute_query("SELECT name FROM system.namespaces ORDER BY name", None, None, None)
                .await;
            pb.finish_and_clear();
            resp
        } else {
            self.client
                .execute_query("SELECT name FROM system.namespaces ORDER BY name", None, None, None)
                .await
        };

        let mut namespaces: Vec<String> = Vec::new();
        if let Ok(ns_resp) = namespaces_res {
            if let Some(result) = ns_resp.results.first() {
                if let Some(rows) = &result.rows {
                    let name_idx = result.schema.iter().position(|f| f.name == "name");
                    for row in rows {
                        if let Some(idx) = name_idx {
                            if let Some(ns) = row.get(idx).and_then(|v| v.as_str()) {
                                namespaces.push(ns.to_string());
                            }
                        }
                    }
                }
            }
        }

        let response = if self.animations {
            let pb = Self::create_spinner("Fetching tables...");
            let resp = self
                .client
                .execute_query(
                    "SELECT table_name, namespace_id FROM system.tables",
                    None,
                    None,
                    None,
                )
                .await?;
            pb.finish_and_clear();
            resp
        } else {
            self.client
                .execute_query(
                    "SELECT table_name, namespace_id FROM system.tables",
                    None,
                    None,
                    None,
                )
                .await?
        };

        let mut table_names = Vec::new();
        let mut ns_map: HashMap<String, Vec<String>> = HashMap::new();
        if let Some(result) = response.results.first() {
            if let Some(rows) = &result.rows {
                let table_name_idx = result.schema.iter().position(|f| f.name == "table_name");
                let ns_idx = result.schema.iter().position(|f| f.name == "namespace_id");

                for row in rows {
                    let name_opt =
                        table_name_idx.and_then(|idx| row.get(idx)).and_then(|v| v.as_str());
                    let ns_opt = ns_idx.and_then(|idx| row.get(idx)).and_then(|v| v.as_str());
                    if let Some(name) = name_opt {
                        table_names.push(name.to_string());
                        if let Some(ns) = ns_opt {
                            ns_map.entry(ns.to_string()).or_default().push(name.to_string());
                        }
                    }
                }
            }
        }

        let sys_tables_res = self
            .client
            .execute_query(
                "SELECT table_schema, table_name FROM information_schema.tables WHERE \
                 table_schema IN ('system', 'information_schema') ORDER BY table_schema, \
                 table_name",
                None,
                None,
                None,
            )
            .await;

        if let Ok(sys_resp) = sys_tables_res {
            if let Some(result) = sys_resp.results.first() {
                if let Some(rows) = &result.rows {
                    let table_name_idx = result.schema.iter().position(|f| f.name == "table_name");
                    let schema_idx = result.schema.iter().position(|f| f.name == "table_schema");

                    for row in rows {
                        let name_opt =
                            table_name_idx.and_then(|idx| row.get(idx)).and_then(|v| v.as_str());
                        let schema_opt =
                            schema_idx.and_then(|idx| row.get(idx)).and_then(|v| v.as_str());
                        if let (Some(name), Some(schema)) = (name_opt, schema_opt) {
                            if !table_names.contains(&name.to_string()) {
                                table_names.push(name.to_string());
                            }
                            ns_map.entry(schema.to_string()).or_default().push(name.to_string());
                            if !namespaces.contains(&schema.to_string()) {
                                namespaces.push(schema.to_string());
                            }
                        }
                    }
                }
            }
        }

        let sys_ns = "system".to_string();
        if !namespaces.contains(&sys_ns) {
            namespaces.push(sys_ns.clone());
        }

        for tbl in SYSTEM_TABLES {
            if !table_names.contains(&tbl.to_string()) {
                table_names.push(tbl.to_string());
            }
            ns_map.entry(sys_ns.clone()).or_default().push(tbl.to_string());
        }

        namespaces.sort();
        namespaces.dedup();

        completer.set_namespaces(namespaces);
        completer.set_tables(table_names);
        for (ns, mut tables) in ns_map {
            tables.sort();
            tables.dedup();
            completer.set_namespace_tables(ns, tables);
        }
        completer.clear_columns();

        if let Ok(column_response) = if self.animations {
            let pb = Self::create_spinner("Fetching columns...");
            let resp = self
                .client
                .execute_query(
                    "SELECT table_name, column_name FROM information_schema.columns ORDER BY \
                     table_name, ordinal_position",
                    None,
                    None,
                    None,
                )
                .await;
            pb.finish_and_clear();
            resp
        } else {
            self.client
                .execute_query(
                    "SELECT table_name, column_name FROM information_schema.columns ORDER BY \
                     table_name, ordinal_position",
                    None,
                    None,
                    None,
                )
                .await
        } {
            if let Some(result) = column_response.results.first() {
                if let Some(rows) = &result.rows {
                    let mut column_map: HashMap<String, Vec<String>> = HashMap::new();
                    let table_name_idx = result.schema.iter().position(|f| f.name == "table_name");
                    let column_name_idx =
                        result.schema.iter().position(|f| f.name == "column_name");

                    for row in rows {
                        let table_opt =
                            table_name_idx.and_then(|idx| row.get(idx)).and_then(|v| v.as_str());
                        let column_opt =
                            column_name_idx.and_then(|idx| row.get(idx)).and_then(|v| v.as_str());
                        if let (Some(table), Some(column)) = (table_opt, column_opt) {
                            column_map
                                .entry(table.to_string())
                                .or_default()
                                .push(column.to_string());
                        }
                    }

                    for (table, columns) in column_map {
                        completer.set_columns(table, columns);
                    }
                }
            }
        }
        Ok(())
    }
}

fn update_prompt_label(
    available: Option<&crate::update_check::UpdateAvailability>,
    use_colors: bool,
) -> Option<String> {
    available.map(|available| {
        let label = format!("update:{}", available.latest_version);
        if use_colors {
            format!("{}", label.yellow().bold())
        } else {
            label
        }
    })
}

#[cfg(test)]
mod tests {
    use super::update_prompt_label;
    use crate::update_check::UpdateAvailability;

    #[test]
    fn update_prompt_label_is_absent_without_update() {
        assert_eq!(update_prompt_label(None, false), None);
    }

    #[test]
    fn update_prompt_label_includes_latest_version() {
        let available = UpdateAvailability {
            current_version: "0.5.1-beta.2".to_string(),
            latest_version: "0.5.1-beta.3".to_string(),
        };

        assert_eq!(
            update_prompt_label(Some(&available), false).as_deref(),
            Some("update:0.5.1-beta.3")
        );
    }
}

type CharIter<'a> = std::iter::Peekable<std::str::Chars<'a>>;

struct SqlHighlighter {
    keywords: HashSet<String>,
    types: HashSet<String>,
    color_enabled: bool,
}

impl SqlHighlighter {
    fn new(color_enabled: bool) -> Self {
        let keywords =
            SQL_KEYWORDS.iter().map(|kw| kw.to_ascii_uppercase()).collect::<HashSet<_>>();
        let types = SQL_TYPES.iter().map(|kw| kw.to_ascii_uppercase()).collect::<HashSet<_>>();

        Self {
            keywords,
            types,
            color_enabled,
        }
    }

    fn color_enabled(&self) -> bool {
        self.color_enabled
    }

    fn highlight(&self, line: &str) -> Option<String> {
        if !self.color_enabled || line.trim().is_empty() {
            return None;
        }

        Some(self.highlight_line(line))
    }

    fn highlight_line(&self, line: &str) -> String {
        let mut result = String::with_capacity(line.len() * 2);
        let mut iter = line.chars().peekable();

        while let Some(ch) = iter.next() {
            if ch.is_whitespace() {
                result.push(ch);
                continue;
            }

            if ch == '-' {
                if let Some('-') = iter.peek().copied() {
                    result.push_str(&self.collect_comment(&mut iter));
                    return result;
                }
                result.push(ch);
                continue;
            }

            if ch == '\'' || ch == '"' {
                result.push_str(&self.collect_string(ch, &mut iter));
                continue;
            }

            if ch.is_ascii_digit() {
                result.push_str(&self.collect_number(ch, &mut iter));
                continue;
            }

            if ch.is_alphabetic() || ch == '_' {
                result.push_str(&self.collect_identifier(ch, &mut iter));
                continue;
            }

            result.push(ch);
        }

        result
    }

    fn collect_comment(&self, iter: &mut CharIter<'_>) -> String {
        let mut comment = String::from("--");
        iter.next();
        for ch in iter {
            comment.push(ch);
        }
        self.style_comment(&comment)
    }

    fn collect_string(&self, quote: char, iter: &mut CharIter<'_>) -> String {
        let mut literal = String::new();
        literal.push(quote);

        if quote == '\'' {
            while let Some(next) = iter.next() {
                literal.push(next);
                if next == quote {
                    if let Some(&dup) = iter.peek() {
                        if dup == quote {
                            literal.push(dup);
                            iter.next();
                            continue;
                        }
                    }
                    break;
                }
            }
        } else {
            let mut escaped = false;
            for next in iter.by_ref() {
                literal.push(next);
                if escaped {
                    escaped = false;
                    continue;
                }
                if next == '\\' {
                    escaped = true;
                    continue;
                }
                if next == quote {
                    break;
                }
            }
        }

        self.style_string(&literal)
    }

    fn collect_number(&self, first: char, iter: &mut CharIter<'_>) -> String {
        let mut number = String::new();
        number.push(first);

        while let Some(&next) = iter.peek() {
            if next.is_ascii_digit() || next == '_' || next == '.' {
                number.push(next);
                iter.next();
                continue;
            }

            if matches!(next, 'e' | 'E') {
                number.push(next);
                iter.next();
                if let Some(&sign) = iter.peek() {
                    if sign == '+' || sign == '-' {
                        number.push(sign);
                        iter.next();
                    }
                }
                continue;
            }

            break;
        }

        self.style_number(&number)
    }

    fn collect_identifier(&self, first: char, iter: &mut CharIter<'_>) -> String {
        let mut ident = String::new();
        ident.push(first);

        while let Some(&next) = iter.peek() {
            if next.is_ascii_alphanumeric() || next == '_' {
                ident.push(next);
                iter.next();
            } else {
                break;
            }
        }

        let upper = ident.to_ascii_uppercase();
        if self.types.contains(&upper) {
            self.style_type(&ident)
        } else if self.keywords.contains(&upper) {
            self.style_keyword(&ident)
        } else {
            self.style_identifier(&ident)
        }
    }

    fn style_keyword(&self, token: &str) -> String {
        token.blue().bold().to_string()
    }

    fn style_type(&self, token: &str) -> String {
        token.magenta().bold().to_string()
    }

    fn style_identifier(&self, token: &str) -> String {
        token.to_string()
    }

    fn style_number(&self, token: &str) -> String {
        token.yellow().to_string()
    }

    fn style_string(&self, token: &str) -> String {
        token.green().to_string()
    }

    fn style_comment(&self, token: &str) -> String {
        token.dimmed().to_string()
    }
}

struct CLIHelper {
    completer: AutoCompleter,
    highlighter: SqlHighlighter,
}

impl CLIHelper {
    fn new(completer: AutoCompleter, color_enabled: bool) -> Self {
        Self {
            highlighter: SqlHighlighter::new(color_enabled),
            completer,
        }
    }
}

impl Completer for CLIHelper {
    type Candidate = <AutoCompleter as Completer>::Candidate;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        self.completer.complete(line, pos, ctx)
    }
}

impl Hinter for CLIHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, _ctx: &rustyline::Context<'_>) -> Option<Self::Hint> {
        self.completer.completion_hint(line, pos)
    }
}

impl Highlighter for CLIHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        if let Some(highlighted) = self.highlighter.highlight(line) {
            Cow::Owned(highlighted)
        } else {
            Cow::Borrowed(line)
        }
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        if self.highlighter.color_enabled() && !hint.is_empty() {
            Cow::Owned(hint.dimmed().to_string())
        } else {
            Cow::Borrowed(hint)
        }
    }
}

impl Validator for CLIHelper {}

impl Helper for CLIHelper {}
