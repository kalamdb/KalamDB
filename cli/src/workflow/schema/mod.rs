pub mod dart;
pub mod diff;
pub mod gen;
pub mod load;
pub mod model;
pub mod naming;
pub mod output;
pub mod procedure_bindings;
pub mod procedures;
pub mod rust;
pub mod types;
pub mod typescript;

pub use diff::diff_project_schema_files;
pub use gen::{generate_schema_artifacts, GenerateOptions};
pub use load::compile_project_contract;
pub use model::LanguageTarget;

use crate::{error::Result, workflow::WorkflowContext};

pub fn generate_schema(ctx: &WorkflowContext, languages: Option<Vec<String>>) -> Result<()> {
    let output = ctx.output();
    if let Some(requested) = languages.as_ref() {
        gen::validate_language_filter(requested, &ctx.config.schema.languages)?;
    }
    generate_schema_artifacts(ctx, &GenerateOptions { languages }, &output)
}

#[cfg(test)]
mod generate_tests;
