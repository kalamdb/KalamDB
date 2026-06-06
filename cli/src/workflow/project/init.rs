//! `kalam init` project scaffolding.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use url::Url;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    terminal_ui::{self, SelectOption},
    workflow::{
        dev::server::{write_local_server_config, DEFAULT_DEV_SERVER_URL, LOCAL_SERVER_CONFIG},
        project::{
            config::{
                ConnectionEnv, DevSection, KalamProjectConfig, LoggingSection, MigrationsSection,
                ProjectSection, SchemaMode, SchemaSection, SchemaTarget, KALAM_TOML,
            },
            prompts::{
                interactive_available, print_workflow_banner, prompt_multi_select, prompt_select,
                prompt_text, prompt_text_with_default,
            },
        },
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerMode {
    Local,
    Remote,
}

#[derive(Debug, Clone)]
pub struct InitOptions {
    pub name: Option<String>,
    pub schema_mode: Option<SchemaMode>,
    pub languages: Option<Vec<String>>,
    pub server_mode: Option<ServerMode>,
    pub server_url: Option<String>,
    pub yes: bool,
    pub cwd: PathBuf,
}

pub fn run_init(options: InitOptions, output: &WorkflowOutput) -> Result<()> {
    if options.cwd.join(KALAM_TOML).exists() {
        return Err(CLIError::ConfigurationError(format!(
            "project already initialized at '{}'",
            options.cwd.display()
        )));
    }

    ensure_interactive_or_yes(&options)?;

    if !options.yes {
        print_workflow_banner(
            "KalamDB project setup",
            "Create a project configuration for development and deployment.",
            output.use_color,
        );
    }

    let name = resolve_project_name(&options, output.use_color)?;
    let schema_mode = resolve_schema_mode(&options, output.use_color)?;
    let languages = resolve_languages(&options, output.use_color)?;
    let server_mode = resolve_server_mode(&options, output.use_color)?;
    let server_url = resolve_server_url(&options, server_mode, output.use_color)?;

    let config = build_config(&name, schema_mode, &languages, server_mode, &server_url);
    config.validate()?;

    write_project_scaffold(&options.cwd, &config, schema_mode, server_mode, &server_url, output)?;
    output.status(format!("initialized KalamDB project '{name}'"));
    Ok(())
}

fn ensure_interactive_or_yes(options: &InitOptions) -> Result<()> {
    if options.yes || interactive_available() {
        return Ok(());
    }
    Err(CLIError::ConfigurationError(
        "interactive init requires a TTY; pass --yes or provide --name, --schema-mode, \
         --languages, --server-mode, and --server-url"
            .into(),
    ))
}

fn resolve_project_name(options: &InitOptions, color: bool) -> Result<String> {
    if let Some(name) = options.name.as_ref().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        return Ok(name.to_string());
    }
    if options.yes {
        let fallback =
            options.cwd.file_name().and_then(|n| n.to_str()).unwrap_or("my-app").to_string();
        return Ok(fallback);
    }

    prompt_text("Project name:", color).and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            Err(CLIError::ConfigurationError("project name must not be empty".into()))
        } else {
            Ok(trimmed.to_string())
        }
    })
}

fn resolve_schema_mode(options: &InitOptions, color: bool) -> Result<SchemaMode> {
    if let Some(mode) = options.schema_mode {
        return Ok(mode);
    }
    if options.yes {
        return Ok(SchemaMode::Sql);
    }

    let options_list = [
        SelectOption::described("SQL file", "Use schema.sql as the source of truth"),
        SelectOption::described("Remote schema", "Read schema from a linked KalamDB server"),
    ];
    let selected = prompt_select("Schema mode", &options_list, 0, color)?;
    let mode = match selected {
        0 => SchemaMode::Sql,
        _ => SchemaMode::Remote,
    };
    eprintln!(
        "{} {}",
        terminal_ui::prompt_label("Schema mode:", color),
        terminal_ui::style_value(schema_mode_label(mode), color)
    );
    Ok(mode)
}

fn resolve_languages(options: &InitOptions, color: bool) -> Result<Vec<String>> {
    if let Some(languages) = &options.languages {
        return Ok(normalize_languages(languages)?);
    }
    if options.yes {
        return Ok(vec!["typescript".into(), "dart".into()]);
    }

    let options_list = [
        SelectOption::described("TypeScript", "Generate src/generated/kalam.ts"),
        SelectOption::described("Dart", "Generate lib/generated/kalam.dart"),
    ];
    let selected =
        prompt_multi_select("Generated language targets", &options_list, &[true, true], color)?;

    let mut languages = Vec::new();
    for idx in selected {
        match idx {
            0 => languages.push("typescript".into()),
            1 => languages.push("dart".into()),
            _ => {},
        }
    }
    let languages = normalize_languages(&languages)?;
    eprintln!(
        "{} {}",
        terminal_ui::prompt_label("Languages:", color),
        terminal_ui::style_value(&languages.join(", "), color)
    );
    Ok(languages)
}

fn resolve_server_mode(options: &InitOptions, color: bool) -> Result<ServerMode> {
    if let Some(mode) = options.server_mode {
        return Ok(mode);
    }
    if options.yes {
        return Ok(ServerMode::Local);
    }

    let options_list = [
        SelectOption::described("Local", "Start or reuse KalamDB with kalam dev"),
        SelectOption::described("Remote", "Connect to an existing KalamDB server"),
    ];
    let selected = prompt_select("Server mode", &options_list, 0, color)?;
    let mode = match selected {
        0 => ServerMode::Local,
        _ => ServerMode::Remote,
    };
    eprintln!(
        "{} {}",
        terminal_ui::prompt_label("Server mode:", color),
        terminal_ui::style_value(server_mode_label(mode), color)
    );
    Ok(mode)
}

fn resolve_server_url(
    options: &InitOptions,
    server_mode: ServerMode,
    color: bool,
) -> Result<String> {
    if let Some(url) = options.server_url.as_ref().map(|u| u.trim()).filter(|u| !u.is_empty()) {
        return validate_server_url(url);
    }

    match server_mode {
        ServerMode::Local => Ok(DEFAULT_DEV_SERVER_URL.to_string()),
        ServerMode::Remote if options.yes => Ok(DEFAULT_DEV_SERVER_URL.to_string()),
        ServerMode::Remote => {
            let url = prompt_text_with_default("Server URL", DEFAULT_DEV_SERVER_URL, color)?;
            validate_server_url(&url)
        },
    }
}

fn validate_server_url(value: &str) -> Result<String> {
    Url::parse(value)
        .map_err(|e| CLIError::ConfigurationError(format!("invalid server url '{value}': {e}")))?;
    Ok(value.to_string())
}

fn normalize_languages(languages: &[String]) -> Result<Vec<String>> {
    if languages.is_empty() {
        return Err(CLIError::ConfigurationError(
            "at least one language target is required".into(),
        ));
    }

    let mut normalized = Vec::new();
    for language in languages {
        match language.trim().to_ascii_lowercase().as_str() {
            "typescript" | "ts" => normalized.push("typescript".into()),
            "dart" => normalized.push("dart".into()),
            other => {
                return Err(CLIError::ConfigurationError(format!(
                    "unsupported language '{other}'; supported: typescript, dart"
                )));
            },
        }
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn schema_mode_label(mode: SchemaMode) -> &'static str {
    match mode {
        SchemaMode::Sql => "sql",
        SchemaMode::Remote => "remote",
    }
}

fn server_mode_label(mode: ServerMode) -> &'static str {
    match mode {
        ServerMode::Local => "local",
        ServerMode::Remote => "remote",
    }
}

fn build_config(
    name: &str,
    schema_mode: SchemaMode,
    languages: &[String],
    server_mode: ServerMode,
    server_url: &str,
) -> KalamProjectConfig {
    let mut targets = HashMap::new();
    for language in languages {
        let output = match language.as_str() {
            "typescript" => "src/generated/kalam.ts",
            "dart" => "lib/generated/kalam.dart",
            _ => continue,
        };
        targets.insert(
            language.clone(),
            SchemaTarget {
                output: output.into(),
            },
        );
    }

    KalamProjectConfig {
        project: ProjectSection {
            name: name.to_string(),
            default_env: "dev".into(),
        },
        connection: HashMap::from([(
            "dev".into(),
            ConnectionEnv {
                url: server_url.to_string(),
                namespace: name.to_string(),
            },
        )]),
        schema: SchemaSection {
            mode: schema_mode,
            path: Some("schema.sql".into()),
            watch: true,
            languages: languages.to_vec(),
            targets,
        },
        migrations: MigrationsSection {
            dir: "kalam/migrations".into(),
            auto_create: true,
        },
        dev: DevSection {
            auto_start_db: matches!(server_mode, ServerMode::Local),
            ..DevSection::default()
        },
        logging: LoggingSection::default(),
    }
}

fn write_project_scaffold(
    root: &Path,
    config: &KalamProjectConfig,
    schema_mode: SchemaMode,
    server_mode: ServerMode,
    server_url: &str,
    output: &WorkflowOutput,
) -> Result<()> {
    fs::create_dir_all(root)?;

    config.save_to_path(&root.join(KALAM_TOML))?;
    output.detail(format!("created {}", KALAM_TOML));

    if matches!(schema_mode, SchemaMode::Sql) {
        let schema_path = root.join("schema.sql");
        if !schema_path.exists() {
            fs::write(&schema_path, default_schema_sql(&config.project.name))?;
            output.detail("created schema.sql");
        }
    }

    if matches!(server_mode, ServerMode::Local) {
        let port = crate::workflow::dev::server::parse_server_port(server_url)?;
        write_local_server_config(root, port)?;
        output.detail(format!("created {LOCAL_SERVER_CONFIG}"));
    }

    let migrations_dir = config.migrations_dir(root);
    fs::create_dir_all(&migrations_dir)?;
    let gitkeep = migrations_dir.join(".gitkeep");
    if !gitkeep.exists() {
        fs::write(gitkeep, "")?;
    }
    output.detail(format!("created {}/", config.migrations.dir));

    for language in &config.schema.languages {
        if let Some(target) = config.schema.targets.get(language) {
            let out_path = root.join(&target.output);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            output.detail(format!("created output directory for {language}"));
        }
    }

    let env_example = root.join(".env.example");
    if !env_example.exists() {
        fs::write(
            &env_example,
            format!(
                "# KalamDB workflow environment overrides\nKALAM_ENV=dev\nKALAM_URL={server_url}\nKALAM_NAMESPACE={}\n",
                config.project.name
            ),
        )?;
        output.detail("created .env.example");
    }

    Ok(())
}

fn default_schema_sql(project_name: &str) -> String {
    format!(
        r#"-- Example schema for {project_name}
CREATE TABLE users (
  id INTEGER PRIMARY KEY,
  email TEXT NOT NULL,
  created_at TIMESTAMP
);
"#
    )
}

pub fn detect_package_manager(root: &Path) -> Option<&'static str> {
    if root.join("pnpm-lock.yaml").exists() {
        Some("pnpm")
    } else if root.join("yarn.lock").exists() {
        Some("yarn")
    } else if root.join("package-lock.json").exists() {
        Some("npm")
    } else if root.join("pubspec.yaml").exists() {
        Some("dart pub")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkflowLoggingPolicy;
    use tempfile::TempDir;

    #[test]
    fn init_scaffolds_project_files() {
        let temp = TempDir::new().unwrap();
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        run_init(
            InitOptions {
                name: Some("demo-app".into()),
                schema_mode: Some(SchemaMode::Sql),
                languages: Some(vec!["typescript".into()]),
                server_mode: Some(ServerMode::Local),
                server_url: None,
                yes: true,
                cwd: temp.path().to_path_buf(),
            },
            &output,
        )
        .unwrap();

        assert!(temp.path().join(KALAM_TOML).is_file());
        assert!(temp.path().join("schema.sql").is_file());
        assert!(temp.path().join("kalam/migrations/.gitkeep").is_file());
        assert!(temp.path().join("src/generated").is_dir());
        assert!(temp.path().join(LOCAL_SERVER_CONFIG).is_file());
    }

    #[test]
    fn remote_server_mode_disables_auto_start_db() {
        let temp = TempDir::new().unwrap();
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        run_init(
            InitOptions {
                name: Some("remote-app".into()),
                schema_mode: Some(SchemaMode::Sql),
                languages: Some(vec!["typescript".into()]),
                server_mode: Some(ServerMode::Remote),
                server_url: Some("http://localhost:2900".into()),
                yes: true,
                cwd: temp.path().to_path_buf(),
            },
            &output,
        )
        .unwrap();

        let config = KalamProjectConfig::load_from_path(&temp.path().join(KALAM_TOML)).unwrap();
        assert!(!config.dev.auto_start_db);
        assert!(!temp.path().join(LOCAL_SERVER_CONFIG).exists());
    }
}
