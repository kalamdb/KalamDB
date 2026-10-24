//! Compile the project schema contract from local SQL files.

use std::path::Path;

use kalamdb_commons::{
    datatypes::ToArrowType,
    models::{RoutineId, RoutineSecurityMode, TypeId},
    KalamDataType,
};
use kalamdb_sql::{
    apply_identity_manifest, canonical_contract_hash, compile_contract,
    contracts::{ContractField, ContractRoutine, ContractType, ContractTypeKind},
    ContractSnapshot, ContractSource, TypeIdentityManifest,
};

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
    let mut snapshot =
        compile_contract(&sources, project_contract_namespace(config)).map_err(|err| {
            CLIError::ConfigurationError(format!(
                "failed to compile schema contract: {}",
                err.message
            ))
        })?;
    let identity_path = if path.is_dir() {
        Some(path.join("identity.json"))
    } else {
        path.parent().map(|parent| parent.join("identity.json"))
    };
    if let Some(identity_path) = identity_path {
        if identity_path.exists() {
            let json = read_schema_file(&identity_path)?;
            let manifest = TypeIdentityManifest::parse(&json).map_err(|err| {
                CLIError::ConfigurationError(format!(
                    "failed to parse type identity manifest: {}",
                    err.message
                ))
            })?;
            apply_identity_manifest(&mut snapshot, &manifest).map_err(|err| {
                CLIError::ConfigurationError(format!(
                    "failed to apply type identity manifest: {}",
                    err.message
                ))
            })?;
        }
    }
    merge_builtin_file_extension(&mut snapshot);
    let hash = canonical_contract_hash(&snapshot);
    Ok((snapshot, hash))
}

/// Merge the built-in `pg_catalog.file` composite type and its three filestore
/// functions (`file_put`, `file_read`, `file_delete`) into a [`ContractSnapshot`].
pub fn merge_builtin_file_extension(snapshot: &mut ContractSnapshot) {
    let file_type_id = TypeId::new("pg_catalog.file");
    snapshot.schemas.insert("pg_catalog".to_string());

    let file_fields = vec![
        builtin_field("id", "TEXT", KalamDataType::Text, true, 1),
        builtin_field("sub", "TEXT", KalamDataType::Text, true, 2),
        builtin_field("name", "TEXT", KalamDataType::Text, true, 3),
        builtin_field("size", "BIGINT", KalamDataType::BigInt, true, 4),
        builtin_field("mime", "TEXT", KalamDataType::Text, true, 5),
        builtin_field("sha256", "TEXT", KalamDataType::Text, true, 6),
        builtin_field("shard", "INT", KalamDataType::Int, false, 7),
    ];
    let file_arrow = KalamDataType::File
        .to_arrow_type()
        .expect("KalamDataType::File has a canonical Arrow DataType");

    snapshot
        .types
        .entry("pg_catalog.file".to_string())
        .or_insert_with(|| ContractType {
            type_id: file_type_id.clone(),
            schema:  "pg_catalog".to_string(),
            name:    "file".to_string(),
            kind:    ContractTypeKind::Composite {
                fields: file_fields,
            },
            arrow:   file_arrow,
            comment: None,
        });

    for table in snapshot.tables.values_mut() {
        for field in &mut table.fields {
            rewrite_file_field(field, &file_type_id);
        }
    }
    for ty in snapshot.types.values_mut() {
        if let ContractTypeKind::Composite { fields }
        | ContractTypeKind::ImplicitTableRow { fields, .. } = &mut ty.kind
        {
            for field in fields {
                rewrite_file_field(field, &file_type_id);
            }
        }
    }
    for routine in snapshot.routines.values_mut() {
        for param in &mut routine.parameters {
            rewrite_file_field(param, &file_type_id);
        }
        if let Some(ret) = &mut routine.return_type {
            rewrite_file_field(ret, &file_type_id);
        }
    }

    snapshot
        .routines
        .entry("pg_catalog.file_put".to_string())
        .or_insert_with(|| ContractRoutine {
            routine_id: RoutineId::new("pg_catalog.file_put"),
            signature: "pg_catalog.file_put(text)".to_string(),
            schema: "pg_catalog".to_string(),
            name: "file_put".to_string(),
            kind: kalamdb_commons::models::RoutineKind::Function,
            cardinality: kalamdb_sql::contracts::RoutineCardinality::Scalar,
            parameters: vec![builtin_field(
                "upload_key",
                "TEXT",
                KalamDataType::Text,
                true,
                1,
            )],
            return_type: Some(ContractField {
                name: "returns".to_string(),
                type_name: "pg_catalog.file".to_string(),
                type_id: Some(file_type_id.clone()),
                data_type: Some(KalamDataType::File),
                not_null: true,
                ..Default::default()
            }),
            language: Some("INTERNAL".to_string()),
            security: RoutineSecurityMode::Invoker,
            is_internal: true,
            ..Default::default()
        });

    snapshot.routines.entry("pg_catalog.file_read".to_string()).or_insert_with(|| {
        ContractRoutine {
            routine_id: RoutineId::new("pg_catalog.file_read"),
            signature: "pg_catalog.file_read(pg_catalog.file)".to_string(),
            schema: "pg_catalog".to_string(),
            name: "file_read".to_string(),
            kind: kalamdb_commons::models::RoutineKind::Function,
            cardinality: kalamdb_sql::contracts::RoutineCardinality::Scalar,
            parameters: vec![ContractField {
                name: "file".to_string(),
                type_name: "pg_catalog.file".to_string(),
                type_id: Some(file_type_id.clone()),
                data_type: Some(KalamDataType::File),
                not_null: true,
                slot: 1,
                ..Default::default()
            }],
            return_type: Some(builtin_field("returns", "BYTES", KalamDataType::Bytes, true, 0)),
            language: Some("INTERNAL".to_string()),
            security: RoutineSecurityMode::Invoker,
            is_internal: true,
            ..Default::default()
        }
    });

    snapshot
        .routines
        .entry("pg_catalog.file_delete".to_string())
        .or_insert_with(|| ContractRoutine {
            routine_id: RoutineId::new("pg_catalog.file_delete"),
            signature: "pg_catalog.file_delete(pg_catalog.file)".to_string(),
            schema: "pg_catalog".to_string(),
            name: "file_delete".to_string(),
            kind: kalamdb_commons::models::RoutineKind::Function,
            cardinality: kalamdb_sql::contracts::RoutineCardinality::Void,
            parameters: vec![ContractField {
                name: "file".to_string(),
                type_name: "pg_catalog.file".to_string(),
                type_id: Some(file_type_id.clone()),
                data_type: Some(KalamDataType::File),
                not_null: true,
                slot: 1,
                ..Default::default()
            }],
            return_type: None,
            language: Some("INTERNAL".to_string()),
            security: RoutineSecurityMode::Invoker,
            is_internal: true,
            ..Default::default()
        });
}

fn builtin_field(
    name: &str,
    type_name: &str,
    data_type: KalamDataType,
    not_null: bool,
    slot: i32,
) -> ContractField {
    ContractField {
        name: name.to_string(),
        type_name: type_name.to_string(),
        type_id: None,
        data_type: Some(data_type),
        not_null,
        slot,
        ..Default::default()
    }
}

fn rewrite_file_field(field: &mut ContractField, file_type_id: &TypeId) {
    if field.type_id.is_none()
        && (field.type_name.eq_ignore_ascii_case("FILE")
            || field.data_type == Some(KalamDataType::File))
    {
        field.type_id = Some(file_type_id.clone());
        field.type_name = "pg_catalog.file".to_string();
    }
}

/// Unqualified SQL in schema.sql belongs to the project's connection namespace.
///
/// `kalam dev` applies that SQL in the same namespace. Generating `public.*`
/// pointed the starter app at a schema Drizzle refuses to name and that the
/// dev server never created.
pub(crate) fn project_contract_namespace(config: &KalamProjectConfig) -> &str {
    config
        .connection
        .get(&config.project.default_env)
        .map(|env| env.namespace.as_str())
        .unwrap_or(DEFAULT_NAMESPACE)
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
