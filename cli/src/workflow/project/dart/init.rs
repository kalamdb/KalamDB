//! Dart/Flutter-specific `kalam init` helpers: templates and Flutter bootstrap.

use std::{env, path::Path, process::Command};

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    terminal_ui::{self, SelectOption},
    workflow::project::{
        dart::templates::{self, DEFAULT_TEMPLATE},
        prompts::prompt_select,
        templates::EmbeddedTemplate,
        ts::{apply_scaffold, SKIP_PACKAGE_INSTALL_ENV},
    },
};

pub const SCHEMA_TARGET_OUTPUT: &str = "lib/generated/kalam.dart";
pub const DEFAULT_DEV_COMMAND: &str = "flutter run";

pub fn is_enabled(languages: &[String]) -> bool {
    languages.iter().any(|language| language == "dart")
}

pub fn resolve_starter(
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

    let available = templates::available();
    if available.is_empty() {
        return Err(CLIError::ConfigurationError(
            "no built-in Dart/Flutter templates are available in this CLI build".into(),
        ));
    }

    let options: Vec<SelectOption<'static>> = available
        .iter()
        .map(|template| SelectOption::described(template.id, template.description))
        .collect();
    let default_index =
        available.iter().position(|template| template.id == DEFAULT_TEMPLATE).unwrap_or(0);
    let selected = prompt_select("Dart/Flutter project template", &options, default_index, color)?;
    let template = available[selected];
    eprintln!(
        "{} {}",
        terminal_ui::prompt_label("Dart template:", color),
        terminal_ui::style_value(template.id, color)
    );
    Ok(Some(template))
}

pub fn apply_dart_scaffold(
    root: &Path,
    template: &EmbeddedTemplate,
    project_name: &str,
    server_url: &str,
    namespace: &str,
    output: &WorkflowOutput,
) -> Result<()> {
    apply_scaffold(root, template, project_name, server_url, namespace, output)
}

pub fn maybe_bootstrap_flutter_project(
    root: &Path,
    package_name: &str,
    output: &WorkflowOutput,
) -> Result<()> {
    if env::var_os(SKIP_PACKAGE_INSTALL_ENV).is_some() {
        output.detail("skipped flutter create step");
        return Ok(());
    }

    if Command::new("flutter").arg("--version").output().is_err() {
        output.detail("skipped flutter create; flutter was not found on PATH");
        return Ok(());
    }

    output.status("creating Flutter platform files");
    let create = Command::new("flutter")
        .current_dir(root)
        .args(["create", ".", "--project-name", package_name, "--platforms", "macos,web"])
        .output()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("failed to run flutter create: {error}"))
        })?;
    if !create.status.success() {
        output.warn(
            "flutter create failed, but project files are on disk — add platforms with `flutter create .`",
        );
        return Ok(());
    }

    let pub_get = Command::new("flutter").current_dir(root).args(["pub", "get"]).output();
    match pub_get {
        Ok(output_status) if output_status.status.success() => {
            output.status("installed Flutter packages");
        },
        _ => {
            output.warn("flutter pub get failed; run `flutter pub get` in the project directory");
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dart_starter_defaults_to_simple_live() {
        let template = resolve_starter(&["dart".into()], None, true, false)
            .expect("starter should resolve")
            .expect("starter should be selected");
        assert_eq!(template.id, "simple-live");
        assert_eq!(template.language, "dart");
    }

    #[test]
    fn dart_starter_is_skipped_without_dart_language() {
        let starter =
            resolve_starter(&["typescript".into()], Some("simple-live"), true, false).unwrap();
        assert!(starter.is_none());
    }

    #[test]
    fn dart_simple_live_template_renders_flutter_starter() {
        use crate::workflow::project::templates::{find_template, render_template_pairs};

        let template = find_template("dart", "simple-live").expect("dart simple-live template");
        let schema = template
            .files
            .iter()
            .find(|file| file.project_path == "schema.sql")
            .expect("schema.sql template");
        let rendered_schema =
            render_template_pairs(schema.content, &[("project_name", "demo-app")]).unwrap();
        assert!(rendered_schema.contains("demo-app"));
        assert!(rendered_schema.contains("CREATE TABLE users"));

        let pubspec = template
            .files
            .iter()
            .find(|file| file.project_path == "pubspec.yaml")
            .expect("pubspec.yaml template");
        assert!(pubspec.content.contains("kalam_sync"));
        assert!(pubspec.content.contains("sdk: flutter"));

        let main = template
            .files
            .iter()
            .find(|file| file.project_path == "lib/main.dart")
            .expect("lib/main.dart template");
        let rendered_main = render_template_pairs(
            main.content,
            &[
                ("server_url", "http://localhost:2900"),
                ("namespace", "demo_app"),
                ("project_name", "demo-app"),
            ],
        )
        .unwrap();
        assert!(rendered_main.contains("http://localhost:2900"));
        assert!(rendered_main.contains("demo_app"));
        assert!(rendered_main.contains("KalamTables.users"));
        assert!(rendered_main.contains("Kalam.open"));
    }
}
