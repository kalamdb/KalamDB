//! EXECUTE ACL evaluation for SQL procedures.

use kalamdb_commons::{
    models::{RoutineGrantee, UserId},
    Role,
};
use kalamdb_functions::FunctionsError;
use kalamdb_system::{CatalogRoutine, CatalogStores};

use crate::error::KalamDbError;

pub fn require_execute(
    stores: &CatalogStores,
    routine: &CatalogRoutine,
    user_id: &UserId,
    role: Role,
) -> Result<(), KalamDbError> {
    if matches!(role, Role::Dba | Role::System) {
        return Ok(());
    }
    if routine.owner == *user_id {
        return Ok(());
    }
    let grants = stores.list_grants(&routine.routine_id).map_err(|error| {
        KalamDbError::CatalogError(format!("failed to load routine grants: {error}"))
    })?;
    let allowed = grants.iter().any(|grant| grantee_matches(&grant.grantee, role));
    if allowed {
        Ok(())
    } else if matches!(role, Role::Anonymous) {
        Err(KalamDbError::from(FunctionsError::AuthenticationRequired))
    } else {
        Err(KalamDbError::from(FunctionsError::ExecuteDenied(
            routine.routine_id.to_string(),
        )))
    }
}

fn grantee_matches(grantee: &RoutineGrantee, role: Role) -> bool {
    match grantee {
        RoutineGrantee::Public => !matches!(role, Role::Anonymous),
        RoutineGrantee::User => role == Role::User,
        RoutineGrantee::Service => role == Role::Service,
        RoutineGrantee::Anonymous => role == Role::Anonymous,
        RoutineGrantee::Role(name) => role.as_str().eq_ignore_ascii_case(name),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kalamdb_commons::{
        models::{
            NamespaceId, RoutineGrantId, RoutineGrantee, RoutineId, RoutineSecurityMode, UserId,
        },
        Role,
    };
    use kalamdb_store::test_utils::InMemoryBackend;
    use kalamdb_system::{CatalogRoutine, CatalogRoutineGrant, CatalogStores};

    use super::require_execute;

    fn stores() -> CatalogStores {
        CatalogStores::new(Arc::new(InMemoryBackend::new()))
    }

    fn routine() -> CatalogRoutine {
        let ns = NamespaceId::new("api");
        CatalogRoutine {
            routine_id:         RoutineId::from_parts(Some(&ns), "health"),
            namespace_id:       ns,
            name:               "health".to_string(),
            owner:              UserId::new("owner"),
            security:           RoutineSecurityMode::Invoker,
            language:           None,
            body:               None,
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

    fn grant(routine: &CatalogRoutine, grantee: RoutineGrantee) -> CatalogRoutineGrant {
        CatalogRoutineGrant {
            grant_id: RoutineGrantId::new(&routine.routine_id, &grantee),
            routine_id: routine.routine_id.clone(),
            grantee,
        }
    }

    #[test]
    fn execute_denied_before_enter_for_user_without_grant() {
        let stores = stores();
        let routine = routine();
        stores.upsert_routine(routine.clone()).unwrap();
        let err =
            require_execute(&stores, &routine, &UserId::new("alice"), Role::User).unwrap_err();
        assert_eq!(
            err.function_error_code(),
            Some(kalamdb_functions::FunctionErrorCode::ExecuteDenied)
        );
    }

    #[test]
    fn anonymous_denied_by_default() {
        let stores = stores();
        let routine = routine();
        stores.upsert_routine(routine.clone()).unwrap();
        stores.upsert_grant(grant(&routine, RoutineGrantee::Public)).unwrap();
        let err =
            require_execute(&stores, &routine, &UserId::new("anon"), Role::Anonymous).unwrap_err();
        assert_eq!(
            err.function_error_code(),
            Some(kalamdb_functions::FunctionErrorCode::AuthenticationRequired)
        );
    }

    #[test]
    fn explicit_anonymous_grant_allows_execute() {
        let stores = stores();
        let routine = routine();
        stores.upsert_routine(routine.clone()).unwrap();
        stores.upsert_grant(grant(&routine, RoutineGrantee::Anonymous)).unwrap();
        require_execute(&stores, &routine, &UserId::new("anon"), Role::Anonymous).unwrap();
    }

    #[test]
    fn public_grant_does_not_match_anonymous() {
        let stores = stores();
        let routine = routine();
        stores.upsert_routine(routine.clone()).unwrap();
        stores.upsert_grant(grant(&routine, RoutineGrantee::Public)).unwrap();
        require_execute(&stores, &routine, &UserId::new("alice"), Role::User).unwrap();
        let err =
            require_execute(&stores, &routine, &UserId::new("anon"), Role::Anonymous).unwrap_err();
        assert!(err.function_error_code().is_some());
    }
}
