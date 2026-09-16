//! Compile the project schema contract from local SQL files.

use std::path::Path;

use kalamdb_sql::{canonical_contract_hash, compile_contract, ContractSnapshot, ContractSource};

use crate::{
    error::{CLIError, Result},
    workflow::{
        io::read_schema_file,
        project::config::{KalamProjectConfig, SchemaMode},
        schema::naming::DEFAULT_NAMESPACE,
    },
};

pub fn compile_project_contract(
    project_root: &Path,
    config: &KalamProjectConfig,
) -> Result<(ContractSnapshot, String)> {
    match config.schema.mode {
        SchemaMode::Sql => {},
        SchemaMode::Remote => {
            return Err(CLIError::ConfigurationError(
                "schema generation requires local SQL files; set schema.mode = 'sql'".into(),
            ));
        },
    }
    let path = config
        .schema_source_path(project_root)
        .ok_or_else(|| CLIError::ConfigurationError("schema.path is not configured".into()))?;
    if !path.exists() {
        return Err(CLIError::FileError(format!(
            "schema file '{}' does not exist",
            path.display()
        )));
    }

    let mut owned = Vec::new();
    if path.is_dir() {
        let mut files = Vec::new();
        collect_sql_files(&path, &mut files)?;
        files.sort();
        for file in files {
            let sql = read_schema_file(&file)?;
            let display = file.strip_prefix(project_root).unwrap_or(&file).display().to_string();
            owned.push((display, sql));
        }
    } else {
        let sql = read_schema_file(&path)?;
        let display = path.strip_prefix(project_root).unwrap_or(&path).display().to_string();
        owned.push((display, sql));
    }

    if owned.is_empty() {
        owned.push(("<empty>".to_string(), String::new()));
    }

    let sources: Vec<ContractSource<'_>> = owned
        .iter()
        .map(|(path, sql)| ContractSource {
            path: path.as_str(),
            sql:  sql.as_str(),
        })
        .collect();
    let snapshot = compile_contract(&sources, DEFAULT_NAMESPACE).map_err(|err| {
        CLIError::ConfigurationError(format!("failed to compile schema contract: {}", err.message))
    })?;
    let hash = canonical_contract_hash(&snapshot);
    Ok((snapshot, hash))
}

fn collect_sql_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<()> {
    let entries = std::fs::read_dir(dir).map_err(|error| {
        CLIError::FileError(format!("failed to read schema directory '{}': {error}", dir.display()))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            CLIError::FileError(format!(
                "failed to read schema directory '{}': {error}",
                dir.display()
            ))
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_sql_files(&path, out)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("sql") {
            out.push(path);
        }
    }
    Ok(())
}
