//! `kalam init` project scaffolding.

use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::json;

use url::Url;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    terminal_ui::{self, SelectOption},
    workflow::{
        dev::server::{write_local_server_config, DEFAULT_DEV_SERVER_URL},
        display_project_path,
        project::{
            config::{
                ConnectionEnv, DevSection, KalamProjectConfig, LoggingSection, MigrationsSection,
                ProjectSection, SchemaMode, SchemaSection, SchemaTarget, KALAM_TOML,
            },
            identifiers::{normalize_namespace_name, parse_namespace_id},
            prompts::{
                interactive_available, print_workflow_banner, prompt_multi_select, prompt_select,
                prompt_text, prompt_text_with_default,
            },
            templates::{
                self, find_template_file, render_template, render_template_pairs, EmbeddedTemplate,
                DEFAULT_TYPESCRIPT_TEMPLATE, DEFAULT_SCAFFOLD_TEMPLATE,
            },
        },
    },
};

const SKIP_PACKAGE_INSTALL_ENV: &str = "KALAM_TEST_SKIP_PACKAGE_INSTALL";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemplateSource {
    Builtin,
    Repository,
}

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
    pub template: Option<String>,
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
    let typescript_template =
        resolve_typescript_template(&options, &languages, output.use_color)?;
    let server_mode = resolve_server_mode(&options, output.use_color)?;
    let server_url = resolve_server_url(&options, server_mode, output.use_color)?;

    let config = build_config(&name, schema_mode, &languages, server_mode, &server_url)?;
    config.validate()?;
    if let Some(connection) = config.connection.get("dev") {
        if connection.namespace.as_str() != name {
            output.detail(format!(
                "using namespace '{}' for project name '{name}'",
                connection.namespace.as_str()
            ));
        }
    }

    write_project_scaffold(
        &options.cwd,
        &config,
        schema_mode,
        server_mode,
        &server_url,
        typescript_template,
        output,
    )?;
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
        if matches!(mode, SchemaMode::Remote) {
            return Err(CLIError::ConfigurationError(
                "remote schema mode is not available yet; use --schema-mode sql".into(),
            ));
        }
        return Ok(mode);
    }
    if options.yes {
        return Ok(SchemaMode::Sql);
    }

    let options_list = [
        SelectOption::described("SQL file", "Use schema.sql as the source of truth"),
        SelectOption::disabled("Remote schema", "Coming soon"),
    ];
    let selected = prompt_select("Schema mode", &options_list, 0, color)?;
    let mode = SchemaMode::Sql;
    if selected != 0 {
        return Err(CLIError::ConfigurationError(
            "remote schema mode is not available yet".into(),
        ));
    }
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

fn resolve_typescript_template(
    options: &InitOptions,
    languages: &[String],
    color: bool,
) -> Result<Option<&'static EmbeddedTemplate>> {
    if !languages.iter().any(|language| language == "typescript") {
        return Ok(None);
    }

    if let Some(template_id) =
        options.template.as_ref().map(|value| value.trim()).filter(|value| !value.is_empty())
    {
        return templates::resolve_typescript_template(Some(template_id)).map(Some);
    }

    if options.yes {
        return templates::resolve_typescript_template(Some(DEFAULT_TYPESCRIPT_TEMPLATE)).map(Some);
    }

    let source_options = [
        SelectOption::described("Built-in templates", "Use templates bundled with the CLI"),
        SelectOption::disabled("From repository", "Load a template from a local path (coming soon)"),
    ];
    let source_selected = prompt_select("Template source", &source_options, 0, color)?;
    let source = match source_selected {
        0 => TemplateSource::Builtin,
        _ => TemplateSource::Repository,
    };

    if matches!(source, TemplateSource::Repository) {
        return Err(CLIError::ConfigurationError(
            "loading templates from a repository is not available yet".into(),
        ));
    }

    let available = templates::templates_for_language("typescript");
    if available.is_empty() {
        return Err(CLIError::ConfigurationError(
            "no built-in typescript templates are available".into(),
        ));
    }

    let template_options: Vec<SelectOption<'_>> = available
        .iter()
        .map(|template| SelectOption::described(template.id, template.description))
        .collect();
    let default_index = available
        .iter()
        .position(|template| template.id == DEFAULT_TYPESCRIPT_TEMPLATE)
        .unwrap_or(0);
    let selected = prompt_select("Project template", &template_options, default_index, color)?;
    let template = available[selected];
    eprintln!(
        "{} {}",
        terminal_ui::prompt_label("Template:", color),
        terminal_ui::style_value(template.id, color)
    );
    Ok(Some(template))
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
) -> Result<KalamProjectConfig> {
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

    Ok(KalamProjectConfig {
        project: ProjectSection {
            name: name.to_string(),
            default_env: "dev".into(),
            kalam_dir: "kalam".into(),
        },
        connection: HashMap::from([(
            "dev".into(),
            ConnectionEnv {
                url: server_url.to_string(),
                namespace: parse_namespace_id(&normalize_namespace_name(name))?,
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
    })
}

fn write_project_scaffold(
    root: &Path,
    config: &KalamProjectConfig,
    schema_mode: SchemaMode,
    server_mode: ServerMode,
    server_url: &str,
    typescript_template: Option<&EmbeddedTemplate>,
    output: &WorkflowOutput,
) -> Result<()> {
    fs::create_dir_all(root)?;

    write_kalam_toml_from_template(root, config, server_mode, server_url)?;
    output.detail(format!("created {}", KALAM_TOML));

    write_default_gitignore(root, &config.schema.languages, output)?;
    let kalam_gitignore = config.ensure_kalam_gitignore(root)?;
    output.detail(format!(
        "created {}",
        display_project_path(root, &kalam_gitignore)
    ));
    let cli_log_dir = config.ensure_cli_log_dir(root)?;
    output.detail(format!(
        "created {}/",
        display_project_path(root, &cli_log_dir)
    ));

    if let Some(template) = typescript_template {
        let replacements = template_replacements(&config.project.name, server_url);
        apply_embedded_template(root, template, &replacements, output)?;
    } else if matches!(schema_mode, SchemaMode::Sql) {
        let schema_path = root.join("schema.sql");
        if !schema_path.exists() {
            fs::write(
                &schema_path,
                "-- Add your schema here\n",
            )?;
            output.detail(format!(
                "created {}",
                display_project_path(root, &schema_path)
            ));
        }
    }

    if matches!(server_mode, ServerMode::Local) {
        let port = crate::workflow::dev::server::parse_server_port(server_url)?;
        let server_config_path = config.local_server_config_path(root);
        let created = !server_config_path.exists();
        write_local_server_config(root, config, port)?;
        if created {
            output.detail(format!(
                "created {}",
                display_project_path(root, &server_config_path)
            ));
        }
    }

    let migrations_dir = config.migrations_dir(root);
    fs::create_dir_all(&migrations_dir)?;
    let gitkeep = migrations_dir.join(".gitkeep");
    if !gitkeep.exists() {
        fs::write(gitkeep, "")?;
    }
    output.detail(format!(
        "created {}/",
        display_project_path(root, &migrations_dir)
    ));

    for language in &config.schema.languages {
        if let Some(target) = config.schema.targets.get(language) {
            let out_path = root.join(&target.output);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            output.detail(format!(
                "created {}",
                display_project_path(root, out_path.parent().unwrap_or(&out_path))
            ));
        }
    }

    let default_profile =
        crate::workflow::project::resolve::credential_instance_for_env(&config.project.default_env);
    let env_template = scaffold_template_file(".env.example")?;
    let env_contents = render_template(env_template, &json!({ "default_profile": default_profile }))?;

    let env_file = root.join(".env");
    if !env_file.exists() {
        fs::write(&env_file, &env_contents)?;
        output.detail("created .env");
    }

    let env_example = root.join(".env.example");
    if !env_example.exists() {
        fs::write(&env_example, env_contents)?;
        output.detail("created .env.example");
    }

    Ok(())
}

fn template_replacements<'a>(project_name: &'a str, server_url: &'a str) -> [(&'a str, &'a str); 2] {
    [("project_name", project_name), ("server_url", server_url)]
}

fn apply_embedded_template(
    root: &Path,
    template: &EmbeddedTemplate,
    replacements: &[(&str, &str)],
    output: &WorkflowOutput,
) -> Result<()> {
    for file in template.files {
        let destination_path = root.join(file.project_path);
        if destination_path.exists() {
            continue;
        }
        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let rendered = render_template_pairs(file.content, replacements)?;
        fs::write(&destination_path, rendered)?;
        output.detail(format!(
            "created {}",
            display_project_path(root, &destination_path)
        ));
    }
    Ok(())
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

    let template = scaffold_template_file(".gitignore")?;
    let contents = render_template(
        template,
        &json!({
            "typescript": languages.iter().any(|language| language == "typescript"),
            "dart": languages.iter().any(|language| language == "dart"),
        }),
    )?;
    fs::write(&gitignore_path, contents)?;
    output.detail("created .gitignore");
    Ok(())
}

fn scaffold_template_file(project_path: &str) -> Result<&'static str> {
    let template = templates::resolve_scaffold_template()?;
    find_template_file(template, project_path).ok_or_else(|| {
        CLIError::ConfigurationError(format!(
            "missing scaffold template file '{project_path}' in '{DEFAULT_SCAFFOLD_TEMPLATE}'"
        ))
    })
}

fn write_kalam_toml_from_template(
    root: &Path,
    config: &KalamProjectConfig,
    server_mode: ServerMode,
    server_url: &str,
) -> Result<()> {
    let template = scaffold_template_file("kalam.toml")?;
    let languages_array = format_languages_array(&config.schema.languages);
    let schema_mode = schema_mode_label(config.schema.mode);
    let schema_path = config.schema.path.as_deref().unwrap_or("schema.sql");
    let namespace = config
        .connection
        .get("dev")
        .map(|connection| connection.namespace.as_str())
        .unwrap_or("");
    let contents = render_template(
        template,
        &json!({
            "project_name": config.project.name,
            "namespace": namespace,
            "server_url": server_url,
            "schema_mode": schema_mode,
            "schema_path": schema_path,
            "languages_array": languages_array,
            "typescript": config.schema.languages.iter().any(|language| language == "typescript"),
            "dart": config.schema.languages.iter().any(|language| language == "dart"),
            "auto_start_db": matches!(server_mode, ServerMode::Local),
        }),
    )?;
    let config_path = root.join(KALAM_TOML);
    fs::write(&config_path, contents)?;
    KalamProjectConfig::load_from_path(&config_path)?;
    Ok(())
}

fn format_languages_array(languages: &[String]) -> String {
    let quoted: Vec<String> = languages
        .iter()
        .map(|language| format!("\"{language}\""))
        .collect();
    format!("[{}]", quoted.join(", "))
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
                template: Some("simple-live".into()),
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
        assert!(temp.path().join("package.json").is_file());
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
                template: None,
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
                template: None,
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
                template: None,
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
    fn init_normalizes_namespace_for_hyphenated_project_name() {
        let temp = TempDir::new().unwrap();
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        run_init(
            InitOptions {
                name: Some("dev-test1".into()),
                schema_mode: Some(SchemaMode::Sql),
                languages: Some(vec!["typescript".into()]),
                template: None,
                server_mode: Some(ServerMode::Local),
                server_url: None,
                yes: true,
                cwd: temp.path().to_path_buf(),
            },
            &output,
        )
        .unwrap();

        let config = KalamProjectConfig::load_from_path(&temp.path().join(KALAM_TOML)).unwrap();
        assert_eq!(config.project.name, "dev-test1");
        assert_eq!(
            config.connection.get("dev").expect("dev env").namespace,
            kalamdb_commons::NamespaceId::new("dev_test1")
        );
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
                template: None,
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
        let template =
            templates::find_template("typescript", "simple-live").expect("simple-live template");
        let schema = template
            .files
            .iter()
            .find(|file| file.project_path == "schema.sql")
            .expect("schema.sql template");
        let rendered_schema =
            render_template_pairs(schema.content, &[("project_name", "demo-app")]).unwrap();
        assert!(rendered_schema.contains("demo-app"));
        assert!(rendered_schema.contains("CREATE TABLE users"));

        let package_json = template
            .files
            .iter()
            .find(|file| file.project_path == "package.json")
            .expect("package.json template");
        let rendered_package =
            render_template_pairs(package_json.content, &[("project_name", "demo-app")]).unwrap();
        assert!(rendered_package.contains(r#""name": "demo-app""#));

        let starter = template
            .files
            .iter()
            .find(|file| file.project_path == "src/index.ts")
            .expect("src/index.ts template");
        let rendered_starter = render_template_pairs(
            starter.content,
            &[("server_url", "http://localhost:2900")],
        )
        .unwrap();
        assert!(rendered_starter.contains("http://localhost:2900"));
        assert!(rendered_starter.contains("liveTable"));
        assert!(rendered_starter.contains("createClient"));
    }

    #[test]
    fn typescript_tsconfig_includes_node_types() {
        let template =
            templates::find_template("typescript", "simple-live").expect("simple-live template");
        let tsconfig = template
            .files
            .iter()
            .find(|file| file.project_path == "tsconfig.json")
            .expect("tsconfig.json template");
        assert!(tsconfig.content.contains(r#""types": ["node"]"#));
    }
}
