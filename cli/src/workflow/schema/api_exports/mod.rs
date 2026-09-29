//! Optional contract exports: identity manifest, nested FlatBuffers, OpenAPI, protobuf.

mod flatbuffers;
mod openapi;
mod protobuf;

use kalamdb_sql::contracts::{ContractField, ContractSnapshot, TypeIdentityManifest};

use crate::{
    error::Result,
    workflow::schema::{
        naming::AssignedNames,
        output::{write_text, SchemaEmitInput},
    },
};

pub fn write_api_exports(input: &SchemaEmitInput<'_>) -> Result<()> {
    let generated_dir = input.output_path.parent().unwrap_or(input.output_path);
    write_text(
        &generated_dir.join("identity.json"),
        &serde_json::to_string_pretty(&TypeIdentityManifest::from_snapshot(input.snapshot))
            .unwrap_or_else(|_| "{}".to_string()),
    )?;
    write_text(
        &generated_dir.join("types.fbs"),
        &flatbuffers::render(input.snapshot, input.names),
    )?;
    write_text(
        &generated_dir.join("openapi.yaml"),
        &openapi::render(input.snapshot, input.names),
    )?;
    write_text(
        &generated_dir.join("types.proto"),
        &protobuf::render(input.snapshot, input.names),
    )?;
    Ok(())
}

fn for_each_composite(
    snapshot: &ContractSnapshot,
    names: &AssignedNames,
    mut visit: impl FnMut(&str, &[ContractField]),
) {
    for (id, ty) in &snapshot.types {
        let kalamdb_sql::contracts::ContractTypeKind::Composite { fields } = &ty.kind else {
            continue;
        };
        visit(names.type_ident(id), fields);
    }
}

fn live_fields(fields: &[ContractField]) -> impl Iterator<Item = &ContractField> {
    fields.iter().filter(|field| !field.dropped)
}
