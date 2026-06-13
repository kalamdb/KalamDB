//! TypeScript-specific `kalam init` helpers: templates, package managers, and scaffolding.

use std::{env, fs, path::Path};

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    terminal_ui::{self, SelectOption},
    workflow::{
        display_project_path,
        project::{
            guidance::init_repository_templates_unavailable,
            prompts::prompt_select,
            scaffold,
            templates::{render_template_pairs, EmbeddedTemplate},
            ts::{
                guidance::init_no_templates,
                package_manager::{
                    resolve_package_manager_for_init, PackageManager, PackageManagerInitOptions,
                },
                templates::{self, DEFAULT_TEMPLATE},
            },
        },
    },
};

pub const SKIP_PACKAGE_INSTALL_ENV: &str = "KALAM_TEST_SKIP_PACKAGE_INSTALL";

pub const SCHEMA_TARGET_OUTPUT: &str = "src/generated/kalam.ts";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemplateSource {
    Builtin,
    Repository,
}

pub fn is_enabled(languages: &[String]) -> bool {
    languages.iter().any(|language| language == "typescript")
}

pub fn resolve_package_manager(
    languages: &[String],
    explicit: Option<PackageManager>,
    non_interactive: bool,
    output: &WorkflowOutput,
) -> Result<Option<PackageManager>> {
    if !is_enabled(languages) {
        return Ok(None);
    }

    let manager = resolve_package_manager_for_init(PackageManagerInitOptions {
        explicit,
        non_interactive,
        color: output.use_color,
        detail: &|message: &str| output.detail(message),
    })?;
    Ok(Some(manager))
}

pub fn resolve_template(
    languages: &[String],
    template_id: Option<&str>,
    non_interactive: bool,
    color: bool,
) -> Result<Option<&'static EmbeddedTemplate>> {
    if !is_enabled(languages) {
        return Ok(None);
    }

    if let Some(template_id) = template_id.map(str::trim).filter(|value| !value.is_empty()) {
        return templates::resolve(Some(template_id)).map(Some);
    }

    if non_interactive {
        return templates::resolve(Some(DEFAULT_TEMPLATE)).map(Some);
    }

    let source_options = [
        SelectOption::described("Built-in templates", "Use templates bundled with the CLI"),
        SelectOption::disabled(
            "From repository",
            "Load a template from a local path (coming soon)",
        ),
    ];
    let source_selected = prompt_select("Template source", &source_options, 0, color)?;
    let source = match source_selected {
        0 => TemplateSource::Builtin,
        _ => TemplateSource::Repository,
    };

    if matches!(source, TemplateSource::Repository) {
        return Err(CLIError::ConfigurationError(init_repository_templates_unavailable()));
    }

    let available = templates::available();
    if available.is_empty() {
        return Err(CLIError::ConfigurationError(init_no_templates()));
    }

    let template_options: Vec<SelectOption<'_>> = available
        .iter()
        .map(|template| SelectOption::described(template.id, template.description))
        .collect();
    let default_index = available
        .iter()
        .position(|template| template.id == DEFAULT_TEMPLATE)
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

pub fn install_dependencies<F>(
    root: &Path,
    package_manager: Option<PackageManager>,
    output: &WorkflowOutput,
    installer: &mut F,
) -> Result<()>
where
    F: FnMut(&Path, PackageManager) -> Result<()>,
{
    if env::var_os(SKIP_PACKAGE_INSTALL_ENV).is_some() {
        output.detail("skipped package install step");
        return Ok(());
    }

    let Some(manager) = package_manager else {
        return Ok(());
    };

    output.status(manager.install_description());
    match installer(root, manager) {
        Ok(()) => {
            output.status(manager.installed_success_message());
            Ok(())
        },
        Err(error) => {
            output.warn(
                "dependency install failed, but project files are on disk — see the error details below",
            );
            Err(error)
        },
    }
}

pub fn apply_scaffold(
    root: &Path,
    template: &EmbeddedTemplate,
    project_name: &str,
    server_url: &str,
    namespace: &str,
    output: &WorkflowOutput,
) -> Result<()> {
    let replacements = [
        ("project_name", project_name),
        ("server_url", server_url),
        ("namespace", namespace),
    ];
    for file in template.files {
        let destination_path = root.join(file.project_path);
        if destination_path.exists() {
            continue;
        }
        if let Some(parent) = destination_path.parent() {
            scaffold::io_with_guidance(
                "create template parent directory",
                parent,
                fs::create_dir_all(parent),
            )?;
        }

        let rendered = render_template_pairs(file.content, &replacements)?;
        scaffold::io_with_guidance(
            "write template file",
            &destination_path,
            fs::write(&destination_path, rendered),
        )?;
        output.detail(format!("created {}", display_project_path(root, &destination_path)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::WorkflowLoggingPolicy,
        output::WorkflowOutput,
        workflow::project::{
            config::SchemaMode,
            init::{run_init_with_installer, InitOptions, ServerMode},
            templates,
            ts::package_manager::{detect_installed_package_managers, PackageManager},
        },
    };
    use std::{fs, path::PathBuf};
    use tempfile::TempDir;

    #[test]
    fn init_typescript_project_requests_package_install() {
        let available = detect_installed_package_managers();
        let Some(manager) = available.first().copied() else {
            return;
        };

        let temp = TempDir::new().unwrap();
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let mut observed: Option<(PathBuf, PackageManager)> = None;
        run_init_with_installer(
            InitOptions {
                name: Some("demo-ts".into()),
                schema_mode: Some(SchemaMode::Sql),
                languages: Some(vec!["typescript".into()]),
                template: Some("simple-live".into()),
                package_manager: Some(manager),
                server_mode: Some(ServerMode::Local),
                server_url: None,
                yes: true,
                cwd: temp.path().to_path_buf(),
            },
            &output,
            |root, selected| {
                observed = Some((root.to_path_buf(), selected));
                Ok(())
            },
        )
        .expect("init should succeed");

        let (root, selected) = observed.expect("typescript init should request a package install");
        assert_eq!(root, temp.path());
        assert_eq!(selected, manager);
        let config = crate::workflow::project::config::KalamProjectConfig::load_from_path(
            &temp.path().join("kalam.toml"),
        )
        .unwrap();
        assert_eq!(config.project.package_manager.as_deref(), Some(manager.as_str()));
    }

    #[test]
    fn init_writes_package_manager_to_kalam_toml_when_typescript_selected() {
        let available = detect_installed_package_managers();
        let Some(manager) = available.first().copied() else {
            return;
        };

        let temp = TempDir::new().unwrap();
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        run_init_with_installer(
            InitOptions {
                name: Some("demo-ts".into()),
                schema_mode: Some(SchemaMode::Sql),
                languages: Some(vec!["typescript".into()]),
                template: Some("simple-live".into()),
                package_manager: Some(manager),
                server_mode: Some(ServerMode::Local),
                server_url: None,
                yes: true,
                cwd: temp.path().to_path_buf(),
            },
            &output,
            |_, _| Ok(()),
        )
        .expect("init should succeed");

        let kalam_toml = fs::read_to_string(temp.path().join("kalam.toml")).unwrap();
        assert!(kalam_toml.contains(&format!("package_manager = \"{}\"", manager.as_str())));
        let config = crate::workflow::project::config::KalamProjectConfig::load_from_path(
            &temp.path().join("kalam.toml"),
        )
        .unwrap();
        assert_eq!(config.project.package_manager.as_deref(), Some(manager.as_str()));
    }

    #[test]
    fn init_skips_package_install_when_env_var_set() {
        let available = detect_installed_package_managers();
        let Some(manager) = available.first().copied() else {
            return;
        };

        let temp = TempDir::new().unwrap();
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let mut called = false;
        env::set_var(SKIP_PACKAGE_INSTALL_ENV, "1");
        run_init_with_installer(
            InitOptions {
                name: Some("demo-ts".into()),
                schema_mode: Some(SchemaMode::Sql),
                languages: Some(vec!["typescript".into()]),
                template: Some("simple-live".into()),
                package_manager: Some(manager),
                server_mode: Some(ServerMode::Local),
                server_url: None,
                yes: true,
                cwd: temp.path().to_path_buf(),
            },
            &output,
            |_, _| {
                called = true;
                Ok(())
            },
        )
        .expect("init should succeed");
        env::remove_var(SKIP_PACKAGE_INSTALL_ENV);

        assert!(!called, "install should be skipped when env var is set");
    }

    #[test]
    fn project_templates_render_expected_values() {
        use crate::workflow::project::templates::render_template_pairs;

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
            &[
                ("server_url", "http://localhost:2900"),
                ("namespace", "demo_app"),
            ],
        )
        .unwrap();
        assert!(rendered_starter.contains("http://localhost:2900"));
        assert!(rendered_starter.contains("demo_app"));
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
        assert!(tsconfig.content.contains("\"types\": [\"node\"]"));
        assert!(tsconfig.content.contains("\"rootDir\": \"./src\""));
    }
}
