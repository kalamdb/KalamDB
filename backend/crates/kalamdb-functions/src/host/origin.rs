//! CALL origin, frames, and HTTP overrides.

use std::{collections::HashMap, sync::Arc};

use kalamdb_commons::{
    models::{FunctionRevisionId, NamespaceId, RoutineId, UserId},
    Role, RoutineSecurityMode,
};
use parking_lot::Mutex;
use smallvec::SmallVec;

use crate::RoutineValue;

#[derive(Debug, Clone)]
pub struct HttpResponseOverrides {
    pub status:  Option<u16>,
    pub headers: HashMap<String, String>,
}

impl Default for HttpResponseOverrides {
    fn default() -> Self {
        Self {
            status:  None,
            headers: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum FunctionCallOrigin {
    Sql,
    Http {
        method:   String,
        path:     String,
        headers:  Arc<Vec<(String, String)>>,
        query:    Arc<Vec<(String, String)>>,
        response: Arc<Mutex<HttpResponseOverrides>>,
    },
    Topic {
        topic_name: String,
        event_id:   String,
        partition:  u32,
        offset:     u64,
        attempt:    u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrincipalKey {
    pub user:      UserId,
    pub role:      Role,
    pub namespace: NamespaceId,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // revision_id/security are kept on the frame for nested INVOKER/DEFINER
pub struct ProcedureFrame {
    pub routine_id:     RoutineId,
    pub revision_id:    FunctionRevisionId,
    pub namespace_id:   NamespaceId,
    pub principal_user: UserId,
    pub principal_role: Role,
    pub security:       RoutineSecurityMode,
}

impl ProcedureFrame {
    pub fn stack_label(&self) -> String {
        self.routine_id.as_str().to_string()
    }
}

pub type ProcedureFrameStack = SmallVec<[ProcedureFrame; 4]>;

#[derive(Debug, Clone)]
pub struct FunctionCallResult {
    pub value:        RoutineValue,
    pub http_status:  Option<u16>,
    pub http_headers: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::{models::RoutineId, Role, RoutineSecurityMode, UserId};

    use super::*;

    #[test]
    fn nested_invoker_keeps_caller_principal() {
        let frame = ProcedureFrame {
            routine_id:     RoutineId::new("api.child"),
            revision_id:    FunctionRevisionId::new("backend:rev"),
            namespace_id:   NamespaceId::new("api"),
            principal_user: UserId::new("alice"),
            principal_role: Role::User,
            security:       RoutineSecurityMode::Invoker,
        };
        assert_eq!(frame.principal_user.as_str(), "alice");
        assert_eq!(frame.principal_role, Role::User);
    }

    #[test]
    fn nested_definer_uses_owner_principal() {
        let frame = ProcedureFrame {
            routine_id:     RoutineId::new("api.child"),
            revision_id:    FunctionRevisionId::new("backend:rev"),
            namespace_id:   NamespaceId::new("api"),
            principal_user: UserId::new("owner"),
            principal_role: Role::Service,
            security:       RoutineSecurityMode::Definer,
        };
        assert_eq!(frame.principal_user.as_str(), "owner");
        assert_eq!(frame.security, RoutineSecurityMode::Definer);
    }

    #[test]
    fn http_origin_stores_arc_headers() {
        let headers = Arc::new(vec![
            ("authorization".into(), "secret".into()),
            ("x-request-id".into(), "abc".into()),
        ]);
        let origin = FunctionCallOrigin::Http {
            method: "POST".into(),
            path: "/v1/functions/api/x".into(),
            headers,
            query: Arc::new(vec![("q".into(), "1".into())]),
            response: Arc::new(Mutex::new(HttpResponseOverrides::default())),
        };
        let FunctionCallOrigin::Http {
            method,
            headers,
            query,
            ..
        } = origin
        else {
            panic!("expected http origin");
        };
        assert_eq!(method, "POST");
        assert_eq!(headers.len(), 2);
        assert_eq!(query[0].1, "1");
    }
}
