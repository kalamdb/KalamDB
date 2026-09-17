use std::fs;

use crate::{
    error::{CLIError, Result},
    workflow::{
        schema::{
            compile_project_contract,
            procedure_bindings::discover_procedure_bindings,
            typescript::{implemented_procedure_source, procedure_impl_path},
        },
        WorkflowContext,
    },
};

pub fn override_function(ctx: &WorkflowContext, procedure: &str) -> Result<()> {
    let (namespace, name) = procedure.split_once('.').ok_or_else(|| {
        CLIError::ConfigurationError(
            "functions override requires namespace.name (example: api.health)".into(),
        )
    })?;
    let key = format!("{namespace}.{name}");
    if let Ok((snapshot, _)) = compile_project_contract(&ctx.project_root, &ctx.config) {
        let bindings = discover_procedure_bindings(&ctx.project_root, &snapshot)?;
        if let Some(existing) = bindings.iter().find(|binding| binding.routine_id == key) {
            return Err(CLIError::ConfigurationError(format!(
                "procedure {key} is already bound as '{}' in {}",
                existing.export_name,
                existing.source_path.display()
            )));
        }
    }
    let path = procedure_impl_path(&ctx.project_root, namespace, name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            CLIError::FileError(format!("failed to create '{}': {error}", parent.display()))
        })?;
    }
    if path.exists() {
        return Err(CLIError::ConfigurationError(format!(
            "override already exists at {}",
            path.display()
        )));
    }
    let body = seed_override_body(ctx, namespace, name)?;
    fs::write(&path, body).map_err(|error| {
        CLIError::FileError(format!("failed to write '{}': {error}", path.display()))
    })?;
    ctx.output().status(format!("scaffolded {}", path.display()));
    Ok(())
}

fn seed_override_body(ctx: &WorkflowContext, namespace: &str, name: &str) -> Result<String> {
    let Ok((snapshot, _)) = compile_project_contract(&ctx.project_root, &ctx.config) else {
        return implemented_procedure_source(namespace, name, None);
    };
    let key = format!("{namespace}.{name}");
    let inline = snapshot.routines.get(&key).and_then(|routine| routine.body.clone());
    implemented_procedure_source(namespace, name, inline.as_deref())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn override_scaffolds_named_builder() {
        let temp = TempDir::new().unwrap();
        let ctx = crate::workflow::test_support::test_workflow_context(temp.path());
        override_function(&ctx, "api.health").unwrap();
        let path = temp.path().join("functions/src/api/health.ts");
        let source = fs::read_to_string(&path).unwrap();
        assert!(source.contains("procedure.api.health("));
        assert!(source.contains("export const health"));
        let err = override_function(&ctx, "api.health").unwrap_err();
        assert!(
            err.to_string().contains("already bound") || err.to_string().contains("already exists"),
            "{err}"
        );
    }
}
