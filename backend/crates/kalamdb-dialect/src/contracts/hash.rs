//! Canonical contract hashing. Formatting and SQL `--` comments are stripped
//! before compile. Catalog `COMMENT` / `COMMENT ON` text is documentation-only
//! and is not hashed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::snapshot::{
    ContractField, ContractRoutine, ContractSnapshot, ContractTable, ContractType,
    ContractTypeKind, CURRENT_CONTRACT_EPOCH,
};

/// Scoped SHA-256 revision hashes per catalog entity so unrelated DDL does not invalidate
/// cached descriptors or ETags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedContractHashes {
    pub epoch: u16,
    pub contract_hash: String,
    pub types: BTreeMap<String, String>,
    pub tables: BTreeMap<String, String>,
    pub routines: BTreeMap<String, String>,
}

/// SHA-256 hex digest of the resolved logical contract. Independent of SQL whitespace/comments.
pub fn canonical_contract_hash(snapshot: &ContractSnapshot) -> String {
    sha256_hex(&canonical_bytes(snapshot))
}

/// Compute the full contract digest plus individual scoped entity digests (`T004`).
pub fn scoped_contract_hashes(snapshot: &ContractSnapshot) -> ScopedContractHashes {
    let mut types = BTreeMap::new();
    for (id, ty) in &snapshot.types {
        let mut buf = String::new();
        write_type_canonical(&mut buf, id, ty);
        types.insert(id.clone(), sha256_hex(buf.as_bytes()));
    }

    let mut tables = BTreeMap::new();
    for (id, table) in &snapshot.tables {
        let mut buf = String::new();
        write_table_canonical(&mut buf, id, table);
        for field in &table.fields {
            if let Some(type_id) = &field.type_id {
                if let Some(type_hash) = types.get(type_id.as_str()) {
                    buf.push_str("dep_type\t");
                    buf.push_str(type_id.as_str());
                    buf.push('\t');
                    buf.push_str(type_hash);
                    buf.push('\n');
                }
            }
        }
        tables.insert(id.clone(), sha256_hex(buf.as_bytes()));
    }

    let mut routines = BTreeMap::new();
    for (id, routine) in &snapshot.routines {
        let mut buf = String::new();
        write_routine_canonical(&mut buf, id, routine);
        for field in routine
            .parameters
            .iter()
            .chain(routine.return_type.iter())
            .chain(routine.returns_table.iter())
        {
            if let Some(type_id) = &field.type_id {
                if let Some(type_hash) = types.get(type_id.as_str()) {
                    buf.push_str("dep_type\t");
                    buf.push_str(type_id.as_str());
                    buf.push('\t');
                    buf.push_str(type_hash);
                    buf.push('\n');
                }
            }
        }
        routines.insert(id.clone(), sha256_hex(buf.as_bytes()));
    }

    ScopedContractHashes {
        epoch: CURRENT_CONTRACT_EPOCH,
        contract_hash: canonical_contract_hash(snapshot),
        types,
        tables,
        routines,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().fold(String::with_capacity(64), |mut out, byte| {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn canonical_bytes(snapshot: &ContractSnapshot) -> Vec<u8> {
    let mut out = String::new();
    for schema in &snapshot.schemas {
        out.push_str("schema\t");
        out.push_str(schema);
        out.push('\n');
    }
    for (id, ty) in &snapshot.types {
        write_type_canonical(&mut out, id, ty);
    }
    for (id, table) in &snapshot.tables {
        write_table_canonical(&mut out, id, table);
    }
    for (id, routine) in &snapshot.routines {
        write_routine_canonical(&mut out, id, routine);
    }
    for (id, trigger) in &snapshot.triggers {
        out.push_str("trigger\t");
        out.push_str(id);
        out.push('\t');
        out.push_str(&trigger.topic_id);
        out.push('\t');
        out.push_str(&trigger.routine_id);
        out.push('\t');
        out.push_str(&trigger.principal);
        out.push('\t');
        out.push_str(&trigger.start_from);
        out.push('\t');
        out.push_str(&trigger.retries.to_string());
        out.push('\t');
        out.push_str(&trigger.retry_backoff_ms.to_string());
        out.push('\t');
        out.push_str(&trigger.concurrency.to_string());
        out.push('\n');
    }
    out.into_bytes()
}

fn write_type_canonical(out: &mut String, id: &str, ty: &ContractType) {
    out.push_str("type\t");
    out.push_str(id);
    out.push('\t');
    match &ty.kind {
        ContractTypeKind::ImplicitTableRow { table_id, fields } => {
            out.push_str("implicit_table_row\t");
            out.push_str(table_id);
            out.push('\n');
            write_fields(out, fields);
        },
        ContractTypeKind::RowAlias { source } => {
            out.push_str("row_alias\t");
            out.push_str(source.as_str());
            out.push('\n');
        },
        ContractTypeKind::Composite { fields } => {
            out.push_str("composite\n");
            write_fields(out, fields);
        },
        ContractTypeKind::Enum { labels } => {
            out.push_str("enum\t");
            out.push_str(&labels.join(","));
            out.push('\n');
        },
        ContractTypeKind::Domain {
            base_type,
            collation,
            default_expr,
            not_null,
            checks,
        } => {
            out.push_str("domain\t");
            out.push_str(if *not_null { "1" } else { "0" });
            out.push('\t');
            out.push_str(collation.as_deref().unwrap_or(""));
            out.push('\t');
            out.push_str(default_expr.as_deref().unwrap_or(""));
            out.push('\n');
            write_field(out, base_type);
            for check in checks {
                out.push_str("check\t");
                out.push_str(check.name.as_deref().unwrap_or(""));
                out.push('\t');
                out.push_str(if check.validated { "1" } else { "0" });
                out.push('\t');
                out.push_str(&check.expression);
                out.push('\n');
            }
        },
        ContractTypeKind::TopicPayload { topic_id, sources } => {
            out.push_str("topic_payload\t");
            out.push_str(topic_id);
            out.push('\t');
            out.push_str(&sources.join(","));
            out.push('\n');
        },
    }
}

fn write_table_canonical(out: &mut String, id: &str, table: &ContractTable) {
    out.push_str("table\t");
    out.push_str(id);
    out.push('\t');
    out.push_str(table.kind.as_str());
    if let Some(alias) = &table.row_alias_id {
        out.push_str("\talias\t");
        out.push_str(alias.as_str());
    }
    if !table.primary_key.is_empty() {
        out.push_str("\tpk\t");
        out.push_str(&table.primary_key.join(","));
    }
    out.push('\n');
    write_fields(out, &table.fields);
}

fn write_routine_canonical(out: &mut String, id: &str, routine: &ContractRoutine) {
    out.push_str("routine\t");
    out.push_str(id);
    out.push('\t');
    out.push_str(routine.security.as_str());
    out.push('\t');
    out.push_str(routine.language.as_deref().unwrap_or(""));
    out.push('\t');
    out.push_str(routine.cardinality.as_str());
    if !routine.signature.is_empty() {
        out.push('\t');
        out.push_str(&routine.signature);
    }
    out.push('\n');
    write_fields(out, &routine.parameters);
    if let Some(ret) = &routine.return_type {
        out.push_str("returns\t");
        write_field(out, ret);
    }
    if !routine.returns_table.is_empty() {
        out.push_str("returns_table\n");
        write_fields(out, &routine.returns_table);
    }
    if let Some(body) = &routine.body {
        out.push_str("body\t");
        out.push_str(body.trim());
        out.push('\n');
    }
    for grant in &routine.grants {
        out.push_str("grant\t");
        out.push_str(&grant.as_sql());
        out.push('\n');
    }
}

fn write_fields(out: &mut String, fields: &[ContractField]) {
    for field in fields {
        write_field(out, field);
    }
}

fn write_field(out: &mut String, field: &ContractField) {
    out.push_str("field\t");
    out.push_str(&field.name);
    out.push('\t');
    out.push_str(&field.type_name);
    out.push('\t');
    if let Some(type_id) = &field.type_id {
        out.push_str(type_id.as_str());
    }
    out.push('\t');
    out.push_str(&field.slot.to_string());
    out.push('\t');
    out.push_str(if field.is_array { "1" } else { "0" });
    out.push('\t');
    out.push_str(if field.not_null { "1" } else { "0" });
    out.push('\t');
    out.push_str(if field.nonempty { "1" } else { "0" });
    out.push('\t');
    out.push_str(if field.dropped { "dropped" } else { "live" });
    if field.primary_key {
        out.push_str("\tpk:");
        out.push_str(&field.primary_key_ordinal.unwrap_or(1).to_string());
    }
    if field.has_default {
        out.push_str("\tdefault:");
        out.push_str(field.default_expr.as_deref().unwrap_or(""));
    }
    if field.is_identity {
        out.push_str("\tidentity");
    }
    if field.is_generated {
        out.push_str("\tgenerated");
    }
    if field.is_read_only {
        out.push_str("\treadonly");
    }
    if field.param_mode != kalamdb_commons::models::RoutineParameterMode::In {
        out.push_str("\tmode:");
        out.push_str(field.param_mode.as_str());
    }
    out.push('\n');
}
