//! Typed Python clients for the routines a contract declares.
//!
//! TypeScript and Dart call routines through the client's `invokeRoutine`
//! transport. The Python native client has no such entry point, so these
//! methods go through `query` / `query_rows` with the CALL and SELECT SQL
//! [`ProcedureCatalog`] already builds. The surface is the same: one class per
//! schema, one method per routine, with the generated request and result types.

use kalamdb_sql::contracts::ContractSnapshot;

use super::rows::{assign_unique_field_names, python_field_name, PyColumn};
use crate::workflow::schema::{
    naming::{pascal_case, snake_case, AssignedNames},
    procedures::{ProcedureCatalog, ProcedureSpec},
};

/// The whole functions section, or an empty string when nothing declares one.
///
/// Rendered here rather than in the template: the shape varies with whether a
/// routine takes parameters and whether it returns anything, and the blank
/// lines between generated classes should not depend on Handlebars'
/// standalone-tag whitespace rules.
pub fn functions_block(
    snapshot: &ContractSnapshot,
    names: &AssignedNames,
    procedures: &ProcedureCatalog<'_>,
) -> String {
    if procedures.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    let mut namespaces: Vec<(String, String)> = Vec::new();
    for namespace in procedures.namespaces() {
        let class_name = format!("{}Functions", pascal_case(namespace.schema));
        let attr_name = python_field_name(&snake_case(namespace.schema));
        namespaces.push((attr_name, class_name.clone()));

        out.push_str(&format!(
            "\n\nclass {class_name}:\n    \
             \"\"\"Routines declared in the {} schema.\"\"\"\n\n    \
             def __init__(self, client: KalamClient) -> None:\n        \
             self._client = client\n",
            namespace.schema
        ));
        for spec in &namespace.procedures {
            out.push_str(&method(spec, names, snapshot));
        }
    }

    out.push_str(
        "\n\nclass KalamFunctions:\n    \
         \"\"\"Server-side routines from schema.sql, grouped by schema.\"\"\"\n\n    \
         def __init__(self, client: KalamClient) -> None:\n",
    );
    for (attr_name, class_name) in &namespaces {
        out.push_str(&format!("        self.{attr_name} = {class_name}(client)\n"));
    }
    // The template supplies the blank lines around the block.
    out.trim_end().to_string()
}

fn method(spec: &ProcedureSpec<'_>, names: &AssignedNames, snapshot: &ContractSnapshot) -> String {
    let method_ident = python_field_name(&spec.routine.name);
    let routine_id = format!("{}.{}", spec.routine.schema, spec.routine.name);
    let signature = if spec.has_params() {
        format!("self, request: {}", spec.rust_input_ident())
    } else {
        "self".to_string()
    };
    let returns = match spec.return_type() {
        Some(_) => spec.rust_output_ident(),
        None => "None".to_string(),
    };

    let mut body = format!("\n    async def {method_ident}({signature}) -> {returns}:\n");
    if let Some(doc) = docstring(spec.comment()) {
        body.push_str(&doc);
    }

    // Parameters bind in declaration order, which is the order the catalog
    // numbered the placeholders in.
    let arguments = if spec.has_params() {
        let mut params: Vec<PyColumn> = spec
            .parameters()
            .iter()
            .map(|field| PyColumn::from_field(field, names, snapshot))
            .collect();
        // The request dataclass renames colliding fields the same way, so the
        // two agree on which attribute holds which parameter.
        assign_unique_field_names(&mut params);
        let binds: Vec<String> = params
            .iter()
            .map(|param| format!("encode_param(request.{})", param.field_name))
            .collect();
        format!("\n            [{}],", binds.join(", "))
    } else {
        String::new()
    };

    let sql = &spec.call_sql;
    match spec.return_type() {
        None => {
            body.push_str(&format!(
                "        await self._client.query(\n            \"{sql}\",{arguments}\n        )\n"
            ));
        },
        Some(field) => {
            let decode = PyColumn::from_field(field, names, snapshot)
                .decode_value(&format!("routine_result(\"{routine_id}\", rows)"));
            body.push_str(&format!(
                "        rows = await self._client.query_rows(\n            \
                 \"{sql}\",{arguments}\n        )\n        return {decode}\n"
            ));
        },
    }
    body
}

/// A method docstring indented into the body, or nothing without a comment.
fn docstring(comment: Option<&str>) -> Option<String> {
    let text = comment.map(str::trim).filter(|text| !text.is_empty())?;
    let body: Vec<String> = text
        .lines()
        .map(|line| format!("        {}", line.replace('\\', "\\\\").replace('"', "'")))
        .collect();
    if body.len() == 1 {
        return Some(format!("        \"\"\"{}\"\"\"\n", body[0].trim_start()));
    }
    Some(format!("        \"\"\"\n{}\n        \"\"\"\n", body.join("\n")))
}
