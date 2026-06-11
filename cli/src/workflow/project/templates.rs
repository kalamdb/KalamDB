//! Embedded project templates compiled into the CLI binary.

include!(concat!(env!("OUT_DIR"), "/embedded_templates.rs"));

use crate::error::{CLIError, Result};

pub const DEFAULT_TYPESCRIPT_TEMPLATE: &str = "simple-live";

pub fn templates_for_language(language: &str) -> Vec<&'static EmbeddedTemplate> {
    EMBEDDED_TEMPLATES
        .iter()
        .filter(|template| template.language == language)
        .collect()
}

pub fn find_template(language: &str, template_id: &str) -> Option<&'static EmbeddedTemplate> {
    EMBEDDED_TEMPLATES
        .iter()
        .find(|template| template.language == language && template.id == template_id)
}

pub fn default_template_for_language(language: &str) -> Result<&'static EmbeddedTemplate> {
    let templates = templates_for_language(language);
    if templates.is_empty() {
        return Err(CLIError::ConfigurationError(format!(
            "no built-in templates available for language '{language}'"
        )));
    }
    Ok(templates[0])
}

pub fn resolve_typescript_template(template_id: Option<&str>) -> Result<&'static EmbeddedTemplate> {
    if let Some(template_id) = template_id {
        find_template("typescript", template_id).ok_or_else(|| {
            CLIError::ConfigurationError(format!(
                "unknown typescript template '{template_id}'"
            ))
        })
    } else {
        default_template_for_language("typescript")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_templates_include_simple_live() {
        let template = find_template("typescript", "simple-live").expect("simple-live template");
        assert!(!template.description.is_empty());
        assert!(template.files.iter().any(|file| file.project_path == "schema.sql"));
        assert!(template.files.iter().any(|file| file.project_path == "src/index.ts"));
    }
}
