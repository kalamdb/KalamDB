use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, BooleanBuilder, Int64Builder, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::Role;
use kalamdb_macros::table;
use kalamdb_session::is_admin_role;
use kalamdb_system::SystemTablesRegistry;

use crate::{
    error::RegistryError,
    pg_catalog::{stable_oid, PgCatalogView},
};

#[table(
    name = "pg_roles",
    namespace = SchemaId::pg_catalog(),
    view = true,
    comment = "Roles"
)]
#[derive(Debug)]
struct PgRolesRow {
    /// Role oid
    oid: i64,
    /// Role name
    rolname: String,
    /// Superuser
    rolsuper: bool,
    /// Inherits privileges
    rolinherit: bool,
    /// Can create roles
    rolcreaterole: bool,
    /// Can create databases
    rolcreatedb: bool,
    /// Can log in
    rolcanlogin: bool,
    /// Replication
    rolreplication: bool,
    /// Bypasses row security
    rolbypassrls: bool,
}

fn schema() -> SchemaRef {
    PgRolesRow::arrow_schema()
}

#[derive(Debug)]
pub struct PgRolesView {
    name: &'static str,
    system_registry: Arc<SystemTablesRegistry>,
}

impl PgRolesView {
    pub fn roles(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self {
            name: "pg_roles",
            system_registry,
        }
    }

    pub fn authid(system_registry: Arc<SystemTablesRegistry>) -> Self {
        Self {
            name: "pg_authid",
            system_registry,
        }
    }
}

impl PgCatalogView for PgRolesView {
    fn name(&self) -> &'static str {
        self.name
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn compute_batch(&self, role: Role) -> Result<RecordBatch, RegistryError> {
        let mut oids = Int64Builder::new();
        let mut rolnames = StringBuilder::new();
        let mut rolsupers = BooleanBuilder::new();
        let mut rolinherits = BooleanBuilder::new();
        let mut rolcreateroles = BooleanBuilder::new();
        let mut rolcreatedbs = BooleanBuilder::new();
        let mut rolcanlogins = BooleanBuilder::new();
        let mut rolreplications = BooleanBuilder::new();
        let mut rolbypassrlss = BooleanBuilder::new();

        // In pg_authid, only admin roles see role rows; credentials (rolpassword) are never
        // exposed.
        if self.name == "pg_authid" && !is_admin_role(role) {
            return RecordBatch::try_new(
                self.schema(),
                vec![
                    Arc::new(oids.finish()) as ArrayRef,
                    Arc::new(rolnames.finish()) as ArrayRef,
                    Arc::new(rolsupers.finish()) as ArrayRef,
                    Arc::new(rolinherits.finish()) as ArrayRef,
                    Arc::new(rolcreateroles.finish()) as ArrayRef,
                    Arc::new(rolcreatedbs.finish()) as ArrayRef,
                    Arc::new(rolcanlogins.finish()) as ArrayRef,
                    Arc::new(rolreplications.finish()) as ArrayRef,
                    Arc::new(rolbypassrlss.finish()) as ArrayRef,
                ],
            )
            .map_err(|error| {
                RegistryError::Other(format!("failed to build {}: {error}", self.name))
            });
        }

        if let Ok(users) = self.system_registry.users().list_users() {
            for user in users {
                if user.deleted_at.is_some() {
                    continue;
                }
                let username = user.user_id.as_str();
                let is_super = matches!(user.role, Role::System | Role::Dba);
                let is_admin = is_admin_role(user.role);
                oids.append_value(stable_oid(&["role", username]));
                rolnames.append_value(username);
                rolsupers.append_value(is_super);
                rolinherits.append_value(true);
                rolcreateroles.append_value(is_admin);
                rolcreatedbs.append_value(is_admin);
                rolcanlogins.append_value(true);
                rolreplications.append_value(false);
                rolbypassrlss.append_value(is_super);
            }
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(oids.finish()) as ArrayRef,
                Arc::new(rolnames.finish()) as ArrayRef,
                Arc::new(rolsupers.finish()) as ArrayRef,
                Arc::new(rolinherits.finish()) as ArrayRef,
                Arc::new(rolcreateroles.finish()) as ArrayRef,
                Arc::new(rolcreatedbs.finish()) as ArrayRef,
                Arc::new(rolcanlogins.finish()) as ArrayRef,
                Arc::new(rolreplications.finish()) as ArrayRef,
                Arc::new(rolbypassrlss.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build {}: {error}", self.name)))
    }
}
