//! Embedded project templates compiled into the CLI binary.

include!(concat!(env!("OUT_DIR"), "/embedded_templates.rs"));

use handlebars::Handlebars;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::error::{CLIError, Result};

pub const DEFAULT_SCAFFOLD_TEMPLATE: &str = "default";

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

pub fn resolve_scaffold_template() -> Result<&'static EmbeddedTemplate> {
    find_template("scaffold", DEFAULT_SCAFFOLD_TEMPLATE).ok_or_else(|| {
        CLIError::ConfigurationError(format!(
            "missing built-in scaffold template '{DEFAULT_SCAFFOLD_TEMPLATE}'"
        ))
    })
}

pub fn find_template_file(template: &EmbeddedTemplate, project_path: &str) -> Option<&'static str> {
    template
        .files
        .iter()
        .find(|file| file.project_path == project_path)
        .map(|file| file.content)
}

pub fn render_template(template: &str, context: &impl Serialize) -> Result<String> {
    let mut handlebars = Handlebars::new();
    handlebars.register_escape_fn(handlebars::no_escape);
    handlebars.render_template(template, context).map_err(|error| {
        CLIError::ConfigurationError(format!("failed to render template: {error}"))
    })
}

pub fn render_template_pairs(template: &str, replacements: &[(&str, &str)]) -> Result<String> {
    let mut context = Map::new();
    for (key, value) in replacements {
        context.insert(key.to_string(), Value::String((*value).to_string()));
    }
    render_template(template, &context)
}

pub fn find_scaffold_template_file(project_path: &str) -> Result<&'static str> {
    let template = resolve_scaffold_template()?;
    find_template_file(template, project_path).ok_or_else(|| {
        CLIError::ConfigurationError(format!(
            "missing scaffold template file '{project_path}' in '{DEFAULT_SCAFFOLD_TEMPLATE}'"
        ))
    })
}

pub fn format_languages_array(languages: &[impl AsRef<str>]) -> String {
    let quoted: Vec<String> = languages
        .iter()
        .map(|language| format!("\"{}\"", language.as_ref()))
        .collect();
    format!("[{}]", quoted.join(", "))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemplateStringEscape {
    None,
    DoubleQuoted,
    JsSingleQuoted,
}

fn template_string_escape_for_path(project_path: &str) -> TemplateStringEscape {
    if project_path.ends_with(".json") {
        return TemplateStringEscape::DoubleQuoted;
    }
    if project_path.ends_with(".ts") || project_path.ends_with(".tsx") {
        return TemplateStringEscape::JsSingleQuoted;
    }
    TemplateStringEscape::None
}

pub fn render_template_pairs_for_path(
    project_path: &str,
    template: &str,
    replacements: &[(&str, &str)],
) -> Result<String> {
    match template_string_escape_for_path(project_path) {
        TemplateStringEscape::None => render_template_pairs(template, replacements),
        TemplateStringEscape::DoubleQuoted => {
            render_template_pairs_escaped(template, replacements, escape_double_quoted_string)
        },
        TemplateStringEscape::JsSingleQuoted => {
            render_template_pairs_escaped(template, replacements, escape_js_single_quoted_string)
        },
    }
}

pub struct KalamTomlScaffoldInput<'a> {
    pub project_name: &'a str,
    pub namespace: &'a str,
    pub server_url: &'a str,
    pub schema_mode: &'a str,
    pub schema_path: &'a str,
    pub languages: &'a [String],
    pub auto_start_db: bool,
    pub package_manager: Option<&'a str>,
    pub dev_process_command: &'a str,
}

pub fn render_kalam_toml_scaffold(
    template: &str,
    input: &KalamTomlScaffoldInput<'_>,
) -> Result<String> {
    let language_names: Vec<&str> = input.languages.iter().map(String::as_str).collect();
    render_template(
        template,
        &serde_json::json!({
            "project_name": escape_double_quoted_string(input.project_name),
            "namespace": escape_double_quoted_string(input.namespace),
            "server_url": escape_double_quoted_string(input.server_url),
            "schema_mode": input.schema_mode,
            "schema_path": input.schema_path,
            "languages_array": format_languages_array(&language_names),
            "typescript": input.languages.iter().any(|language| language == "typescript"),
            "dart": input.languages.iter().any(|language| language == "dart"),
            "auto_start_db": input.auto_start_db,
            "package_manager": input.package_manager.unwrap_or(""),
            "dev_process_command": escape_double_quoted_string(input.dev_process_command),
        }),
    )
}

fn render_template_pairs_escaped(
    template: &str,
    replacements: &[(&str, &str)],
    escape: fn(&str) -> String,
) -> Result<String> {
    let escaped: Vec<(String, String)> = replacements
        .iter()
        .map(|(key, value)| ((*key).to_string(), escape(value)))
        .collect();
    let borrowed: Vec<(&str, &str)> = escaped
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    render_template_pairs(template, &borrowed)
}

fn escape_common_string(value: &str, quote: char) -> String {
    let mut escaped = value.replace('\\', "\\\\");
    if quote == '"' {
        escaped = escaped.replace('"', "\\\"");
    } else if quote == '\'' {
        escaped = escaped.replace('\'', "\\'");
    }
    escaped
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Escape a value for use inside a TOML or JSON double-quoted string.
pub fn escape_double_quoted_string(value: &str) -> String {
    escape_common_string(value, '"')
}

/// Escape a value for use inside a TOML basic string (`"..."`).
pub fn escape_toml_basic_string(value: &str) -> String {
    escape_double_quoted_string(value)
}

/// Escape a value for use inside a JSON double-quoted string.
pub fn escape_json_string(value: &str) -> String {
    escape_double_quoted_string(value)
}

/// Escape a value for use inside a JavaScript single-quoted string literal.
pub fn escape_js_single_quoted_string(value: &str) -> String {
    escape_common_string(value, '\'')
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

    #[test]
    fn embedded_templates_include_scaffold_defaults() {
        let template = resolve_scaffold_template().expect("scaffold template");
        assert!(find_template_file(template, "kalam.toml").is_some());
        assert!(find_template_file(template, "kalam/server/server.toml").is_some());
        assert!(find_template_file(template, ".env.example").is_some());
    }

    #[test]
    fn render_template_replaces_placeholders() {
        let rendered = render_template_pairs("hello {{name}}", &[("name", "world")]).unwrap();
        assert_eq!(rendered, "hello world");
    }

    #[test]
    fn escape_toml_basic_string_quotes_special_characters() {
        assert_eq!(
            escape_toml_basic_string("my\"app\n"),
            "my\\\"app\\n"
        );
    }

    #[test]
    fn escape_js_single_quoted_string_quotes_special_characters() {
        assert_eq!(
            escape_js_single_quoted_string("http://x'; alert(1);//"),
            "http://x\\'; alert(1);//"
        );
    }

    #[test]
    fn render_template_pairs_for_js_keeps_generated_code_safe() {
        let rendered = render_template_pairs_for_path(
            "src/index.ts",
            "const url = '{{server_url}}';",
            &[("server_url", "http://x'; alert(1);//")],
        )
        .unwrap();
        assert_eq!(rendered, "const url = 'http://x\\'; alert(1);//';");
    }

    #[test]
    fn render_template_supports_if_blocks() {
        let rendered = render_template(
            "{{#if typescript}}enabled{{else}}disabled{{/if}}",
            &serde_json::json!({ "typescript": true }),
        )
        .unwrap();
        assert_eq!(rendered, "enabled");

        let rendered = render_template(
            "{{#if typescript}}enabled{{else}}disabled{{/if}}",
            &serde_json::json!({ "typescript": false }),
        )
        .unwrap();
        assert_eq!(rendered, "disabled");
    }

    #[test]
    fn scaffold_kalam_toml_renders_language_targets_conditionally() {
        use crate::workflow::project::config::KalamProjectConfig;

        let template = resolve_scaffold_template().expect("scaffold template");
        let kalam_toml = find_template_file(template, "kalam.toml").expect("kalam.toml template");

        let typescript_only = render_scaffold_kalam_toml(
            kalam_toml,
            "demo-ts",
            "http://localhost:2900",
            true,
            false,
            true,
            Some("npm run dev"),
        );
        assert!(typescript_only.contains("[schema.targets.typescript]"));
        assert!(!typescript_only.contains("[schema.targets.dart]"));
        assert!(!typescript_only.contains("&quot;"));
        assert!(typescript_only.contains("[dev.processes]"));
        assert!(typescript_only.contains("app = \"npm run dev\""));
        assert!(typescript_only.contains("kalam dev"));
        let parsed =
            KalamProjectConfig::parse(&typescript_only).expect("parse typescript kalam.toml");
        assert!(parsed.schema.targets.contains_key("typescript"));
        assert!(!parsed.schema.targets.contains_key("dart"));
        assert_eq!(parsed.dev.processes.get("app").map(String::as_str), Some("npm run dev"));

        let dart_only = render_scaffold_kalam_toml(
            kalam_toml,
            "demo-dart",
            "http://localhost:2900",
            false,
            true,
            true,
            None,
        );
        assert!(!dart_only.contains("[schema.targets.typescript]"));
        assert!(dart_only.contains("[schema.targets.dart]"));
        assert!(dart_only.contains("# [dev.processes]"));
        assert!(dart_only.contains("# app = \"npm run dev\""));
        assert!(!dart_only.contains("\n[dev.processes]\n"));
        let parsed = KalamProjectConfig::parse(&dart_only).expect("parse dart kalam.toml");
        assert!(!parsed.schema.targets.contains_key("typescript"));
        assert!(parsed.schema.targets.contains_key("dart"));
        assert!(parsed.dev.processes.is_empty());

        let both = render_scaffold_kalam_toml(
            kalam_toml,
            "demo-both",
            "http://localhost:2900",
            true,
            true,
            true,
            Some("pnpm dev"),
        );
        assert!(both.contains("[schema.targets.typescript]"));
        assert!(both.contains("[schema.targets.dart]"));
        assert!(both.contains("app = \"pnpm dev\""));
        let parsed = KalamProjectConfig::parse(&both).expect("parse dual-language kalam.toml");
        assert!(parsed.schema.targets.contains_key("typescript"));
        assert!(parsed.schema.targets.contains_key("dart"));
        assert_eq!(parsed.dev.processes.get("app").map(String::as_str), Some("pnpm dev"));
    }

    #[test]
    fn scaffold_gitignore_includes_language_sections_conditionally() {
        let template = resolve_scaffold_template().expect("scaffold template");
        let gitignore = find_template_file(template, ".gitignore").expect(".gitignore template");

        let typescript_only = render_template(
            gitignore,
            &serde_json::json!({
                "typescript": true,
                "dart": false,
            }),
        )
        .unwrap();
        assert!(typescript_only.contains("node_modules/"));
        assert!(!typescript_only.contains(".dart_tool/"));

        let dart_only = render_template(
            gitignore,
            &serde_json::json!({
                "typescript": false,
                "dart": true,
            }),
        )
        .unwrap();
        assert!(!dart_only.contains("node_modules/"));
        assert!(dart_only.contains(".dart_tool/"));
    }

    #[test]
    fn scaffold_server_toml_renders_port_paths_and_rate_limits() {
        let template = resolve_scaffold_template().expect("scaffold template");
        let server_toml =
            find_template_file(template, "kalam/server/server.toml").expect("server.toml template");

        let rendered = render_template(
            server_toml,
            &serde_json::json!({
                "port": 3001,
                "data_path": "kalam/server/data",
                "logs_path": "kalam/server/logs",
            }),
        )
        .unwrap();

        assert!(rendered.contains("port = 3001"));
        assert!(rendered.contains("public_origin = \"http://localhost:3001\""));
        assert!(rendered.contains("data_path = \"kalam/server/data\""));
        assert!(rendered.contains("logs_path = \"kalam/server/logs\""));
        assert!(rendered.contains("[rate_limit]"));
        assert!(rendered.contains("max_queries_per_sec = 100000"));
        assert!(!rendered.contains("&quot;"));
    }

    fn render_scaffold_kalam_toml(
        template: &str,
        project_name: &str,
        server_url: &str,
        typescript: bool,
        dart: bool,
        auto_start_db: bool,
        dev_process_command: Option<&str>,
    ) -> String {
        use crate::workflow::project::identifiers::normalize_namespace_name;

        let mut languages = Vec::new();
        if typescript {
            languages.push("typescript".to_string());
        }
        if dart {
            languages.push("dart".to_string());
        }

        render_kalam_toml_scaffold(
            template,
            &KalamTomlScaffoldInput {
                project_name,
                namespace: &normalize_namespace_name(project_name),
                server_url,
                schema_mode: "sql",
                schema_path: "schema.sql",
                languages: &languages,
                auto_start_db,
                package_manager: None,
                dev_process_command: dev_process_command.unwrap_or(""),
            },
        )
        .unwrap()
    }
}
