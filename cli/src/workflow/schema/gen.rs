//! Schema generation handlers.

use std::{path::Path, process::Command};

use kalam_client::credentials::CredentialStore;
use url::Url;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        dev::server::local_server_root_password,
        project::resolve::{resolve_kalam_profile, ResolvedEnvironment},
        schema::model::{parse_language_list, LanguageTarget},
        WorkflowContext,
    },
    FileCredentialStore,
};

const ORM_CODEGEN_SCRIPT: &str = include_str!("../../../templates/typescript/orm-codegen.mjs");

pub struct GenerateOptions {
    pub languages: Option<Vec<String>>,
}

pub fn generate_schema_artifacts(
    ctx: &WorkflowContext,
    options: &GenerateOptions,
    output: &WorkflowOutput,
) -> Result<()> {
    let languages = options
        .languages
        .as_ref()
        .map(|values| parse_language_list(values))
        .unwrap_or_else(|| parse_language_list(&ctx.config.schema.languages));

    if languages.is_empty() {
        return Err(CLIError::ConfigurationError(
            "no language targets selected for generation".into(),
        ));
    }

    let mut resolved_environment: Option<ResolvedEnvironment> = None;
    for language in languages {
        let key = language.as_str();
        let Some(target) = ctx.config.schema.targets.get(key) else {
            return Err(CLIError::ConfigurationError(format!(
                "missing schema.targets.{key} in kalam.toml"
            )));
        };

        let output_path = ctx.project_root.join(&target.output);
        match language {
            LanguageTarget::TypeScript => {
                if resolved_environment.is_none() {
                    resolved_environment = Some(ctx.resolved_environment()?);
                }
                let environment =
                    resolved_environment.as_ref().expect("resolved environment initialized");
                generate_typescript_via_orm(ctx, environment, &output_path)?;
            },
            LanguageTarget::Dart => write_dart_placeholder(&output_path)?,
        }
        output.status(format!("generated {} -> {}", key, target.output));
    }

    Ok(())
}

fn generate_typescript_via_orm(
    ctx: &WorkflowContext,
    environment: &ResolvedEnvironment,
    output_path: &Path,
) -> Result<()> {
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let auth = resolve_typescript_codegen_auth(ctx, environment)?;
    let mut command = Command::new(resolve_node_binary());
    command
        .current_dir(&ctx.project_root)
        .args(node_codegen_args())
        .arg("-e")
        .arg(ORM_CODEGEN_SCRIPT)
        .env("KALAM_SCHEMA_URL", &environment.url)
        .env("KALAM_SCHEMA_NAMESPACE", &environment.namespace)
        .env("KALAM_SCHEMA_OUT", output_path);

    match auth {
        OrmCodegenAuth::Basic { user, password } => {
            command
                .env("KALAM_SCHEMA_AUTH_MODE", "basic")
                .env("KALAM_SCHEMA_USER", user)
                .env("KALAM_SCHEMA_PASSWORD", password);
        },
        OrmCodegenAuth::Jwt { token } => {
            command.env("KALAM_SCHEMA_AUTH_MODE", "jwt").env("KALAM_SCHEMA_TOKEN", token);
        },
        OrmCodegenAuth::None => {
            command.env("KALAM_SCHEMA_AUTH_MODE", "none");
        },
    }

    let output = command.output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            CLIError::ConfigurationError(
                "Node.js is required for TypeScript schema generation via @kalamdb/orm".into(),
            )
        } else {
            CLIError::FileError(format!("failed to launch @kalamdb/orm generator: {error}"))
        }
    })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stderr.contains("Cannot find package '@kalamdb/orm'")
        || stderr.contains("Cannot find module '@kalamdb/orm'")
    {
        return Err(CLIError::ConfigurationError(
            "TypeScript schema generation requires @kalamdb/orm to be installed in the project"
                .into(),
        ));
    }
    if stderr.contains("Cannot find package '@kalamdb/client'")
        || stderr.contains("Cannot find module '@kalamdb/client'")
    {
        return Err(CLIError::ConfigurationError(
            "TypeScript schema generation requires @kalamdb/client to be installed in the project"
                .into(),
        ));
    }

    let detail = format!(
        "TypeScript schema generation via @kalamdb/orm failed.\nstdout:\n{}\nstderr:\n{}",
        stdout.trim(),
        stderr.trim()
    );
    Err(CLIError::ConfigurationError(detail))
}

fn write_dart_placeholder(output_path: &Path) -> Result<()> {
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        output_path,
        "// Placeholder generated by kalam schema gen.\n// Dart schema generation via a dedicated package will be added later.\n",
    )
    .map_err(|error| {
        CLIError::FileError(format!("failed to write '{}': {error}", output_path.display()))
    })?;
    Ok(())
}

fn resolve_node_binary() -> &'static str {
    "node"
}

fn node_codegen_args() -> &'static [&'static str] {
    &["--preserve-symlinks", "--input-type=module"]
}

enum OrmCodegenAuth {
    Basic { user: String, password: String },
    Jwt { token: String },
    None,
}

fn resolve_typescript_codegen_auth(
    ctx: &WorkflowContext,
    environment: &ResolvedEnvironment,
) -> Result<OrmCodegenAuth> {
    if let Some(token) = load_project_profile_token(ctx)? {
        return Ok(OrmCodegenAuth::Jwt { token });
    }

    if ctx.config.dev.auto_start_db {
        if let Some(password) = local_server_root_password(&ctx.project_root, &ctx.config)? {
            return Ok(OrmCodegenAuth::Basic {
                user: "root".into(),
                password,
            });
        }
    }

    if ctx.config.dev.auto_start_db && is_loopback_server_url(&environment.url) {
        return Ok(OrmCodegenAuth::Basic {
            user: "root".into(),
            password: "mypass".into(),
        });
    }

    if let Some(token) = ctx.cli_config.auth.as_ref().and_then(|auth| auth.jwt_token.clone()) {
        return Ok(OrmCodegenAuth::Jwt { token });
    }

    let store = FileCredentialStore::new().map_err(|error| {
        CLIError::ConfigurationError(format!("failed to open credentials store: {error}"))
    })?;
    if let Some(credentials) =
        store.get_workflow_env_credentials(&environment.name).map_err(|error| {
            CLIError::ConfigurationError(format!("failed to load workflow credentials: {error}"))
        })?
    {
        return Ok(OrmCodegenAuth::Jwt {
            token: credentials.jwt_token,
        });
    }

    Ok(OrmCodegenAuth::None)
}

fn load_project_profile_token(ctx: &WorkflowContext) -> Result<Option<String>> {
    let Some(profile) = resolve_kalam_profile(&ctx.project_root)? else {
        return Ok(None);
    };

    let store = FileCredentialStore::new().map_err(|error| {
        CLIError::ConfigurationError(format!("failed to open credentials store: {error}"))
    })?;
    let credentials = store.get_credentials(&profile).map_err(|error| {
        CLIError::ConfigurationError(format!(
            "failed to load credentials for profile '{profile}': {error}"
        ))
    })?;

    let credentials = credentials.ok_or_else(|| {
        CLIError::ConfigurationError(format!(
            "profile '{profile}' from .env was not found in ~/.kalam credentials; run `kalam login --instance {profile}`"
        ))
    })?;

    Ok(Some(credentials.jwt_token))
}

fn is_loopback_server_url(server_url: &str) -> bool {
    Url::parse(server_url)
        .ok()
        .and_then(|url| {
            url.host_str().map(|host| {
                host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
            })
        })
        .unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_codegen_args_preserve_project_symlink_resolution() {
        assert!(node_codegen_args().contains(&"--preserve-symlinks"));
    }
}
