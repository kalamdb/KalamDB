use std::{fs, path::Path};

use serde_json::json;

use super::{prompts::schema_mode_label, ServerMode};
use crate::{
    error::Result,
    output::WorkflowOutput,
    workflow::{
        dev::server::{write_local_server_config, DEFAULT_LOCAL_DEV_ROOT_PASSWORD},
        display_project_path,
        project::{
            config::{KalamProjectConfig, SchemaMode, KALAM_TOML},
            connection_url::parse_server_port,
            scaffold,
            templates::{
                find_scaffold_template_file, find_schema_gen_template_file,
                render_kalam_toml_scaffold, render_scaffold_file, EmbeddedTemplate,
                KalamTomlScaffoldInput, TYPESCRIPT_ORM_CODEGEN_PATH,
            },
            ts::apply_scaffold,
        },
    },
};

pub(super) fn write_project_scaffold(
    root: &Path,
    config: &KalamProjectConfig,
    schema_mode: SchemaMode,
    server_mode: ServerMode,
    server_url: &str,
    typescript_template: Option<&EmbeddedTemplate>,
    dart_template: Option<&EmbeddedTemplate>,
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
    }
    if let Some(template) = dart_template {
        let namespace = config
            .connection
            .get("dev")
            .map(|connection| connection.namespace.as_str())
            .unwrap_or("");
        crate::workflow::project::dart::init::apply_dart_scaffold(
            root,
            template,
            &config.project.name,
            server_url,
            namespace,
            output,
        )?;
    }
    if typescript_template.is_none()
        && dart_template.is_none()
        && matches!(schema_mode, SchemaMode::Sql)
    {
        write_scaffold_file_if_missing(
            root,
            &root.join("schema.sql"),
            "schema.sql",
            "write schema file",
            output,
        )?;
    }

    if matches!(server_mode, ServerMode::Local) {
        let port = parse_server_port(server_url)?;
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
    write_scaffold_file_if_missing(
        root,
        &migrations_dir.join(".gitkeep"),
        "kalam/migrations/.gitkeep",
        "write migrations placeholder",
        output,
    )?;
    output.detail(format!("created {}/", display_project_path(root, &migrations_dir)));

    write_scaffold_file_if_missing(
        root,
        &config.seed_path(root),
        "kalam/seed.sql",
        "write development fixtures",
        output,
    )?;

    if config.schema.languages.iter().any(|language| language == "typescript") {
        write_schema_gen_file_if_missing(
            root,
            &root.join(TYPESCRIPT_ORM_CODEGEN_PATH),
            "typescript",
            TYPESCRIPT_ORM_CODEGEN_PATH,
            "write TypeScript ORM codegen script",
            output,
        )?;
    }

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

    let languages = crate::workflow::schema::model::parse_language_list(&config.schema.languages);
    if !languages.is_empty() {
        match crate::workflow::schema::gen::generate_languages(root, config, &languages, None) {
            Ok(()) => {
                for language in &languages {
                    if let Some(target) = config.schema.targets.get(language.as_str()) {
                        output.detail(format!(
                            "generated {}",
                            display_project_path(root, &root.join(&target.output))
                        ));
                    }
                }
            },
            Err(error) => {
                output.detail(format!("skipped schema generation during init: {error}"));
            },
        }
    }

    let default_profile =
        crate::workflow::project::resolve::credential_instance_for_env(&config.project.default_env);
    let namespace = config
        .connection
        .get("dev")
        .map(|connection| connection.namespace.as_str())
        .unwrap_or("");
    let local_root_password =
        matches!(server_mode, ServerMode::Local).then_some(DEFAULT_LOCAL_DEV_ROOT_PASSWORD);
    let env_contents = render_scaffold_file(
        ".env.example",
        &json!({
            "default_profile": default_profile,
            "namespace": namespace,
            "local_root_password": local_root_password,
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

    let contents = render_scaffold_file(
        ".gitignore",
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

fn write_kalam_toml_from_template(
    root: &Path,
    config: &KalamProjectConfig,
    server_mode: ServerMode,
    server_url: &str,
) -> Result<()> {
    let schema_path = config.schema.path.as_deref().unwrap_or("schema.sql");
    let namespace = config
        .connection
        .get("dev")
        .map(|connection| connection.namespace.as_str())
        .unwrap_or("");
    let dev_process_command = config.dev.processes.get("app").cloned().unwrap_or_default();
    let contents = render_kalam_toml_scaffold(&KalamTomlScaffoldInput {
        project_name: &config.project.name,
        namespace,
        server_url,
        schema_mode: schema_mode_label(config.schema.mode),
        schema_path,
        languages: &config.schema.languages,
        auto_start_db: matches!(server_mode, ServerMode::Local),
        package_manager: config.project.package_manager.as_deref(),
        server_version: config.project.server_version.as_deref().unwrap_or(crate::CLI_VERSION),
        dev_process_command: &dev_process_command,
    })?;
    let config_path = root.join(KALAM_TOML);
    scaffold::io_with_guidance(
        "write kalam.toml",
        &config_path,
        fs::write(&config_path, contents),
    )?;
    KalamProjectConfig::load_from_path(&config_path)?;
    Ok(())
}

fn write_scaffold_file_if_missing(
    root: &Path,
    dest: &Path,
    project_path: &str,
    operation: &str,
    output: &WorkflowOutput,
) -> Result<()> {
    write_file_if_missing(root, dest, find_scaffold_template_file(project_path)?, operation, output)
}

fn write_schema_gen_file_if_missing(
    root: &Path,
    dest: &Path,
    id: &str,
    project_path: &str,
    operation: &str,
    output: &WorkflowOutput,
) -> Result<()> {
    write_file_if_missing(
        root,
        dest,
        find_schema_gen_template_file(id, project_path)?,
        operation,
        output,
    )
}

fn write_file_if_missing(
    root: &Path,
    dest: &Path,
    contents: &str,
    operation: &str,
    output: &WorkflowOutput,
) -> Result<()> {
    if dest.exists() {
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        scaffold::io_with_guidance("create parent directory", parent, fs::create_dir_all(parent))?;
    }
    scaffold::io_with_guidance(operation, dest, fs::write(dest, contents))?;
    output.detail(format!("created {}", display_project_path(root, dest)));
    Ok(())
}
