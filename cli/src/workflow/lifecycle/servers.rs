//! User-wide discovery of CLI-managed servers without scanning user folders.

use std::{collections::BTreeMap, fs, path::Path};

use super::tracked_server::TrackedServer;
use crate::{
    error::{CLIError, Result},
    fs_atomic::{read_to_string_if_exists, write_atomic, FileReadPolicy, FileWriteOptions},
    history::get_kalam_config_dir,
    output::WorkflowOutput,
    workflow::{
        instance::{self, InstanceRecord, ManagedLayout},
        target::{resolve_lifecycle_target, TargetSelector},
    },
};

pub(crate) fn track_server(layout: &ManagedLayout, output: &WorkflowOutput) {
    if let Err(error) = register(&get_kalam_config_dir().join("server-registry"), layout) {
        output.warn(format!("Could not add this server to `kalam servers`: {error}"));
    }
}

fn registry_json_error(error: serde_json::Error) -> CLIError {
    CLIError::FileError(format!("Invalid server registry data: {error}"))
}

fn register(registry: &Path, layout: &ManagedLayout) -> Result<()> {
    let entry = TrackedServer {
        instance_path: fs::canonicalize(&layout.instance_path)?,
        folder:        fs::canonicalize(&layout.working_dir)?,
    };
    let payload = serde_json::to_vec(&entry).map_err(registry_json_error)?;
    let key = crate::release_download::sha256_bytes(
        &serde_json::to_vec(&entry.instance_path).map_err(registry_json_error)?,
    );
    write_atomic(&registry.join(format!("{key}.json")), &payload, FileWriteOptions::SECRET_FILE)?;
    Ok(())
}

fn read_record(entry: &TrackedServer) -> Result<Option<InstanceRecord>> {
    read_to_string_if_exists(&entry.instance_path, FileReadPolicy::UserProvided)?
        .map(|contents| serde_json::from_str(&contents).map_err(registry_json_error))
        .transpose()
}

fn read_registry(registry: &Path, output: &WorkflowOutput) -> Result<Vec<TrackedServer>> {
    let entries = match fs::read_dir(registry) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut servers = BTreeMap::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let parsed = (|| -> Result<TrackedServer> {
            let contents = crate::fs_atomic::read_to_string(&path, FileReadPolicy::LocalSecrets)?;
            Ok(serde_json::from_str(&contents).map_err(registry_json_error)?)
        })();
        match parsed {
            Ok(entry) => {
                servers.insert(entry.instance_path.clone(), entry);
            },
            Err(error) => output
                .warn(format!("Skipping unreadable registry entry '{}': {error}", path.display())),
        }
    }
    Ok(servers.into_values().collect())
}

pub(super) fn tracked_servers(
    output: &WorkflowOutput,
) -> Result<Vec<(TrackedServer, Option<InstanceRecord>)>> {
    // Import pre-registry instances we can locate without a filesystem scan.
    let global = instance::shared_layout();
    if global.instance_path.is_file() {
        track_server(&global, output);
    }
    if let Ok(target) = resolve_lifecycle_target(&TargetSelector::new(std::env::current_dir()?)) {
        if let Some(layout) = target.layout {
            if layout.instance_path.is_file() {
                track_server(&layout, output);
            }
        }
    }
    let entries = read_registry(&get_kalam_config_dir().join("server-registry"), output)?;
    Ok(entries
        .into_iter()
        .map(|entry| {
            let record = match read_record(&entry) {
                Ok(record) => record,
                Err(error) => {
                    output.warn(format!(
                        "Could not read server in '{}': {error}",
                        entry.folder.display()
                    ));
                    None
                },
            };
            (entry, record)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::WorkflowLoggingPolicy, workflow::instance::TargetKind};

    #[test]
    fn registry_preserves_global_identity_and_ignores_corrupt_entries() {
        let temp = tempfile::tempdir().unwrap();
        let layout = instance::project_layout(&temp.path().join("global"));
        let mut record = instance::new_instance_record(
            TargetKind::SharedLocal,
            &layout,
            2900,
            None,
            None,
            instance::StartedBy::Up,
            true,
            None,
            None,
        );
        record.pid = Some(std::process::id());
        record.exe = Some(std::env::current_exe().unwrap());
        instance::save_instance(&layout, &record).unwrap();
        let registry = temp.path().join("registry");
        register(&registry, &layout).unwrap();
        fs::write(registry.join("broken.json"), "{").unwrap();
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let entries = read_registry(&registry, &output).unwrap();
        assert_eq!(entries.len(), 1);
        let loaded = read_record(&entries[0]).unwrap().unwrap();
        assert_eq!(loaded.kind, TargetKind::SharedLocal);
        assert!(instance::instance_is_live(&loaded));
        instance::clear_live_pids(&mut record);
        instance::save_instance(&layout, &record).unwrap();
        assert!(!instance::instance_is_live(&read_record(&entries[0]).unwrap().unwrap()));
        assert!(output
            .test_buffered_terminal_lines()
            .iter()
            .any(|line| line.contains("Skipping unreadable")));
    }

    #[test]
    fn registry_deduplicates_and_reads_current_state_without_creating_missing_folders() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("project");
        let layout = instance::project_layout(&folder);
        let mut record = instance::new_instance_record(
            TargetKind::ProjectLocal,
            &layout,
            2900,
            None,
            None,
            instance::StartedBy::Up,
            true,
            None,
            Some(folder.clone()),
        );
        instance::save_instance(&layout, &record).unwrap();
        let registry = temp.path().join("registry");
        register(&registry, &layout).unwrap();
        register(&registry, &layout).unwrap();
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let entries = read_registry(&registry, &output).unwrap();
        assert_eq!(entries.len(), 1);
        record.http_port = 3001;
        record.url = instance::default_http_url(3001);
        instance::save_instance(&layout, &record).unwrap();
        assert_eq!(read_record(&entries[0]).unwrap().unwrap().url, "http://127.0.0.1:3001");
        fs::remove_dir_all(&folder).unwrap();
        assert!(read_record(&entries[0]).unwrap().is_none());
        assert!(!folder.exists());
    }
}
