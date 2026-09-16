use std::{fs, time::Duration};

use kalam_client::AuthProvider;
use serde_json::Value;

use crate::{
    error::{CLIError, Result},
    workflow::{
        auth::{login_with_credentials, resolve_workflow_auth_provider},
        WorkflowContext,
    },
};

pub async fn activate_function_module(ctx: &WorkflowContext) -> Result<()> {
    let artifact_path = ctx.project_root.join("functions/.kalam/build/module.js");
    if !artifact_path.is_file() {
        return Ok(());
    }
    let manifest_path = ctx.project_root.join("functions/.kalam/build/manifest.json");
    let manifest: Value = if manifest_path.is_file() {
        serde_json::from_str(&fs::read_to_string(&manifest_path).map_err(|error| {
            CLIError::FileError(format!("failed to read '{}': {error}", manifest_path.display()))
        })?)
        .map_err(|error| {
            CLIError::ConfigurationError(format!("invalid functions manifest: {error}"))
        })?
    } else {
        serde_json::json!({})
    };
    let artifact = fs::read_to_string(&artifact_path).map_err(|error| {
        CLIError::FileError(format!("failed to read '{}': {error}", artifact_path.display()))
    })?;
    let module = manifest
        .get("module")
        .and_then(Value::as_str)
        .unwrap_or(ctx.config.functions.module.as_str());
    let contract_hash = manifest
        .get("contractHash")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let abi_version = manifest.get("abiVersion").and_then(Value::as_u64).unwrap_or(2) as u32;
    let mut exports = Vec::new();
    if let Some(Value::Object(procedures)) = manifest.get("procedures") {
        for (name, kind) in procedures {
            if kind.as_str() == Some("module") {
                exports.push(name.clone());
            }
        }
    }
    let env = ctx.resolved_environment()?;
    let token = workflow_bearer_token(ctx, &env).await?;
    let url =
        format!("{}/v1/api/functions/modules/{module}/activate", env.url.trim_end_matches('/'));
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(ctx.cli_config.resolved_server().timeout))
        .build()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("failed to create HTTP client: {error}"))
        })?
        .post(url)
        .bearer_auth(token)
        .json(&serde_json::json!({
            "artifact": artifact,
            "contractHash": contract_hash,
            "abiVersion": abi_version,
            "exports": exports,
        }))
        .send()
        .await
        .map_err(|error| {
            CLIError::ConfigurationError(format!("function activate failed: {error}"))
        })?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(CLIError::ConfigurationError(format!(
            "function activate failed ({status}): {body}"
        )));
    }
    ctx.output().status(format!("activated function module {module}"));
    Ok(())
}

pub async fn rollback_function(ctx: &WorkflowContext, revision: &str) -> Result<()> {
    if revision.trim().is_empty() {
        return Err(CLIError::ConfigurationError(
            "function rollback requires a revision id".into(),
        ));
    }
    let output = ctx.output();
    let env = ctx.resolved_environment()?;
    let module = ctx.config.functions.module.as_str();
    output.status(format!("CAS rolling back function module {module} to revision {revision}"));
    let token = workflow_bearer_token(ctx, &env).await?;
    let url =
        format!("{}/v1/api/functions/modules/{module}/rollback", env.url.trim_end_matches('/'));
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(ctx.cli_config.resolved_server().timeout))
        .build()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("failed to create HTTP client: {error}"))
        })?
        .post(url)
        .bearer_auth(token)
        .json(&serde_json::json!({ "revisionId": revision }))
        .send()
        .await
        .map_err(|error| {
            CLIError::ConfigurationError(format!("function rollback failed: {error}"))
        })?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(CLIError::ConfigurationError(format!(
            "function rollback failed ({status}): {body}"
        )));
    }
    output.status(format!("rolled back to revision {revision} (pointer swap, no rebuild)"));
    Ok(())
}

async fn workflow_bearer_token(
    ctx: &WorkflowContext,
    env: &crate::workflow::project::resolve::ResolvedEnvironment,
) -> Result<String> {
    match resolve_workflow_auth_provider(ctx, env)? {
        AuthProvider::JwtToken(token) => Ok(token),
        AuthProvider::BasicAuth(user, password) => {
            let login =
                login_with_credentials(&env.url, &user, &password).await.map_err(|error| {
                    CLIError::ConfigurationError(format!("function module auth failed: {error}"))
                })?;
            Ok(login.access_token)
        },
        AuthProvider::None => Err(CLIError::ConfigurationError(
            "function activate/rollback requires a saved CLI authentication profile".into(),
        )),
    }
}
