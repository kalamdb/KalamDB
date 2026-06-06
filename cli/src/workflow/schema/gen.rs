//! Schema generation handlers.

use std::path::Path;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        project::config::KalamProjectConfig,
        schema::{
            emitters::{emit_for_language, write_generated_file},
            load::load_schema_snapshot,
            model::{parse_language_list, LanguageTarget},
        },
    },
};

pub struct GenerateOptions {
    pub languages: Option<Vec<String>>,
}

pub fn generate_schema_artifacts(
    project_root: &Path,
    config: &KalamProjectConfig,
    options: &GenerateOptions,
    output: &WorkflowOutput,
) -> Result<()> {
    let snapshot = load_schema_snapshot(project_root, config)?;
    let languages = options
        .languages
        .as_ref()
        .map(|values| parse_language_list(values))
        .unwrap_or_else(|| parse_language_list(&config.schema.languages));

    if languages.is_empty() {
        return Err(CLIError::ConfigurationError(
            "no language targets selected for generation".into(),
        ));
    }

    for language in languages {
        let key = language.as_str();
        let Some(target) = config.schema.targets.get(key) else {
            return Err(CLIError::ConfigurationError(format!(
                "missing schema.targets.{key} in kalam.toml"
            )));
        };

        let contents = emit_for_language(language, &snapshot)?;
        let output_path = project_root.join(&target.output);
        write_generated_file(&output_path, &contents)?;
        output.status(format!("generated {} -> {}", key, target.output));
    }

    Ok(())
}

pub fn validate_language_filter(
    requested: &[String],
    configured: &[String],
) -> Result<Vec<String>> {
    for value in requested {
        if LanguageTarget::parse(value).is_none() {
            return Err(CLIError::ConfigurationError(format!(
                "unsupported language target '{value}'; supported: typescript, dart"
            )));
        }
        if !configured.iter().any(|lang| lang == value) {
            return Err(CLIError::ConfigurationError(format!(
                "language '{value}' is not enabled in kalam.toml schema.languages"
            )));
        }
    }
    Ok(requested.to_vec())
}
