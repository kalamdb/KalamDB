//! `kalam init` project scaffolding.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
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
            guidance::{
                init_config_validation_failed, init_empty_project_name, init_invalid_server_url,
                init_missing_language_targets, init_missing_scaffold_template,
                init_project_already_exists, init_remote_schema_unavailable,
                init_requires_non_interactive_flags, init_stage_context, init_unsupported_language,
            },
            identifiers::{normalize_namespace_name, parse_namespace_id},
            prompts::{
                interactive_available, print_workflow_banner, prompt_multi_select, prompt_select,
                prompt_text, prompt_text_with_default,
            },
            scaffold,
            templates::{
                self, find_template_file, render_template, EmbeddedTemplate,
                DEFAULT_SCAFFOLD_TEMPLATE,
            },
            ts::{
                apply_scaffold, execute_package_install, install_dependencies,
                resolve_package_manager, resolve_template, PackageManager, SCHEMA_TARGET_OUTPUT,
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
    pub template: Option<String>,
    pub package_manager: Option<PackageManager>,
    pub server_mode: Option<ServerMode>,
    pub server_url: Option<String>,
    pub yes: bool,
    pub cwd: PathBuf,
}

pub fn run_init(options: InitOptions, output: &WorkflowOutput) -> Result<()> {
    run_init_with_installer(options, output, execute_package_install)
}

pub(crate) fn run_init_with_installer<F>(
    options: InitOptions,
    output: &WorkflowOutput,
    mut installer: F,
) -> Result<()>
where
    F: FnMut(&Path, PackageManager) -> Result<()>,
{
    if options.cwd.join(KALAM_TOML).exists() {
        return Err(CLIError::ConfigurationError(init_project_already_exists(&options.cwd)));
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
        resolve_template(&languages, options.template.as_deref(), options.yes, output.use_color)?;
    let server_mode = resolve_server_mode(&options, output.use_color)?;
    let server_url = resolve_server_url(&options, server_mode, output.use_color)?;
    let package_manager =
        resolve_package_manager(&languages, options.package_manager, options.yes, output)?;

    let config =
        build_config(&name, schema_mode, &languages, server_mode, &server_url, package_manager)?;
    config.validate().map_err(|error| map_init_config_validation_error(error))?;
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
    )
    .map_err(|error| map_init_stage_error("writing project files", error))?;
    install_dependencies(&options.cwd, package_manager, output, &mut installer)
        .map_err(|error| map_init_stage_error("installing JavaScript dependencies", error))?;
    output.status(format!("initialized KalamDB project '{name}'"));
    Ok(())
}

fn map_init_stage_error(stage: &str, error: CLIError) -> CLIError {
    match error {
        CLIError::ConfigurationError(message) => {
            CLIError::ConfigurationError(init_stage_context(stage, message))
        },
        CLIError::FileError(message) => {
            CLIError::ConfigurationError(init_stage_context(stage, message))
        },
        other => other,
    }
}

fn map_init_config_validation_error(error: CLIError) -> CLIError {
    match error {
        CLIError::ConfigurationError(message) => CLIError::ConfigurationError(init_stage_context(
            "validating project configuration",
            init_config_validation_failed(&message),
        )),
        other => map_init_stage_error("validating project configuration", other),
    }
}

fn ensure_interactive_or_yes(options: &InitOptions) -> Result<()> {
    if options.yes || interactive_available() {
        return Ok(());
    }
    Err(CLIError::ConfigurationError(init_requires_non_interactive_flags()))
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
            Err(CLIError::ConfigurationError(init_empty_project_name()))
        } else {
            Ok(trimmed.to_string())
        }
    })
}

fn resolve_schema_mode(options: &InitOptions, color: bool) -> Result<SchemaMode> {
    if let Some(mode) = options.schema_mode {
        if matches!(mode, SchemaMode::Remote) {
            return Err(CLIError::ConfigurationError(init_remote_schema_unavailable()));
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
        return Err(CLIError::ConfigurationError(init_remote_schema_unavailable()));
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
    Url::parse(value).map_err(|error| {
        CLIError::ConfigurationError(init_invalid_server_url(value, &error.to_string()))
    })?;
    Ok(value.to_string())
}

fn normalize_languages(languages: &[String]) -> Result<Vec<String>> {
    if languages.is_empty() {
        return Err(CLIError::ConfigurationError(init_missing_language_targets()));
    }

    let mut normalized = Vec::new();
    for language in languages {
        match language.trim().to_ascii_lowercase().as_str() {
            "typescript" | "ts" => normalized.push("typescript".into()),
            "dart" => normalized.push("dart".into()),
            other => {
                return Err(CLIError::ConfigurationError(init_unsupported_language(other)));
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
    package_manager: Option<PackageManager>,
) -> Result<KalamProjectConfig> {
    let mut targets = HashMap::new();
    for language in languages {
        let output = match language.as_str() {
            "typescript" => SCHEMA_TARGET_OUTPUT,
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
            package_manager: package_manager.map(PackageManager::as_str).map(str::to_string),
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
            processes: package_manager
                .map(|manager| HashMap::from([("app".into(), manager.dev_run_command().into())]))
                .unwrap_or_default(),
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
    scaffold::io_with_guidance("create project directory", root, fs::create_dir_all(root))?;

    write_kalam_toml_from_template(root, config, server_mode, server_url)?;
    output.detail(format!("created {}", KALAM_TOML));

    write_default_gitignore(root, &config.schema.languages, output)?;
    let kalam_gitignore = config.ensure_kalam_gitignore(root)?;
    output.detail(format!("created {}", display_project_path(root, &kalam_gitignore)));
    let cli_log_dir = config.ensure_cli_log_dir(root)?;
    output.detail(format!("created {}/", display_project_path(root, &cli_log_dir)));

    if let Some(template) = typescript_template {
        let namespace = config
            .connection
            .get("dev")
            .map(|connection| connection.namespace.as_str())
            .unwrap_or("");
        apply_scaffold(root, template, &config.project.name, server_url, namespace, output)?;
    } else if matches!(schema_mode, SchemaMode::Sql) {
        let schema_path = root.join("schema.sql");
        if !schema_path.exists() {
            scaffold::io_with_guidance(
                "write schema file",
                &schema_path,
                fs::write(&schema_path, "-- Add your schema here\n"),
            )?;
            output.detail(format!("created {}", display_project_path(root, &schema_path)));
        }
    }

    if matches!(server_mode, ServerMode::Local) {
        let port = crate::workflow::dev::server::parse_server_port(server_url)?;
        let server_config_path = config.local_server_config_path(root);
        let created = !server_config_path.exists();
        write_local_server_config(root, config, port)?;
        if created {
            output.detail(format!("created {}", display_project_path(root, &server_config_path)));
        }
    }

    let migrations_dir = config.migrations_dir(root);
    scaffold::io_with_guidance(
        "create migrations directory",
        &migrations_dir,
        fs::create_dir_all(&migrations_dir),
    )?;
    let gitkeep = migrations_dir.join(".gitkeep");
    if !gitkeep.exists() {
        scaffold::io_with_guidance(
            "write migrations placeholder",
            &gitkeep,
            fs::write(&gitkeep, ""),
        )?;
    }
    output.detail(format!("created {}/", display_project_path(root, &migrations_dir)));

    for language in &config.schema.languages {
        if let Some(target) = config.schema.targets.get(language) {
            let out_path = root.join(&target.output);
            if let Some(parent) = out_path.parent() {
                scaffold::io_with_guidance(
                    "create generated output directory",
                    parent,
                    fs::create_dir_all(parent),
                )?;
            }
            output.detail(format!(
                "created {}",
                display_project_path(root, out_path.parent().unwrap_or(&out_path))
            ));
        }
    }

    let default_profile =
        crate::workflow::project::resolve::credential_instance_for_env(&config.project.default_env);
    let namespace = config
        .connection
        .get("dev")
        .map(|connection| connection.namespace.as_str())
        .unwrap_or("");
    let env_template = scaffold_template_file(".env.example")?;
    let env_contents = render_template(
        env_template,
        &json!({
            "default_profile": default_profile,
            "namespace": namespace,
        }),
    )?;

    let env_file = root.join(".env");
    if !env_file.exists() {
        scaffold::io_with_guidance("write .env", &env_file, fs::write(&env_file, &env_contents))?;
        output.detail("created .env");
    }

    let env_example = root.join(".env.example");
    if !env_example.exists() {
        scaffold::io_with_guidance(
            "write .env.example",
            &env_example,
            fs::write(&env_example, env_contents),
        )?;
        output.detail("created .env.example");
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
    scaffold::io_with_guidance(
        "write .gitignore",
        &gitignore_path,
        fs::write(&gitignore_path, contents),
    )?;
    output.detail("created .gitignore");
    Ok(())
}

fn scaffold_template_file(project_path: &str) -> Result<&'static str> {
    let template = templates::resolve_scaffold_template()?;
    find_template_file(template, project_path).ok_or_else(|| {
        CLIError::ConfigurationError(init_missing_scaffold_template(
            project_path,
            DEFAULT_SCAFFOLD_TEMPLATE,
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
    let dev_process_command = config.dev.processes.get("app").cloned().unwrap_or_default();
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
            "package_manager": config.project.package_manager.as_deref().unwrap_or(""),
            "dev_process_command": dev_process_command,
        }),
    )?;
    let config_path = root.join(KALAM_TOML);
    scaffold::io_with_guidance(
        "write kalam.toml",
        &config_path,
        fs::write(&config_path, contents),
    )?;
    KalamProjectConfig::load_from_path(&config_path)?;
    Ok(())
}

fn format_languages_array(languages: &[String]) -> String {
    let quoted: Vec<String> = languages.iter().map(|language| format!("\"{language}\"")).collect();
    format!("[{}]", quoted.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::WorkflowLoggingPolicy, workflow::project::ts::SKIP_PACKAGE_INSTALL_ENV};
    use std::{env, fs};
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
                package_manager: None,
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
        let kalam_toml = fs::read_to_string(temp.path().join(KALAM_TOML)).unwrap();
        assert!(kalam_toml.contains("[dev.processes]"));
        assert!(kalam_toml.contains("run dev"));
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
                package_manager: None,
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
        assert!(env_contents.contains("KALAM_NAMESPACE=demo_app"));
        assert!(env_contents.contains("KALAM_USER=root"));
        assert!(env_contents.contains("KALAM_PASSWORD="));

        let gitignore = fs::read_to_string(temp.path().join(".gitignore")).unwrap();
        assert!(gitignore.lines().any(|line| line.trim() == ".env"));
    }

    #[test]
    fn init_dart_project_omits_package_manager_from_kalam_toml() {
        let temp = TempDir::new().unwrap();
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let original_skip = env::var_os(SKIP_PACKAGE_INSTALL_ENV);
        env::set_var(SKIP_PACKAGE_INSTALL_ENV, "1");

        run_init(
            InitOptions {
                name: Some("demo-dart".into()),
                schema_mode: Some(SchemaMode::Sql),
                languages: Some(vec!["dart".into()]),
                template: None,
                package_manager: None,
                server_mode: Some(ServerMode::Local),
                server_url: None,
                yes: true,
                cwd: temp.path().to_path_buf(),
            },
            &output,
        )
        .unwrap();

        match original_skip {
            Some(value) => env::set_var(SKIP_PACKAGE_INSTALL_ENV, value),
            None => env::remove_var(SKIP_PACKAGE_INSTALL_ENV),
        }

        let kalam_toml = fs::read_to_string(temp.path().join(KALAM_TOML)).unwrap();
        assert!(!kalam_toml.contains("package_manager"));
        assert!(kalam_toml.contains("# [dev.processes]"));
        assert!(!kalam_toml.contains("\n[dev.processes]\n"));

        let config = KalamProjectConfig::load_from_path(&temp.path().join(KALAM_TOML)).unwrap();
        assert!(config.project.package_manager.is_none());
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
                package_manager: None,
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
                package_manager: None,
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

        let env_contents = fs::read_to_string(temp.path().join(".env")).unwrap();
        assert!(env_contents.contains("KALAM_NAMESPACE=dev_test1"));
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
                package_manager: None,
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
}
