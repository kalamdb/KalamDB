//! `kalam init` project scaffolding.

use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use url::Url;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    terminal_ui::{self, SelectOption},
    workflow::{
        dev::server::{write_local_server_config, DEFAULT_DEV_SERVER_URL},
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

const TEMPLATES_DIR: &str = "templates";
const PROJECT_SCHEMA_TEMPLATE: &str = "project/schema.sql.template";
const TYPESCRIPT_PACKAGE_TEMPLATE: &str = "typescript/package.json.template";
const TYPESCRIPT_TSCONFIG_TEMPLATE: &str = "typescript/tsconfig.json.template";
const TYPESCRIPT_INDEX_TEMPLATE: &str = "typescript/src/index.ts.template";
const SKIP_PACKAGE_INSTALL_ENV: &str = "KALAM_TEST_SKIP_PACKAGE_INSTALL";

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
    run_init_with_installer(options, output, execute_install_command)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstallCommand {
    program: &'static str,
    args: Vec<&'static str>,
    description: &'static str,
}

impl InstallCommand {
    fn npm_install() -> Self {
        Self {
            program: "npm",
            args: vec!["install"],
            description: "installing npm dependencies",
        }
    }
}

fn run_init_with_installer<F>(
    options: InitOptions,
    output: &WorkflowOutput,
    mut installer: F,
) -> Result<()>
where
    F: FnMut(&Path, &InstallCommand) -> Result<()>,
{
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
    maybe_install_scaffold_dependencies(
        &options.cwd,
        &config.schema.languages,
        output,
        &mut installer,
    )?;
    output.status(format!("initialized KalamDB project '{name}'"));
    Ok(())
}

fn maybe_install_scaffold_dependencies<F>(
    root: &Path,
    languages: &[String],
    output: &WorkflowOutput,
    installer: &mut F,
) -> Result<()>
where
    F: FnMut(&Path, &InstallCommand) -> Result<()>,
{
    if env::var_os(SKIP_PACKAGE_INSTALL_ENV).is_some() {
        output.detail("skipped package install step");
        return Ok(());
    }

    let Some(command) = scaffold_install_command(languages) else {
        return Ok(());
    };

    output.status(command.description);
    installer(root, &command)?;
    output.status("installed npm dependencies");
    Ok(())
}

fn scaffold_install_command(languages: &[String]) -> Option<InstallCommand> {
    languages
        .iter()
        .any(|language| language == "typescript")
        .then(InstallCommand::npm_install)
}

fn execute_install_command(root: &Path, command: &InstallCommand) -> Result<()> {
    let output = Command::new(command.program)
        .current_dir(root)
        .args(&command.args)
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                CLIError::ConfigurationError(format!(
                    "{} is required to finish project setup",
                    command.program
                ))
            } else {
                CLIError::FileError(format!(
                    "failed to run '{} {}': {error}",
                    command.program,
                    command.args.join(" ")
                ))
            }
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let exit_status = output
        .status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "terminated by signal".to_string());
    let detail = format!(
        "{} {} failed with exit status {exit_status}.\nstdout:\n{}\nstderr:\n{}",
        command.program,
        command.args.join(" "),
        stdout.trim(),
        stderr.trim()
    );
    Err(CLIError::ConfigurationError(detail))
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
        return Ok(vec!["typescript".into()]);
    }

    let options_list = [
        SelectOption::described("TypeScript", "Generate src/generated/kalam.ts"),
        SelectOption::described("Dart", "Generate lib/generated/kalam.dart"),
    ];
    let selected =
        prompt_multi_select("Generated language targets", &options_list, &[true, false], color)?;

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
            kalam_dir: "kalam".into(),
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

    write_default_gitignore(root, &config.schema.languages, output)?;
    let kalam_gitignore = config.ensure_kalam_gitignore(root)?;
    output.detail(format!("created {}", kalam_gitignore.display()));
    let cli_log_dir = config.ensure_cli_log_dir(root)?;
    output.detail(format!("created {}/", cli_log_dir.display()));

    if matches!(schema_mode, SchemaMode::Sql) {
        let schema_path = root.join("schema.sql");
        if !schema_path.exists() {
            fs::write(&schema_path, default_schema_sql(&config.project.name)?)?;
            output.detail("created schema.sql");
        }
    }

    if matches!(server_mode, ServerMode::Local) {
        let port = crate::workflow::dev::server::parse_server_port(server_url)?;
        let server_config_path = config.local_server_config_path(root);
        let created = !server_config_path.exists();
        write_local_server_config(root, config, port)?;
        if created {
            output.detail(format!("created {}", server_config_path.display()));
        }
    }

    let migrations_dir = config.migrations_dir(root);
    fs::create_dir_all(&migrations_dir)?;
    let gitkeep = migrations_dir.join(".gitkeep");
    if !gitkeep.exists() {
        fs::write(gitkeep, "")?;
    }
    output.detail(format!("created {}/", migrations_dir.display()));

    for language in &config.schema.languages {
        if let Some(target) = config.schema.targets.get(language) {
            let out_path = root.join(&target.output);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            output.detail(format!("created output directory for {language}"));
        }
    }

    if config.schema.languages.iter().any(|language| language == "typescript") {
        write_typescript_starter(root, &config.project.name, server_url, output)?;
    }

    let default_profile =
        crate::workflow::project::resolve::credential_instance_for_env(&config.project.default_env);
    let env_template = format!(
        "# KalamDB CLI profile used by kalam dev/deploy\n# kalam dev fills this automatically for managed local servers; use kalam login for remote servers.\nKALAM_PROFILE={default_profile}\n"
    );

    let env_file = root.join(".env");
    if !env_file.exists() {
        fs::write(&env_file, &env_template)?;
        output.detail("created .env");
    }

    let env_example = root.join(".env.example");
    if !env_example.exists() {
        fs::write(&env_example, env_template)?;
        output.detail("created .env.example");
    }

    Ok(())
}

fn default_schema_sql(project_name: &str) -> Result<String> {
    Ok(render_template(
        &read_template(PROJECT_SCHEMA_TEMPLATE)?,
        &[("project_name", project_name)],
    ))
}

fn write_default_gitignore(
    root: &Path,
    languages: &[String],
    output: &WorkflowOutput,
) -> Result<()> {
    let gitignore_path = root.join(".gitignore");
    if gitignore_path.exists() {
        return Ok(());
    }

    let mut lines = vec![
        "# KalamDB project state".to_string(),
        ".env".to_string(),
        "kalam/cli/logs/".to_string(),
        "kalam/server/".to_string(),
        "kalam/.schema-baseline.sql".to_string(),
    ];

    if languages.iter().any(|language| language == "typescript") {
        lines.push(String::new());
        lines.push("# TypeScript / Node".to_string());
        lines.push("node_modules/".to_string());
        lines.push("dist/".to_string());
    }

    if languages.iter().any(|language| language == "dart") {
        lines.push(String::new());
        lines.push("# Dart".to_string());
        lines.push(".dart_tool/".to_string());
        lines.push("build/".to_string());
    }

    fs::write(&gitignore_path, format!("{}\n", lines.join("\n")))?;
    output.detail("created .gitignore");
    Ok(())
}

fn write_typescript_starter(
    root: &Path,
    project_name: &str,
    server_url: &str,
    output: &WorkflowOutput,
) -> Result<()> {
    write_template_file(
        root,
        "package.json",
        TYPESCRIPT_PACKAGE_TEMPLATE,
        &[("project_name", project_name)],
        output,
    )?;
    write_template_file(root, "tsconfig.json", TYPESCRIPT_TSCONFIG_TEMPLATE, &[], output)?;
    write_template_file(
        root,
        "src/index.ts",
        TYPESCRIPT_INDEX_TEMPLATE,
        &[("server_url", server_url)],
        output,
    )?;

    Ok(())
}

pub fn detect_package_manager(root: &Path) -> Option<&'static str> {
    if root.join("bun.lock").exists() || root.join("bun.lockb").exists() {
        Some("bun")
    } else if root.join("pnpm-lock.yaml").exists() {
        Some("pnpm")
    } else if root.join("yarn.lock").exists() {
        Some("yarn")
    } else if root.join("package-lock.json").exists() || root.join("npm-shrinkwrap.json").exists() {
        Some("npm")
    } else if let Some(manager) = package_manager_from_package_json(root) {
        Some(manager)
    } else if root.join("pubspec.yaml").exists() {
        Some("dart pub")
    } else {
        None
    }
}

fn template_path(relative_path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(TEMPLATES_DIR).join(relative_path)
}

fn read_template(relative_path: &str) -> Result<String> {
    let path = template_path(relative_path);
    fs::read_to_string(&path).map_err(|error| {
        crate::error::CLIError::FileError(format!(
            "failed to read template '{}': {error}",
            path.display()
        ))
    })
}

fn render_template(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut rendered = template.to_string();
    for (key, value) in replacements {
        rendered = rendered.replace(&format!("{{{{{key}}}}}"), value);
    }
    rendered
}

fn write_template_file(
    root: &Path,
    destination: &str,
    template: &str,
    replacements: &[(&str, &str)],
    output: &WorkflowOutput,
) -> Result<()> {
    let destination_path = root.join(destination);
    if destination_path.exists() {
        return Ok(());
    }
    if let Some(parent) = destination_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let rendered = render_template(&read_template(template)?, replacements);
    fs::write(&destination_path, rendered)?;
    output.detail(format!("created {destination}"));
    Ok(())
}

fn package_manager_from_package_json(root: &Path) -> Option<&'static str> {
    let package_json_path = root.join("package.json");
    let contents = fs::read_to_string(package_json_path).ok()?;
    let package_json: serde_json::Value = serde_json::from_str(&contents).ok()?;
    let manager = package_json.get("packageManager")?.as_str()?.trim();

    if manager.starts_with("bun@") || manager == "bun" {
        Some("bun")
    } else if manager.starts_with("pnpm@") || manager == "pnpm" {
        Some("pnpm")
    } else if manager.starts_with("yarn@") || manager == "yarn" {
        Some("yarn")
    } else if manager.starts_with("npm@") || manager == "npm" {
        Some("npm")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkflowLoggingPolicy;
    use std::fs;
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
        assert!(temp.path().join("kalam/server/server.toml").is_file());
    }

    #[test]
    fn init_creates_project_env_file_and_ignores_it() {
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

        let env_contents = fs::read_to_string(temp.path().join(".env")).unwrap();
        assert!(env_contents.contains("KALAM_PROFILE=kalam-dev"));

        let gitignore = fs::read_to_string(temp.path().join(".gitignore")).unwrap();
        assert!(gitignore.lines().any(|line| line.trim() == ".env"));
    }

    #[test]
    fn init_typescript_project_requests_npm_install() {
        let temp = TempDir::new().unwrap();
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let mut observed: Option<(PathBuf, InstallCommand)> = None;

        run_init_with_installer(
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
            |root, command| {
                observed = Some((root.to_path_buf(), command.clone()));
                Ok(())
            },
        )
        .unwrap();

        let (root, command) = observed.expect("typescript init should request a package install");
        assert_eq!(root, temp.path());
        assert_eq!(command.program, "npm");
        assert_eq!(command.args, vec!["install"]);
    }

    #[test]
    fn init_dart_only_project_skips_npm_install() {
        let temp = TempDir::new().unwrap();
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let mut called = false;

        run_init_with_installer(
            InitOptions {
                name: Some("demo-dart".into()),
                schema_mode: Some(SchemaMode::Sql),
                languages: Some(vec!["dart".into()]),
                server_mode: Some(ServerMode::Local),
                server_url: None,
                yes: true,
                cwd: temp.path().to_path_buf(),
            },
            &output,
            |_root, _command| {
                called = true;
                Ok(())
            },
        )
        .unwrap();

        assert!(!called, "dart-only init should not request npm install");
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
        assert!(!temp.path().join("kalam/server/server.toml").exists());
    }

    #[test]
    fn detect_package_manager_supports_all_known_lockfiles() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        fs::write(root.join("bun.lock"), "").unwrap();
        assert_eq!(detect_package_manager(root), Some("bun"));
        fs::remove_file(root.join("bun.lock")).unwrap();

        fs::write(root.join("bun.lockb"), "").unwrap();
        assert_eq!(detect_package_manager(root), Some("bun"));
        fs::remove_file(root.join("bun.lockb")).unwrap();

        fs::write(root.join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(detect_package_manager(root), Some("pnpm"));
        fs::remove_file(root.join("pnpm-lock.yaml")).unwrap();

        fs::write(root.join("yarn.lock"), "").unwrap();
        assert_eq!(detect_package_manager(root), Some("yarn"));
        fs::remove_file(root.join("yarn.lock")).unwrap();

        fs::write(root.join("package-lock.json"), "{}").unwrap();
        assert_eq!(detect_package_manager(root), Some("npm"));
        fs::remove_file(root.join("package-lock.json")).unwrap();

        fs::write(root.join("npm-shrinkwrap.json"), "{}").unwrap();
        assert_eq!(detect_package_manager(root), Some("npm"));
        fs::remove_file(root.join("npm-shrinkwrap.json")).unwrap();

        fs::write(root.join("pubspec.yaml"), "name: demo\n").unwrap();
        assert_eq!(detect_package_manager(root), Some("dart pub"));
    }

    #[test]
    fn detect_package_manager_reads_package_manager_field() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        fs::write(root.join("package.json"), r#"{ "packageManager": "bun@1.2.0" }"#).unwrap();
        assert_eq!(detect_package_manager(root), Some("bun"));

        fs::write(root.join("package.json"), r#"{ "packageManager": "pnpm@10.0.0" }"#).unwrap();
        assert_eq!(detect_package_manager(root), Some("pnpm"));

        fs::write(root.join("package.json"), r#"{ "packageManager": "yarn@4.0.0" }"#).unwrap();
        assert_eq!(detect_package_manager(root), Some("yarn"));

        fs::write(root.join("package.json"), r#"{ "packageManager": "npm@10.0.0" }"#).unwrap();
        assert_eq!(detect_package_manager(root), Some("npm"));
    }

    #[test]
    fn project_templates_render_expected_values() {
        let schema = default_schema_sql("demo-app").unwrap();
        assert!(schema.contains("demo-app"));
        assert!(schema.contains("CREATE TABLE users"));

        let package_json = render_template(
            &read_template(TYPESCRIPT_PACKAGE_TEMPLATE).unwrap(),
            &[("project_name", "demo-app")],
        );
        assert!(package_json.contains(r#""name": "demo-app""#));

        let starter = render_template(
            &read_template(TYPESCRIPT_INDEX_TEMPLATE).unwrap(),
            &[("server_url", "http://localhost:2900")],
        );
        assert!(starter.contains("http://localhost:2900"));
        assert!(starter.contains("liveTable"));
        assert!(starter.contains("createClient"));
    }

    #[test]
    fn typescript_tsconfig_includes_node_types() {
        let tsconfig = read_template(TYPESCRIPT_TSCONFIG_TEMPLATE).unwrap();
        assert!(tsconfig.contains(r#""types": ["node"]"#));
    }
}
