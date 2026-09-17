//! Published in-memory resolution of project vs inline implementations.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use kalamdb_commons::{ArtifactId, Role, RoutineId, UserId};
use kalamdb_system::CatalogRoutine;

use crate::revision::ModuleRevision;

/// Immutable snapshot swapped on CAS activation.
#[derive(Debug, Clone)]
pub struct ActiveFunctionSet {
    pub generation:      u64,
    pub contract_hash:   String,
    pub module_revision: Option<Arc<ModuleRevision>>,
    pub procedures:      HashMap<RoutineId, ImplementationRef>,
}

/// How one catalog routine is executed in this published set.
#[derive(Debug, Clone)]
pub enum ImplementationRef {
    Module {
        revision:  Arc<ModuleRevision>,
        procedure: ProcedureSlot,
    },
    Inline {
        artifact: Arc<InlineArtifact>,
    },
    Missing,
}

/// Compiled project export bound to a catalog routine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcedureSlot {
    pub export_name: String,
}

/// Compiled inline JavaScript (or stored TS source awaiting compile).
#[derive(Debug, Clone)]
pub struct InlineArtifact {
    pub artifact_id: ArtifactId,
    pub source:      Arc<str>,
    pub language:    String,
}

impl ActiveFunctionSet {
    pub fn empty() -> Self {
        Self {
            generation:      0,
            contract_hash:   String::new(),
            module_revision: None,
            procedures:      HashMap::new(),
        }
    }

    pub fn lookup(&self, routine_id: &RoutineId) -> ImplementationRef {
        self.procedures.get(routine_id).cloned().unwrap_or(ImplementationRef::Missing)
    }

    /// Resolve module vs inline at publish. A listed module export beats inline;
    /// a missing export restores the compiled inline artifact when one exists.
    pub fn resolve(
        generation: u64,
        contract_hash: String,
        module_revision: Option<Arc<ModuleRevision>>,
        module_exports: &HashSet<String>,
        routines: impl IntoIterator<Item = (CatalogRoutine, Option<Arc<InlineArtifact>>)>,
    ) -> Self {
        let has_module = module_revision.is_some();
        let mut procedures = HashMap::new();
        for (routine, inline) in routines {
            let routine_id = routine.routine_id.clone();
            let export = routine_id.as_str();
            let module_backed = has_module && module_exports.contains(export);
            let implementation = if module_backed {
                ImplementationRef::Module {
                    revision:  Arc::clone(module_revision.as_ref().expect("module present")),
                    procedure: ProcedureSlot {
                        export_name: export.to_string(),
                    },
                }
            } else if let Some(artifact) = inline {
                ImplementationRef::Inline { artifact }
            } else {
                ImplementationRef::Missing
            };
            procedures.insert(routine_id, implementation);
        }
        Self {
            generation,
            contract_hash,
            module_revision,
            procedures,
        }
    }
}

/// Immutable actor and pinned deployment for one root invocation.
#[derive(Clone)]
pub struct FunctionExecutionRoot {
    pub actor_user: UserId,
    pub actor_role: Role,
    pub origin:     crate::FunctionCallOrigin,
    pub deployment: Arc<ActiveFunctionSet>,
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::{
        models::{NamespaceId, RoutineId, RoutineSecurityMode, UserId},
        ArtifactId, FunctionModuleId, Role,
    };

    use super::*;

    fn routine(name: &str, body: Option<&str>) -> CatalogRoutine {
        let namespace = NamespaceId::new("api");
        CatalogRoutine {
            routine_id:         RoutineId::from_parts(Some(&namespace), name),
            namespace_id:       namespace,
            name:               name.to_string(),
            owner:              UserId::new("root"),
            security:           RoutineSecurityMode::Invoker,
            language:           body.map(|_| "JAVASCRIPT".to_string()),
            body:               body.map(str::to_string),
            return_type_id:     None,
            return_type_name:   None,
            return_is_array:    false,
            return_not_null:    false,
            comment:            None,
            return_data_type:   None,
            inline_source_hash: None,
            inline_artifact_id: None,
        }
    }

    fn inline(source: &str) -> Arc<InlineArtifact> {
        Arc::new(InlineArtifact {
            artifact_id: ArtifactId::new("inline"),
            source:      source.into(),
            language:    "JAVASCRIPT".to_string(),
        })
    }

    fn module() -> Arc<ModuleRevision> {
        Arc::new(ModuleRevision {
            module_id:     FunctionModuleId::new("backend"),
            revision_id:   kalamdb_commons::FunctionRevisionId::new("backend:rev"),
            artifact_id:   ArtifactId::new("mod"),
            runtime:       kalamdb_commons::FunctionRuntime::Typescript,
            abi_version:   2,
            contract_hash: "c1".to_string(),
            source:        "function kalamInvoke() {}".into(),
        })
    }

    #[test]
    fn module_export_beats_inline_artifact() {
        let health = routine("health", Some("return 1;"));
        let id = health.routine_id.clone();
        let exports = HashSet::from([id.as_str().to_string()]);
        let set = ActiveFunctionSet::resolve(
            1,
            "c1".into(),
            Some(module()),
            &exports,
            vec![(health, Some(inline("return 1;")))],
        );
        assert!(matches!(set.lookup(&id), ImplementationRef::Module { .. }));
    }

    #[test]
    fn missing_export_restores_inline() {
        let health = routine("health", Some("return 1;"));
        let id = health.routine_id.clone();
        let exports = HashSet::from(["api.other".to_string()]);
        let set = ActiveFunctionSet::resolve(
            1,
            "c1".into(),
            Some(module()),
            &exports,
            vec![(health, Some(inline("return 1;")))],
        );
        assert!(matches!(set.lookup(&id), ImplementationRef::Inline { .. }));
    }

    #[test]
    fn bodyless_without_module_is_missing() {
        let order = routine("create_order", None);
        let id = order.routine_id.clone();
        let set = ActiveFunctionSet::resolve(
            1,
            String::new(),
            None,
            &HashSet::new(),
            vec![(order, None)],
        );
        assert!(matches!(set.lookup(&id), ImplementationRef::Missing));
    }

    #[test]
    fn bodyless_with_empty_module_exports_is_missing() {
        let order = routine("create_order", None);
        let id = order.routine_id.clone();
        let set = ActiveFunctionSet::resolve(
            1,
            "c1".into(),
            Some(module()),
            &HashSet::new(),
            vec![(order, None)],
        );
        assert!(matches!(set.lookup(&id), ImplementationRef::Missing));
    }

    #[test]
    fn empty_exports_keep_inline_artifact() {
        let health = routine("health", Some("return 1;"));
        let id = health.routine_id.clone();
        let set = ActiveFunctionSet::resolve(
            1,
            "c1".into(),
            Some(module()),
            &HashSet::new(),
            vec![(health, Some(inline("return 1;")))],
        );
        assert!(matches!(set.lookup(&id), ImplementationRef::Inline { .. }));
    }

    #[test]
    fn in_flight_root_stays_on_pinned_generation() {
        let set_41 = ActiveFunctionSet {
            generation:      41,
            contract_hash:   "rev-41".into(),
            module_revision: None,
            procedures:      HashMap::new(),
        };
        let set_42 = ActiveFunctionSet {
            generation:      42,
            contract_hash:   "rev-42".into(),
            module_revision: None,
            procedures:      HashMap::new(),
        };
        let root = FunctionExecutionRoot {
            actor_user: UserId::new("alice"),
            actor_role: Role::User,
            origin:     crate::FunctionCallOrigin::Sql,
            deployment: Arc::new(set_41),
        };
        assert_eq!(root.deployment.generation, 41);
        assert_eq!(set_42.generation, 42);
        assert_eq!(root.deployment.contract_hash, "rev-41");
    }
}
