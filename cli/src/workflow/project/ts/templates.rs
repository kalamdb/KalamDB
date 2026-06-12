//! Built-in TypeScript project templates bundled with the CLI.

use crate::{
    error::{CLIError, Result},
    workflow::project::templates::{
        default_template_for_language, find_template, templates_for_language, EmbeddedTemplate,
    },
};

pub const DEFAULT_TEMPLATE: &str = "simple-live";

pub fn available() -> Vec<&'static EmbeddedTemplate> {
    templates_for_language("typescript")
}

pub fn resolve(template_id: Option<&str>) -> Result<&'static EmbeddedTemplate> {
    if let Some(template_id) = template_id {
        find_template("typescript", template_id).ok_or_else(|| {
            CLIError::ConfigurationError(format!("unknown typescript template '{template_id}'"))
        })
    } else {
        default_template_for_language("typescript")
    }
}
