//! One inventory for managed local processes and saved cloud connections.

use std::collections::{HashMap, HashSet};

use kalam_client::credentials::CredentialStore;
use url::Url;

use super::{
    instance_kind::InstanceKind, instance_state::InstanceState, instance_summary::InstanceSummary,
    instances_options::InstancesOptions, servers::tracked_servers,
};
use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{instance, project::preferred_user_label},
    FileCredentialStore,
};

pub fn instance_catalog(
    output: &WorkflowOutput,
    store: &dyn CredentialStore,
) -> Result<Vec<InstanceSummary>> {
    let mut rows = Vec::new();
    for (entry, record) in tracked_servers(output)? {
        let global = record.as_ref().is_some_and(|r| r.kind == instance::TargetKind::SharedLocal);
        let name = if global {
            "global".to_owned()
        } else {
            entry
                .folder
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| "local-server".into())
        };
        let state = match &record {
            Some(record) if instance::instance_is_live(record) => InstanceState::Running,
            Some(_) => InstanceState::Stopped,
            None => InstanceState::Unavailable,
        };
        rows.push(InstanceSummary {
            name,
            kind: InstanceKind::Local,
            state,
            url: record.as_ref().map(|r| r.url.clone()),
            folder: Some(entry.folder),
            global,
            user: None,
            auth: None,
            aliases: Vec::new(),
            identity: entry.instance_path.to_string_lossy().into_owned(),
        });
    }
    merge_credentials(&mut rows, store)?;
    assign_unique_names(&mut rows);
    rows.sort_by(|a, b| a.kind.as_str().cmp(b.kind.as_str()).then(a.name.cmp(&b.name)));
    Ok(rows)
}

fn merge_credentials(rows: &mut Vec<InstanceSummary>, store: &dyn CredentialStore) -> Result<()> {
    let mut names = store.list_instances()?;
    names.sort();
    for name in names {
        let Some(creds) = store.get_credentials(&name)? else {
            continue;
        };
        let endpoint = creds
            .server_url
            .as_deref()
            .or(Some(name.as_str()))
            .and_then(connection_endpoint);
        let matches: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                (row.kind == InstanceKind::Local
                    && endpoint.as_deref().and_then(endpoint_key).is_some()
                    && row.url.as_deref().and_then(endpoint_key)
                        == endpoint.as_deref().and_then(endpoint_key))
                .then_some(index)
            })
            .collect();
        if let [index] = matches.as_slice() {
            rows[*index].aliases.push(name);
            continue;
        }
        let user = creds
            .user
            .as_ref()
            .map(|user| preferred_user_label(user, creds.name.as_deref(), creds.email.as_deref()))
            .or_else(|| creds.display_label().map(str::to_owned));
        rows.push(InstanceSummary {
            name: name.clone(),
            kind: InstanceKind::Cloud,
            state: InstanceState::NotChecked,
            url: endpoint,
            folder: None,
            global: false,
            user,
            auth: Some(
                if creds.is_expired() {
                    "Expired"
                } else {
                    "Signed in"
                }
                .into(),
            ),
            aliases: vec![name.clone()],
            identity: format!("credential:{name}"),
        });
    }
    Ok(())
}

fn connection_endpoint(value: &str) -> Option<String> {
    let mut url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    url.set_username("").ok()?;
    url.set_password(None).ok()?;
    url.set_query(None);
    url.set_fragment(None);
    Some(url.as_str().trim_end_matches('/').to_string())
}

pub(super) fn endpoint_key(value: &str) -> Option<String> {
    let mut url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    if matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]" | "::1")) {
        url.set_host(Some("localhost")).ok()?;
    }
    Some(url.as_str().trim_end_matches('/').to_string())
}

fn assign_unique_names(rows: &mut [InstanceSummary]) {
    let mut counts = HashMap::new();
    for row in rows.iter() {
        *counts.entry(row.name.clone()).or_insert(0usize) += 1;
    }
    let mut used: HashSet<String> = rows.iter().map(|row| row.name.clone()).collect();
    for row in rows.iter_mut() {
        // "local" is the historical CLI credential default, not an explicit selector.
        if counts[&row.name] > 1 || row.name == "local" {
            row.aliases.push(row.name.clone());
            let key = crate::release_download::sha256_bytes(row.identity.as_bytes());
            let base = format!("{}-{}", row.name, &key[..8]);
            let mut candidate = base.clone();
            let mut suffix = 2;
            while used.contains(&candidate) {
                candidate = format!("{base}-{suffix}");
                suffix += 1;
            }
            used.insert(candidate.clone());
            row.name = candidate;
        }
    }
}

pub fn resolve_instance_name(name: &str, output: &WorkflowOutput) -> Result<InstanceSummary> {
    let store = FileCredentialStore::new()?;
    select_instance(name, instance_catalog(output, &store)?)
}

fn select_instance(name: &str, rows: Vec<InstanceSummary>) -> Result<InstanceSummary> {
    let matches: Vec<_> = rows
        .into_iter()
        .filter(|row| row.name == name || row.aliases.iter().any(|alias| alias == name))
        .collect();
    match matches.len() {
        1 => Ok(matches.into_iter().next().expect("one matched instance")),
        0 => Err(CLIError::ConfigurationError(format!(
            "Unknown instance '{name}'. Run `kalam instances` to see available names."
        ))),
        _ => Err(CLIError::ConfigurationError(format!(
            "Instance name '{name}' is ambiguous. Use one of: {}",
            matches.iter().map(|row| row.name.as_str()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

pub async fn list_instances(
    options: InstancesOptions,
    output: &WorkflowOutput,
    store: &dyn CredentialStore,
) -> Result<()> {
    let spinner = output.status_spinner("Loading instances");
    let mut rows = instance_catalog(output, store)?;
    rows.retain(|row| {
        (!options.local || row.kind == InstanceKind::Local)
            && (!options.cloud || row.kind == InstanceKind::Cloud)
    });
    if options.check {
        check_cloud_instances(&mut rows).await?;
    }
    drop(spinner);
    render_instances(rows, output)
}

pub async fn show_cloud_instance_status(
    mut row: InstanceSummary,
    output: &WorkflowOutput,
) -> Result<()> {
    let spinner = output.status_spinner("Checking instance reachability");
    check_cloud_instances(std::slice::from_mut(&mut row)).await?;
    drop(spinner);
    render_instances(vec![row], output)
}

async fn check_cloud_instances(rows: &mut [InstanceSummary]) -> Result<()> {
    // Bound each probe and do not send saved credentials to health endpoints.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| CLIError::ConfigurationError(error.to_string()))?;
    for row in rows.iter_mut().filter(|row| row.kind == InstanceKind::Cloud) {
        row.state = match row.url.as_deref().filter(|url| endpoint_key(url).is_some()) {
            Some(url) => {
                match client.get(format!("{}/health", url.trim_end_matches('/'))).send().await {
                    Ok(_) => InstanceState::Reachable,
                    Err(_) => InstanceState::Unreachable,
                }
            },
            None => InstanceState::Unavailable,
        };
    }
    Ok(())
}

fn render_instances(rows: Vec<InstanceSummary>, output: &WorkflowOutput) -> Result<()> {
    if output.json {
        output.emit_json(&serde_json::json!({"ok": true, "instances": rows}));
        return Ok(());
    }
    output.agent_event("KALAM_INSTANCES", &[("count", &rows.len().to_string())]);
    if rows.is_empty() {
        output.status("No instances found");
        output.detail(
            "Start a local server with `kalam up`, or save a cloud connection with `kalam login`.",
        );
        return Ok(());
    }
    let local = rows.iter().filter(|row| row.kind == InstanceKind::Local).count();
    output.status(format!("KalamDB instances · {} cloud · {local} local", rows.len() - local));
    let width = rows.iter().map(|row| row.name.chars().count()).max().unwrap_or(4).max(4);
    output.listing_line(format!("{:<width$}  {:<5}  {:<11}  URL", "NAME", "TYPE", "STATUS"));
    for row in rows {
        output.instance_row(
            &row.name,
            row.kind.as_str(),
            row.state.label(),
            row.url.as_deref().unwrap_or("not configured"),
            width,
        );
        if let Some(folder) = row.folder {
            output.detail(format!("  Folder  {}", folder.display()));
            if row.global {
                output.detail("  Scope   Global");
            }
        } else {
            output.detail(format!("  User    {}", row.user.as_deref().unwrap_or("unknown")));
            if row.auth.as_deref() == Some("Expired") {
                output.warn("  Auth    Expired");
            } else {
                output.detail(format!("  Auth    {}", row.auth.as_deref().unwrap_or("Unknown")));
            }
        }
        let aliases = row
            .aliases
            .iter()
            .filter(|alias| *alias != &row.name)
            .cloned()
            .collect::<Vec<_>>();
        if !aliases.is_empty() {
            output.detail(format!("  Also    {}", aliases.join(", ")));
        }
        output.agent_event(
            "KALAM_INSTANCE",
            &[
                ("name", &row.name),
                ("type", row.kind.as_str()),
                ("state", row.state.label()),
                ("url", row.url.as_deref().unwrap_or("")),
            ],
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use kalam_client::credentials::{Credentials, MemoryCredentialStore};

    use super::*;

    fn local(name: &str, folder: &str) -> InstanceSummary {
        InstanceSummary {
            name:     name.into(),
            kind:     InstanceKind::Local,
            state:    InstanceState::Running,
            url:      Some("http://127.0.0.1:2900".into()),
            folder:   Some(folder.into()),
            global:   false,
            user:     None,
            auth:     None,
            aliases:  vec![],
            identity: folder.into(),
        }
    }

    fn cloud(name: &str, url: &str) -> InstanceSummary {
        InstanceSummary {
            name:     name.into(),
            kind:     InstanceKind::Cloud,
            state:    InstanceState::NotChecked,
            url:      Some(url.into()),
            folder:   None,
            global:   false,
            user:     Some("user".into()),
            auth:     Some("Signed in".into()),
            aliases:  vec![],
            identity: format!("credential:{name}"),
        }
    }

    #[test]
    fn merges_local_credentials_without_auth_or_tokens_in_output() {
        let mut store = MemoryCredentialStore::new();
        let mut creds = Credentials::new("saved-local".into(), "never-display-this-token".into());
        creds.server_url = Some("http://localhost:2900/".into());
        store.set_credentials(&creds).unwrap();
        let mut rows = vec![local("analytics", "/analytics")];
        merge_credentials(&mut rows, &store).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].auth.is_none());
        assert!(rows[0].user.is_none());
        assert_eq!(select_instance("saved-local", rows.clone()).unwrap().name, "analytics");
        assert!(!serde_json::to_string(&rows).unwrap().contains("never-display"));
    }

    #[test]
    fn duplicate_names_are_distinct_and_ambiguous_names_are_rejected() {
        let mut rows = vec![
            local("analytics", "/one/analytics"),
            local("analytics", "/two/analytics"),
        ];
        assign_unique_names(&mut rows);
        assert_ne!(rows[0].name, rows[1].name);
        assert!(select_instance("analytics", rows.clone()).is_err());
        assert_eq!(select_instance(&rows[0].name, rows.clone()).unwrap().folder, rows[0].folder);
    }

    #[test]
    fn historical_local_credential_stays_selectable_as_alias() {
        let mut rows = vec![cloud("local", "https://example.com")];
        assign_unique_names(&mut rows);
        assert_ne!(rows[0].name, "local");
        assert!(rows[0].aliases.iter().any(|alias| alias == "local"));
        assert_eq!(
            select_instance("local", rows.clone()).unwrap().url.as_deref(),
            Some("https://example.com")
        );
    }

    #[test]
    fn cloud_auth_expiry_is_separate_from_reachability() {
        let mut store = MemoryCredentialStore::new();
        let creds = Credentials::with_details(
            "production".into(),
            "never-display-this-token".into(),
            "alice",
            "2000-01-01T00:00:00Z".into(),
            Some("https://example.com".into()),
        );
        store.set_credentials(&creds).unwrap();
        let mut rows = Vec::new();
        merge_credentials(&mut rows, &store).unwrap();
        assert_eq!(rows[0].kind, InstanceKind::Cloud);
        assert_eq!(rows[0].state, InstanceState::NotChecked);
        assert_eq!(rows[0].auth.as_deref(), Some("Expired"));
        assert!(!serde_json::to_string(&rows).unwrap().contains("never-display"));
    }
}
