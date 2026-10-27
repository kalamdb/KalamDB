//! Schema generation handlers.

use std::path::Path;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        project::config::KalamProjectConfig,
        schema::{
            dart, load,
            model::{parse_language_list, LanguageTarget},
            naming::{assign_names_with_home, NamingOptions},
            output::SchemaEmitInput,
            procedures::ProcedureCatalog,
            rust, typescript,
        },
        WorkflowContext,
    },
};

#[derive(Default)]
pub struct GenerateOptions {
    pub languages: Option<Vec<String>>,
    pub check: bool,
}

pub fn generate_schema_artifacts(
    ctx: &WorkflowContext,
    options: &GenerateOptions,
    output: &WorkflowOutput,
) -> Result<()> {
    let languages = resolve_languages(&ctx.config, options.languages.as_ref())?;
    if options.check {
        check_languages(&ctx.project_root, &ctx.config, &languages, Some(output))
    } else {
        generate_languages(&ctx.project_root, &ctx.config, &languages, Some(output))
    }
}

pub fn check_languages(
    project_root: &Path,
    config: &KalamProjectConfig,
    languages: &[LanguageTarget],
    output: Option<&WorkflowOutput>,
) -> Result<()> {
    let temp = tempfile::TempDir::new().map_err(|err| {
        CLIError::FileError(format!("failed to create staging directory for --check: {err}"))
    })?;
    let staged_root = temp.path();
    if let Some(schema_rel) = &config.schema.path {
        let src_schema = project_root.join(schema_rel);
        let dst_schema = staged_root.join(schema_rel);
        if src_schema.is_file() {
            if let Some(parent) = dst_schema.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::copy(&src_schema, &dst_schema).ok();
        }
    }
    generate_languages(staged_root, config, languages, None)?;
    for language in languages {
        let key = language.as_str();
        let Some(target) = config.schema.targets.get(key) else {
            continue;
        };
        let staged_file = staged_root.join(&target.output);
        let actual_file = project_root.join(&target.output);
        let expected = std::fs::read(&staged_file).map_err(|err| {
            CLIError::FileError(format!("failed to read staged '{}': {err}", target.output))
        })?;
        let actual = std::fs::read(&actual_file).map_err(|_| {
            CLIError::ConfigurationError(format!(
                "schema gen --check failed: missing generated artifact '{}'",
                target.output
            ))
        })?;
        if actual != expected {
            return Err(CLIError::ConfigurationError(format!(
                "schema gen --check failed: '{}' is out of date with current schema contract",
                target.output
            )));
        }
    }
    if let Some(out) = output {
        out.status("schema gen --check passed: generated artifacts match current contract");
    }
    Ok(())
}

pub fn generate_languages(
    project_root: &Path,
    config: &KalamProjectConfig,
    languages: &[LanguageTarget],
    output: Option<&WorkflowOutput>,
) -> Result<()> {
    if languages.is_empty() {
        return Err(CLIError::ConfigurationError(
            "no language targets selected for generation".into(),
        ));
    }

    let (snapshot, hash) = load::compile_project_contract(project_root, config)?;
    for table in snapshot.tables.values() {
        kalamdb_sql::contracts::table_row_contract(table).map_err(|err| {
            CLIError::ConfigurationError(format!("{} row contract: {err}", table.table_id))
        })?;
    }
    let home_schema = load::project_contract_namespace(config);

    for language in languages {
        let key = language.as_str();
        let Some(target) = config.schema.targets.get(key) else {
            return Err(CLIError::ConfigurationError(format!(
                "missing schema.targets.{key} in kalam.toml"
            )));
        };
        let names = assign_names_with_home(
            &snapshot,
            NamingOptions {
                unqualified_names: target.unqualified_names,
            },
            Some(home_schema),
        )?;
        let output_path = project_root.join(&target.output);
        let procedures = ProcedureCatalog::from_snapshot(&snapshot, &names);
        let input = SchemaEmitInput {
            project_root,
            output_path: &output_path,
            snapshot: &snapshot,
            hash: &hash,
            names: &names,
            procedures: &procedures,
        };
        {
            let _spinner = output
                .map(|out| out.status_spinner(format!("generating {} -> {}", key, target.output)));
            write_language(*language, &input)?;
        }
        if let Some(out) = output {
            out.status(format!("generated {} -> {}", key, target.output));
        }
    }

    Ok(())
}

fn write_language(language: LanguageTarget, input: &SchemaEmitInput<'_>) -> Result<()> {
    // One arm per language adapter. Procedure CALL clients come from `input.procedures`.
    match language {
        LanguageTarget::TypeScript => typescript::write_typescript(input),
        LanguageTarget::Dart => dart::write_dart_schema(input),
        LanguageTarget::Rust => rust::write_rust_schema(input),
    }
}

fn resolve_languages(
    config: &KalamProjectConfig,
    requested: Option<&Vec<String>>,
) -> Result<Vec<LanguageTarget>> {
    let languages = requested
        .map(|values| parse_language_list(values))
        .unwrap_or_else(|| parse_language_list(&config.schema.languages));
    if languages.is_empty() {
        return Err(CLIError::ConfigurationError(
            "no language targets selected for generation".into(),
        ));
    }
    Ok(languages)
}

pub fn validate_language_filter(
    requested: &[String],
    configured: &[String],
) -> Result<Vec<String>> {
    let mut normalized = Vec::new();
    for value in requested {
        let Some(parsed) = LanguageTarget::parse(value) else {
            return Err(CLIError::ConfigurationError(format!(
                "unsupported language target '{value}'; supported: typescript, dart, rust"
            )));
        };
        let canonical = parsed.as_str().to_string();
        if !configured.iter().any(|lang| LanguageTarget::parse(lang) == Some(parsed)) {
            return Err(CLIError::ConfigurationError(format!(
                "language '{canonical}' is not enabled in kalam.toml schema.languages"
            )));
        }
        if !normalized.iter().any(|lang| lang == &canonical) {
            normalized.push(canonical);
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_language_filter_accepts_flutter_alias() {
        let filtered =
            validate_language_filter(&["flutter".into()], &["dart".into()]).expect("filter");
        assert_eq!(filtered, vec!["dart"]);
    }

    #[test]
    fn validate_language_filter_accepts_rust() {
        let filtered =
            validate_language_filter(&["rust".into()], &["rust".into()]).expect("filter");
        assert_eq!(filtered, vec!["rust"]);
    }
}
