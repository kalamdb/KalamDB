//! Portable SQL identity manifest: `(schema, name)` → TypeId + field slots.
//!
//! Local compile cannot tell a rename from drop/add. Check this file in with
//! SQL contracts. Never regenerate slots silently.

use std::collections::BTreeMap;

use kalamdb_commons::models::TypeId;
use serde::{Deserialize, Serialize};

use super::{snapshot::ContractSnapshot, ContractError};

/// Source `(schema.name)` → opaque TypeId and never-reused slots.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeIdentityManifest {
    #[serde(default)]
    pub types: BTreeMap<String, TypeIdentityEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeIdentityEntry {
    pub type_id: TypeId,
    #[serde(default)]
    pub slots:   BTreeMap<String, i32>,
}

impl TypeIdentityManifest {
    pub fn parse(json: &str) -> Result<Self, ContractError> {
        serde_json::from_str(json)
            .map_err(|error| ContractError::new(format!("invalid type identity manifest: {error}")))
    }

    pub fn from_snapshot(snapshot: &ContractSnapshot) -> Self {
        let mut types = BTreeMap::new();
        for (key, ty) in &snapshot.types {
            let slots = match &ty.kind {
                super::snapshot::ContractTypeKind::Composite { fields }
                | super::snapshot::ContractTypeKind::ImplicitTableRow { fields, .. } => fields
                    .iter()
                    .filter(|field| field.slot > 0)
                    .map(|field| (field.name.clone(), field.slot))
                    .collect(),
                _ => BTreeMap::new(),
            };
            types.insert(
                key.clone(),
                TypeIdentityEntry {
                    type_id: ty.type_id.clone(),
                    slots,
                },
            );
        }
        Self { types }
    }
}

/// Overlay checked-in TypeIds and slots. Unknown fields get the next unused slot.
pub fn apply_identity_manifest(
    snapshot: &mut ContractSnapshot,
    manifest: &TypeIdentityManifest,
) -> Result<(), ContractError> {
    for (key, ty) in snapshot.types.iter_mut() {
        let Some(entry) = manifest.types.get(key) else {
            continue;
        };
        if entry.type_id.as_str().is_empty() {
            return Err(ContractError::new(format!(
                "identity manifest for '{key}' has an empty type_id"
            )));
        }
        ty.type_id = entry.type_id.clone();
        match &mut ty.kind {
            super::snapshot::ContractTypeKind::Composite { fields }
            | super::snapshot::ContractTypeKind::ImplicitTableRow { fields, .. } => {
                assign_slots(fields, &entry.slots)?;
            },
            _ => {},
        }
    }
    rewrite_type_id_refs(snapshot);
    Ok(())
}

fn rewrite_type_id_refs(snapshot: &mut ContractSnapshot) {
    let alias_to_id: BTreeMap<String, TypeId> = snapshot
        .types
        .iter()
        .map(|(key, ty)| (key.clone(), ty.type_id.clone()))
        .collect();
    let rewrite = |field: &mut super::snapshot::ContractField| {
        if let Some(type_id) = &field.type_id {
            if let Some(opaque) = alias_to_id.get(type_id.as_str()) {
                field.type_id = Some(opaque.clone());
            }
        }
    };
    for ty in snapshot.types.values_mut() {
        match &mut ty.kind {
            super::snapshot::ContractTypeKind::Composite { fields }
            | super::snapshot::ContractTypeKind::ImplicitTableRow { fields, .. } => {
                fields.iter_mut().for_each(&rewrite);
            },
            super::snapshot::ContractTypeKind::RowAlias { source } => {
                if let Some(opaque) = alias_to_id.get(source.as_str()) {
                    *source = opaque.clone();
                }
            },
            _ => {},
        }
    }
    for table in snapshot.tables.values_mut() {
        table.fields.iter_mut().for_each(&rewrite);
    }
    for routine in snapshot.routines.values_mut() {
        routine.parameters.iter_mut().for_each(&rewrite);
        if let Some(ret) = &mut routine.return_type {
            rewrite(ret);
        }
    }
}

fn assign_slots(
    fields: &mut [super::snapshot::ContractField],
    slots: &BTreeMap<String, i32>,
) -> Result<(), ContractError> {
    let mut used: BTreeMap<i32, String> = BTreeMap::new();
    for (name, slot) in slots {
        if *slot <= 0 {
            return Err(ContractError::new(format!(
                "identity slot for '{name}' must be a positive integer"
            )));
        }
        if let Some(existing) = used.insert(*slot, name.clone()) {
            return Err(ContractError::new(format!(
                "identity slot {slot} reused by '{existing}' and '{name}'"
            )));
        }
    }
    let mut next = slots.values().copied().max().unwrap_or(0) + 1;
    for field in fields.iter_mut() {
        if let Some(slot) = slots.get(&field.name) {
            field.slot = *slot;
        } else if field.slot <= 0 {
            field.slot = next;
            next += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::compile_contract_sql;

    #[test]
    fn manifest_replaces_name_derived_type_id() {
        let mut snapshot =
            compile_contract_sql("CREATE TYPE chat.address AS (line1 TEXT);", "chat").unwrap();
        let key = "chat.address".to_string();
        let opaque = TypeId::new("9001");
        let mut manifest = TypeIdentityManifest::default();
        manifest.types.insert(
            key.clone(),
            TypeIdentityEntry {
                type_id: opaque.clone(),
                slots:   BTreeMap::from([("line1".to_string(), 1)]),
            },
        );
        apply_identity_manifest(&mut snapshot, &manifest).unwrap();
        let ty = snapshot.types.get(&key).unwrap();
        assert_eq!(ty.type_id, opaque);
        match &ty.kind {
            crate::contracts::ContractTypeKind::Composite { fields } => {
                assert_eq!(fields[0].slot, 1);
            },
            other => panic!("unexpected {other:?}"),
        }
    }
}
