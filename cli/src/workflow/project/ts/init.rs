//! TypeScript-specific `kalam init` helpers: templates, package managers, and scaffolding.

use std::{env, fs, path::Path};

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    terminal_ui::{self, SelectOption},
    workflow::{
        display_project_path,
        project::{
            prompts::prompt_select,
            repository_examples::{self, RepositoryExample},
            scaffold,
            templates::{render_template_pairs_for_path, EmbeddedTemplate},
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

#[derive(Debug, Clone, Copy)]
pub enum ProjectStarter {
    Embedded(&'static EmbeddedTemplate),
    Repository(&'static RepositoryExample),
}

impl ProjectStarter {
    pub fn id(self) -> &'static str {
        match self {
            Self::Embedded(template) => template.id,
            Self::Repository(example) => example.id,
        }
    }
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

pub fn resolve_starter(
    languages: &[String],
    template_id: Option<&str>,
    non_interactive: bool,
    color: bool,
) -> Result<Option<ProjectStarter>> {
    if !is_enabled(languages) {
        return Ok(None);
    }

    if let Some(template_id) = template_id.map(str::trim).filter(|value| !value.is_empty()) {
        if let Ok(template) = templates::resolve(Some(template_id)) {
            return Ok(Some(ProjectStarter::Embedded(template)));
        }
        if let Some(example) = repository_examples::find(template_id) {
            return Ok(Some(ProjectStarter::Repository(example)));
        }
        return Err(CLIError::ConfigurationError(format!(
            "unknown project template or example '{template_id}'"
        )));
    }

    if non_interactive {
        return templates::resolve(Some(DEFAULT_TEMPLATE))
            .map(ProjectStarter::Embedded)
            .map(Some);
    }

    let embedded_templates = templates::available();
    let repository_examples = repository_examples::available();
    if embedded_templates.is_empty() && repository_examples.is_empty() {
        return Err(CLIError::ConfigurationError(init_no_templates()));
    }

    let template_options = project_sample_options(&embedded_templates);
    let default_index = embedded_templates
        .iter()
        .position(|template| template.id == DEFAULT_TEMPLATE)
        .unwrap_or(0);
    let selected = prompt_select("Project template", &template_options, default_index, color)?;
    let starter = if selected < embedded_templates.len() {
        ProjectStarter::Embedded(embedded_templates[selected])
    } else {
        ProjectStarter::Repository(&repository_examples[selected - embedded_templates.len()])
    };
    eprintln!(
        "{} {}",
        terminal_ui::prompt_label("Template:", color),
        terminal_ui::style_value(starter.id(), color)
    );
    Ok(Some(starter))
}

fn project_sample_options(available: &[&'static EmbeddedTemplate]) -> Vec<SelectOption<'static>> {
    let mut options: Vec<SelectOption<'static>> = available
        .iter()
        .map(|template| SelectOption::described(template.id, template.description))
        .collect();
    options.extend(
        repository_examples::available()
            .iter()
            .map(|example| SelectOption::described(example.id, example.description)),
    );
    options
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

    let result = {
        let _spinner = output.status_spinner(manager.install_description());
        installer(root, manager)
    };
    match result {
        Ok(()) => {
            output.status(manager.installed_success_message());
            Ok(())
        },
        Err(error) => {
            output.warn(
                "dependency install failed, but project files are on disk — see the error details \
                 below",
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
        ("kalam_sdk_version", crate::CLI_VERSION),
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

        let rendered =
            render_template_pairs_for_path(file.project_path, file.content, &replacements)?;
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
    use std::{fs, path::PathBuf};

    use tempfile::TempDir;

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

    fn block_on_init<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Runtime::new().expect("tokio runtime").block_on(future)
    }

    #[test]
    fn init_typescript_project_requests_package_install() {
        let available = detect_installed_package_managers();
        let Some(manager) = available.first().copied() else {
            return;
        };

        crate::workflow::test_support::without_test_env_var(SKIP_PACKAGE_INSTALL_ENV, || {
            let temp = TempDir::new().unwrap();
            let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
            let mut observed: Option<(PathBuf, PackageManager)> = None;
            block_on_init(run_init_with_installer(
                InitOptions {
                    name:            Some("demo-ts".into()),
                    schema_mode:     Some(SchemaMode::Sql),
                    languages:       Some(vec!["typescript".into()]),
                    template:        Some("simple-live".into()),
                    package_manager: Some(manager),
                    server_mode:     Some(ServerMode::Local),
                    server_url:      None,
                    yes:             true,
                    cwd:             temp.path().to_path_buf(),
                },
                &output,
                |root, selected| {
                    observed = Some((root.to_path_buf(), selected));
                    Ok(())
                },
            ))
            .expect("init should succeed");

            let (root, selected) =
                observed.expect("typescript init should request a package install");
            assert_eq!(root, temp.path());
            assert_eq!(selected, manager);
            let config = crate::workflow::project::config::KalamProjectConfig::load_from_path(
                &temp.path().join("kalam.toml"),
            )
            .unwrap();
            assert_eq!(config.project.package_manager.as_deref(), Some(manager.as_str()));
        });
    }

    #[test]
    fn init_writes_package_manager_to_kalam_toml_when_typescript_selected() {
        let available = detect_installed_package_managers();
        let Some(manager) = available.first().copied() else {
            return;
        };

        crate::workflow::test_support::without_test_env_var(SKIP_PACKAGE_INSTALL_ENV, || {
            let temp = TempDir::new().unwrap();
            let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
            block_on_init(run_init_with_installer(
                InitOptions {
                    name:            Some("demo-ts".into()),
                    schema_mode:     Some(SchemaMode::Sql),
                    languages:       Some(vec!["typescript".into()]),
                    template:        Some("simple-live".into()),
                    package_manager: Some(manager),
                    server_mode:     Some(ServerMode::Local),
                    server_url:      None,
                    yes:             true,
                    cwd:             temp.path().to_path_buf(),
                },
                &output,
                |_, _| Ok(()),
            ))
            .expect("init should succeed");

            let kalam_toml = fs::read_to_string(temp.path().join("kalam.toml")).unwrap();
            assert!(kalam_toml.contains(&format!("package_manager = \"{}\"", manager.as_str())));
            let config = crate::workflow::project::config::KalamProjectConfig::load_from_path(
                &temp.path().join("kalam.toml"),
            )
            .unwrap();
            assert_eq!(config.project.package_manager.as_deref(), Some(manager.as_str()));
        });
    }

    #[test]
    fn init_skips_package_install_when_env_var_set() {
        let available = detect_installed_package_managers();
        let Some(manager) = available.first().copied() else {
            return;
        };

        crate::workflow::test_support::with_test_env_var(SKIP_PACKAGE_INSTALL_ENV, "1", || {
            let temp = TempDir::new().unwrap();
            let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
            let mut called = false;
            block_on_init(run_init_with_installer(
                InitOptions {
                    name:            Some("demo-ts".into()),
                    schema_mode:     Some(SchemaMode::Sql),
                    languages:       Some(vec!["typescript".into()]),
                    template:        Some("simple-live".into()),
                    package_manager: Some(manager),
                    server_mode:     Some(ServerMode::Local),
                    server_url:      None,
                    yes:             true,
                    cwd:             temp.path().to_path_buf(),
                },
                &output,
                |_, _| {
                    called = true;
                    Ok(())
                },
            ))
            .expect("init should succeed");

            assert!(!called, "install should be skipped when env var is set");
        });
    }

    #[test]
    fn install_dependencies_emits_running_and_success_status() {
        crate::workflow::test_support::without_test_env_var(SKIP_PACKAGE_INSTALL_ENV, || {
            let temp = TempDir::new().unwrap();
            let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled())
                .with_animations(false);
            install_dependencies(temp.path(), Some(PackageManager::Npm), &output, &mut |_, _| {
                Ok(())
            })
            .expect("install should succeed");

            let lines = output.test_buffered_terminal_lines();
            assert!(
                lines.iter().any(|line| line == "[cli] installing npm dependencies"),
                "expected running status, got {lines:?}"
            );
            assert!(
                lines.iter().any(|line| line == "[cli] installed npm dependencies"),
                "expected success status, got {lines:?}"
            );
        });
    }

    #[test]
    fn project_sample_options_use_embedded_template_metadata() {
        let available = crate::workflow::project::ts::templates::available();
        let options = project_sample_options(&available);
        assert!(
            options.iter().any(|option| option.label == "simple-live"
                && option.description == Some("Live subscription starter with sample inserts")),
            "expected simple-live sample option to come from embedded template metadata"
        );
    }

    #[test]
    fn project_sample_options_include_repository_examples() {
        let available = crate::workflow::project::ts::templates::available();
        let options = project_sample_options(&available);
        assert!(
            options.iter().any(|option| option.label == "chat-with-ai"
                && option.description == Some("Topic-driven React chat with an agent worker")),
            "expected chat-with-ai repository example to be offered during init"
        );
    }

    #[test]
    fn explicit_template_can_select_repository_example() {
        let starter =
            resolve_starter(&["typescript".to_string()], Some("chat-with-ai"), true, false)
                .expect("starter should resolve")
                .expect("starter should be selected");

        assert!(matches!(
            starter,
            ProjectStarter::Repository(example) if example.id == "chat-with-ai"
        ));
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
        let rendered_package = render_template_pairs(
            package_json.content,
            &[
                ("project_name", "demo-app"),
                ("kalam_sdk_version", crate::CLI_VERSION),
            ],
        )
        .unwrap();
        assert!(rendered_package.contains(r#""name": "demo-app""#));
        assert!(
            rendered_package.contains(&format!(r#""@kalamdb/client": "{}""#, crate::CLI_VERSION))
        );
        assert!(!rendered_package.contains("\"latest\""));

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
        assert!(rendered_starter.contains("dotenv/config"));
        assert!(!rendered_starter.contains("30_000"));
        assert!(rendered_package.contains("\"dotenv\""));
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
