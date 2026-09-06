use super::{InitOptions, ServerMode};
use crate::{
    error::{CLIError, Result},
    terminal_ui::SelectOption,
    workflow::{
        dev::server::DEFAULT_DEV_SERVER_URL,
        project::{
            config::SchemaMode,
            connection_url::{validate_http_server_url, validate_loopback_server_url},
            guidance::{
                init_empty_project_name, init_invalid_server_url, init_missing_language_targets,
                init_remote_schema_unavailable, init_requires_non_interactive_flags,
                init_unsupported_language,
            },
            prompts::{
                echo_prompt_selection, interactive_available, prompt_multi_select, prompt_select,
                prompt_text, prompt_text_with_default,
            },
            scaffold_input::validate_project_display_name,
        },
    },
};

pub(super) fn ensure_interactive_or_yes(options: &InitOptions) -> Result<()> {
    if options.yes || interactive_available() {
        return Ok(());
    }
    Err(CLIError::ConfigurationError(init_requires_non_interactive_flags()))
}

pub(super) fn resolve_project_name(options: &InitOptions, color: bool) -> Result<String> {
    if let Some(name) = options.name.as_ref().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        return validate_project_display_name(name);
    }
    if options.yes {
        let fallback =
            options.cwd.file_name().and_then(|n| n.to_str()).unwrap_or("my-app").to_string();
        return validate_project_display_name(&fallback);
    }

    prompt_text("Project name:", color).and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            Err(CLIError::ConfigurationError(init_empty_project_name()))
        } else {
            validate_project_display_name(trimmed)
        }
    })
}

pub(super) fn resolve_schema_mode(options: &InitOptions, color: bool) -> Result<SchemaMode> {
    if let Some(mode) = options.schema_mode {
        if matches!(mode, SchemaMode::Remote) {
            return Err(CLIError::ConfigurationError(init_remote_schema_unavailable()));
        }
        return Ok(mode);
    }
    if options.yes {
        return Ok(SchemaMode::Sql);
    }

    let options_list = [
        SelectOption::described("SQL file", "Use schema.sql as the source of truth"),
        SelectOption::disabled("Remote schema", "Coming soon"),
    ];
    let selected = prompt_select("Schema mode", &options_list, 0, color)?;
    let mode = SchemaMode::Sql;
    if selected != 0 {
        return Err(CLIError::ConfigurationError(init_remote_schema_unavailable()));
    }
    echo_prompt_selection("Schema mode:", schema_mode_label(mode), color);
    Ok(mode)
}

pub(super) fn resolve_languages(options: &InitOptions, color: bool) -> Result<Vec<String>> {
    if let Some(languages) = &options.languages {
        return Ok(normalize_languages(languages)?);
    }
    if options.yes {
        return Ok(vec!["typescript".into()]);
    }

    let options_list = [
        SelectOption::described("TypeScript", "Generate src/generated/kalam.ts"),
        SelectOption::described(
            "Dart / Flutter",
            "Generate lib/generated/kalam.dart and scaffold a Flutter starter",
        ),
    ];
    let selected =
        prompt_multi_select("Generated language targets", &options_list, &[true, false], color)?;

    let mut languages = Vec::new();
    for idx in selected {
        match idx {
            0 => languages.push("typescript".into()),
            1 => languages.push("dart".into()),
            _ => {},
        }
    }
    let languages = normalize_languages(&languages)?;
    echo_prompt_selection("Languages:", &languages.join(", "), color);
    Ok(languages)
}

pub(super) fn resolve_server_mode(options: &InitOptions, color: bool) -> Result<ServerMode> {
    if let Some(mode) = options.server_mode {
        return Ok(mode);
    }
    if options.yes {
        return Ok(ServerMode::Local);
    }

    let options_list = [
        SelectOption::described("Local", "Start or reuse KalamDB with kalam dev"),
        SelectOption::described("Remote", "Connect to an existing KalamDB server"),
    ];
    let selected = prompt_select("Server mode", &options_list, 0, color)?;
    let mode = match selected {
        0 => ServerMode::Local,
        _ => ServerMode::Remote,
    };
    echo_prompt_selection("Server mode:", server_mode_label(mode), color);
    Ok(mode)
}

pub(super) fn resolve_server_url(
    options: &InitOptions,
    server_mode: ServerMode,
    color: bool,
) -> Result<String> {
    if let Some(url) = options.server_url.as_ref().map(|u| u.trim()).filter(|u| !u.is_empty()) {
        return validate_init_server_url(url, server_mode);
    }

    match server_mode {
        ServerMode::Local => validate_loopback_server_url(DEFAULT_DEV_SERVER_URL),
        ServerMode::Remote if options.yes => validate_http_server_url(DEFAULT_DEV_SERVER_URL),
        ServerMode::Remote => {
            let url = prompt_text_with_default("Server URL", DEFAULT_DEV_SERVER_URL, color)?;
            validate_init_server_url(&url, server_mode)
        },
    }
}

fn validate_init_server_url(value: &str, server_mode: ServerMode) -> Result<String> {
    match server_mode {
        ServerMode::Local => validate_loopback_server_url(value),
        ServerMode::Remote => validate_http_server_url(value),
    }
    .map_err(|error| {
        CLIError::ConfigurationError(init_invalid_server_url(value, &error.to_string()))
    })
}

fn normalize_languages(languages: &[String]) -> Result<Vec<String>> {
    if languages.is_empty() {
        return Err(CLIError::ConfigurationError(init_missing_language_targets()));
    }

    let mut normalized = Vec::new();
    for language in languages {
        match language.trim().to_ascii_lowercase().as_str() {
            "typescript" | "ts" => normalized.push("typescript".into()),
            "dart" | "flutter" => normalized.push("dart".into()),
            other => {
                return Err(CLIError::ConfigurationError(init_unsupported_language(other)));
            },
        }
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

pub(super) fn schema_mode_label(mode: SchemaMode) -> &'static str {
    match mode {
        SchemaMode::Sql => "sql",
        SchemaMode::Remote => "remote",
    }
}

fn server_mode_label(mode: ServerMode) -> &'static str {
    match mode {
        ServerMode::Local => "local",
        ServerMode::Remote => "remote",
    }
}
