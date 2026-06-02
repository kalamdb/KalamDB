//! System.users table provider
//!
//! This module provides a DataFusion TableProvider implementation for the system.users table.
//! Uses `IndexedEntityStore` for automatic secondary index management.
//!
//! ## Indexes
//!
//! The users table has two secondary indexes (managed automatically):
//!
//! 1. **UserRoleIndex** - Query users by role
//!    - Key: `{role}:{user_id}`
//!    - Enables: "All users with role 'admin'"

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use datafusion::arrow::{array::RecordBatch, datatypes::SchemaRef};
use kalamdb_commons::{models::rows::SystemTableRow, AuthType, Role, UserId};
use kalamdb_store::{entity_store::EntityStore, IndexedEntityStore, StorageBackend};
use parking_lot::RwLock;

use super::users_indexes::create_users_indexes;
use crate::{
    error::{SystemError, SystemResultExt},
    providers::{
        base::{system_rows_to_batch, IndexedProviderDefinition},
        users::models::User,
    },
    system_row_mapper::{model_to_system_row, system_row_to_model},
    SystemTable,
};

/// Type alias for the indexed users store
pub type UsersStore = IndexedEntityStore<UserId, SystemTableRow>;

const USER_ROLE_INDEX: usize = 0;
const USER_EMAIL_INDEX: usize = 1;
const PRIVILEGED_ROLE_CACHE_INITIAL_CAPACITY: usize = 32;

/// System.users table provider using IndexedEntityStore for automatic index management.
///
/// All insert/update/delete operations automatically maintain secondary indexes
/// using RocksDB's atomic WriteBatch - no manual index management needed.
#[derive(Clone)]
pub struct UsersTableProvider {
    store: UsersStore,
    privileged_roles: Arc<RwLock<HashMap<UserId, Role>>>,
}

impl UsersTableProvider {
    /// Create a new users table provider with automatic index management.
    ///
    /// # Arguments
    /// * `backend` - Storage backend (RocksDB or mock)
    ///
    /// # Returns
    /// A new UsersTableProvider instance with indexes configured
    pub fn new(backend: Arc<dyn StorageBackend>) -> Self {
        let store = IndexedEntityStore::new(
            backend,
            SystemTable::Users.column_family_name().expect("Users is a table, not a view"),
            create_users_indexes(),
        );
        let provider = Self {
            store,
            privileged_roles: Arc::new(RwLock::new(HashMap::with_capacity(
                PRIVILEGED_ROLE_CACHE_INITIAL_CAPACITY,
            ))),
        };
        provider
            .refresh_role_caches()
            .expect("failed to initialize system.users role caches from the role index");
        provider
    }

    /// Create a new user.
    ///
    /// Indexes are automatically maintained via `IndexedEntityStore`.
    ///
    /// # Arguments
    /// * `user` - The user to create
    ///
    /// # Returns
    /// Result indicating success or failure
    pub fn create_user(&self, user: User) -> Result<(), SystemError> {
        if let Some(existing_row) = self.store.get(&user.user_id)? {
            let existing_user = Self::decode_user_row(&existing_row)?;
            if existing_user.deleted_at.is_none() {
                return Err(SystemError::AlreadyExists(format!(
                    "User '{}' already exists",
                    user.user_id
                )));
            }

            // Reactivate soft-deleted user rows while preserving index consistency.
            let row = Self::encode_user_row(&user)?;
            self.store
                .update_with_old(&user.user_id, Some(&existing_row), &row)
                .into_system_error("reactivate user error")?;
            self.update_role_caches_for_user(&user);
            return Ok(());
        }

        // Insert user - indexes are managed automatically
        let row = Self::encode_user_row(&user)?;
        self.store.insert(&user.user_id, &row).into_system_error("insert user error")?;
        self.update_role_caches_for_user(&user);
        Ok(())
    }

    /// Update an existing user.
    ///
    /// Indexes are automatically maintained via `IndexedEntityStore`.
    /// Stale index entries are removed and new ones added atomically.
    ///
    /// # Arguments
    /// * `user` - The updated user data
    ///
    /// # Returns
    /// Result indicating success or failure
    pub fn update_user(&self, user: User) -> Result<(), SystemError> {
        // Check if user exists
        let existing = self.store.get(&user.user_id)?;
        if existing.is_none() {
            return Err(SystemError::NotFound(format!("User not found: {}", user.user_id)));
        }

        let existing_row = existing.unwrap();
        let existing_user = Self::decode_user_row(&existing_row)?;

        if existing_user.user_id.is_admin() && user.role != existing_user.role {
            return Err(SystemError::InvalidOperation(
                "Root user role cannot be changed".to_string(),
            ));
        }

        // Use update_with_old for efficiency (we already have old entity)
        let new_row = Self::encode_user_row(&user)?;
        self.store
            .update_with_old(&user.user_id, Some(&existing_row), &new_row)
            .into_system_error("update user error")?;
        self.update_role_caches_for_user(&user);
        Ok(())
    }

    /// Soft delete a user (sets deleted_at timestamp).
    ///
    /// # Arguments
    /// * `user_id` - The ID of the user to delete
    ///
    /// # Returns
    /// Result indicating success or failure
    pub fn delete_user(&self, user_id: &UserId) -> Result<(), SystemError> {
        if user_id.is_admin() {
            return Err(SystemError::InvalidOperation(
                "Root user cannot be removed from the system".to_string(),
            ));
        }

        // Get existing user
        let mut user = self
            .store
            .get(user_id)?
            .map(|row| Self::decode_user_row(&row))
            .transpose()?
            .ok_or_else(|| SystemError::NotFound(format!("User not found: {}", user_id)))?;

        // Set deleted_at timestamp (soft delete)
        user.deleted_at = Some(chrono::Utc::now().timestamp_millis());

        // Update user with deleted_at
        let row = Self::encode_user_row(&user)?;
        self.store.update(user_id, &row).into_system_error("update user error")?;
        self.update_role_caches_for_user(&user);
        Ok(())
    }

    /// Get a user by ID.
    ///
    /// # Arguments
    /// * `user_id` - The user ID to lookup
    ///
    /// # Returns
    /// Option<User> if found, None otherwise
    pub fn get_user_by_id(&self, user_id: &UserId) -> Result<Option<User>, SystemError> {
        let row = self.store.get(user_id)?;
        row.map(|value| Self::decode_user_row(&value)).transpose()
    }

    /// Get any active user account by email address.
    ///
    /// This includes both regular users and pending invites, and skips soft-deleted rows.
    pub fn get_active_user_by_email(&self, email: &str) -> Result<Option<User>, SystemError> {
        let normalized_email = normalize_invite_email(email);
        if normalized_email.is_empty() {
            return Ok(None);
        }

        let prefix = format!("{}:", normalized_email);
        let user_ids = self
            .store
            .scan_index_keys_iter(USER_EMAIL_INDEX, Some(prefix.as_bytes()), None)
            .into_system_error("scan user email index error")?;

        for user_id in user_ids {
            let user_id = user_id.into_system_error("decode user email index key error")?;
            let Some(user) = self.get_user_by_id(&user_id)? else {
                continue;
            };

            if user.deleted_at.is_none() {
                return Ok(Some(user));
            }
        }

        Ok(None)
    }

    /// Get the active pending OIDC invite for an email address.
    ///
    /// Returns the newest non-deleted, non-expired invite when multiple historical
    /// rows exist for the same email.
    pub fn get_active_oidc_invite_by_email(
        &self,
        email: &str,
        now_millis: i64,
    ) -> Result<Option<User>, SystemError> {
        let normalized_email = normalize_invite_email(email);
        if normalized_email.is_empty() {
            return Ok(None);
        }

        let prefix = format!("{}:invite:", normalized_email);
        let invite_ids = self
            .store
            .scan_index_keys_iter(USER_EMAIL_INDEX, Some(prefix.as_bytes()), None)
            .into_system_error("scan user email index error")?;

        let mut newest_invite: Option<User> = None;
        for invite_id in invite_ids {
            let invite_id = invite_id.into_system_error("decode user email index key error")?;
            let Some(invite) = self.get_user_by_id(&invite_id)? else {
                continue;
            };

            if invite.auth_type != AuthType::OidcInvite || invite.deleted_at.is_some() {
                continue;
            }
            if !invite.invite_expires_at.is_some_and(|expires_at| expires_at >= now_millis) {
                continue;
            }

            let should_replace = newest_invite
                .as_ref()
                .is_none_or(|existing| invite.created_at > existing.created_at);
            if should_replace {
                newest_invite = Some(invite);
            }
        }

        Ok(newest_invite)
    }

    /// Return the target role used by hot-path impersonation checks.
    ///
    /// Service, DBA, and system user IDs are tracked in a small in-memory map.
    /// Soft-deleted privileged IDs stay classified by their persisted role so
    /// deletion cannot downgrade a privileged ID into regular-user permissions.
    /// User IDs absent from that map are treated as regular users, avoiding a
    /// per-request lookup against `system.users` for high-cardinality users.
    #[inline]
    pub fn role_for_impersonation_target(&self, user_id: &UserId) -> Role {
        self.privileged_roles.read().get(user_id).copied().unwrap_or(Role::User)
    }

    /// Helper to create RecordBatch from users
    fn create_batch(
        &self,
        users: Vec<(UserId, SystemTableRow)>,
    ) -> Result<RecordBatch, SystemError> {
        let rows = users.into_iter().map(|(_, row)| row).collect();
        system_rows_to_batch(&Self::schema(), rows)
    }

    /// Scan all users and return as RecordBatch
    pub fn scan_all_users(&self) -> Result<RecordBatch, SystemError> {
        let users = self.store.scan_all_typed(None, None, None)?;
        self.create_batch(users)
    }

    fn refresh_role_caches(&self) -> Result<(), SystemError> {
        let mut roles = HashMap::with_capacity(PRIVILEGED_ROLE_CACHE_INITIAL_CAPACITY);
        for role in [Role::Service, Role::Dba, Role::System] {
            self.load_privileged_role(role, &mut roles)?;
        }
        *self.privileged_roles.write() = roles;
        Ok(())
    }

    fn load_privileged_role(
        &self,
        role: Role,
        roles: &mut HashMap<UserId, Role>,
    ) -> Result<(), SystemError> {
        let prefix = role_index_prefix(role);
        let users = self
            .store
            .scan_index_keys_iter(USER_ROLE_INDEX, Some(prefix.as_slice()), None)
            .into_system_error("scan user role index error")?;

        for user_id in users {
            let user_id = user_id.into_system_error("decode user role index key error")?;
            if let Some(user) = self.get_user_by_id(&user_id)? {
                if is_impersonation_privileged_role(user.role) {
                    roles.insert(user.user_id.clone(), user.role);
                }
            }
        }

        Ok(())
    }

    fn update_role_caches_for_user(&self, user: &User) {
        let mut roles = self.privileged_roles.write();
        if is_impersonation_privileged_role(user.role) {
            roles.insert(user.user_id.clone(), user.role);
        } else {
            roles.remove(&user.user_id);
        }
    }

    fn encode_user_row(user: &User) -> Result<SystemTableRow, SystemError> {
        model_to_system_row(user, &User::definition())
    }

    fn decode_user_row(row: &SystemTableRow) -> Result<User, SystemError> {
        system_row_to_model(row, &User::definition())
    }
}

#[inline]
fn is_impersonation_privileged_role(role: Role) -> bool {
    matches!(role, Role::Service | Role::Dba | Role::System)
}

fn role_index_prefix(role: Role) -> Vec<u8> {
    format!("{}:", role.as_str()).into_bytes()
}

fn normalize_invite_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

crate::impl_system_table_provider_metadata!(
    indexed,
    provider = UsersTableProvider,
    key = UserId,
    table_name = SystemTable::Users.table_name(),
    primary_key_column = "user_id",
    parse_key = |value| Some(UserId::new(value)),
    schema = User::definition().to_arrow_schema().expect("failed to build users schema")
);

crate::impl_indexed_system_table_provider!(
    provider = UsersTableProvider,
    key = UserId,
    value = SystemTableRow,
    store = store,
    definition = provider_definition,
    build_batch = create_batch
);

#[cfg(test)]
mod tests {
    use datafusion::datasource::TableProvider;
    use kalamdb_commons::{AuthType, Role, StorageId};
    use kalamdb_store::test_utils::InMemoryBackend;

    use super::*;

    fn create_test_provider() -> UsersTableProvider {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
        UsersTableProvider::new(backend)
    }

    fn create_test_user(id: &str) -> User {
        User {
            user_id: UserId::new(id),
            password_hash: "hashed_password".to_string(),
            role: Role::User,
            name: None,
            email: Some(format!("{}@example.com", id)),
            auth_type: AuthType::Password,
            auth_data: None,
            storage_mode: crate::providers::storages::models::StorageMode::Table,
            storage_id: Some(StorageId::local()),
            failed_login_attempts: 0,
            locked_until: None,
            last_login_at: None,
            created_at: 1000,
            updated_at: 1000,
            last_seen: None,
            deleted_at: None,
            invite_expires_at: None,
            invited_by: None,
        }
    }

    #[test]
    fn test_create_and_get_user() {
        let provider = create_test_provider();
        let user = create_test_user("user1");

        // Create user
        provider.create_user(user.clone()).unwrap();

        // Get by ID
        let retrieved = provider.get_user_by_id(&UserId::new("user1")).unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.email, Some("user1@example.com".to_string()));
    }

    #[test]
    fn test_update_user() {
        let provider = create_test_provider();
        let user = create_test_user("user1");

        provider.create_user(user).unwrap();

        // Update user
        let mut updated = provider.get_user_by_id(&UserId::new("user1")).unwrap().unwrap();
        updated.email = Some("newemail@example.com".to_string());
        updated.updated_at = 2000;

        provider.update_user(updated).unwrap();

        // Verify update
        let retrieved = provider.get_user_by_id(&UserId::new("user1")).unwrap().unwrap();
        assert_eq!(retrieved.email, Some("newemail@example.com".to_string()));
        assert_eq!(retrieved.updated_at, 2000);
    }

    #[test]
    fn test_delete_user() {
        let provider = create_test_provider();
        let user = create_test_user("user1");

        provider.create_user(user).unwrap();
        provider.delete_user(&UserId::new("user1")).unwrap();

        // Verify deleted_at is set
        let retrieved = provider.get_user_by_id(&UserId::new("user1")).unwrap().unwrap();
        assert!(retrieved.deleted_at.is_some());
    }

    #[test]
    fn test_impersonation_role_cache_tracks_privileged_users() {
        let provider = create_test_provider();
        let user_id = UserId::new("service1");
        let mut user = create_test_user(user_id.as_str());

        assert_eq!(provider.role_for_impersonation_target(&user_id), Role::User);

        user.role = Role::Service;
        provider.create_user(user.clone()).unwrap();
        assert_eq!(provider.role_for_impersonation_target(&user_id), Role::Service);

        user.role = Role::Dba;
        provider.update_user(user.clone()).unwrap();
        assert_eq!(provider.role_for_impersonation_target(&user_id), Role::Dba);

        user.role = Role::User;
        provider.update_user(user).unwrap();
        assert_eq!(provider.role_for_impersonation_target(&user_id), Role::User);
    }

    #[test]
    fn test_impersonation_role_cache_keeps_deleted_privileged_users_classified() {
        let provider = create_test_provider();
        let user_id = UserId::new("dba1");
        let mut user = create_test_user(user_id.as_str());
        user.role = Role::Dba;

        provider.create_user(user).unwrap();
        assert_eq!(provider.role_for_impersonation_target(&user_id), Role::Dba);

        provider.delete_user(&user_id).unwrap();
        assert_eq!(provider.role_for_impersonation_target(&user_id), Role::Dba);
    }

    #[test]
    fn test_impersonation_role_cache_loads_from_role_index_on_startup() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
        let provider = UsersTableProvider::new(backend.clone());

        let mut service = create_test_user("service_cached");
        service.role = Role::Service;
        provider.create_user(service).unwrap();

        let mut dba = create_test_user("dba_cached");
        dba.role = Role::Dba;
        provider.create_user(dba).unwrap();

        let user = create_test_user("regular_cached");
        provider.create_user(user).unwrap();

        let reloaded = UsersTableProvider::new(backend);
        assert_eq!(
            reloaded.role_for_impersonation_target(&UserId::new("service_cached")),
            Role::Service
        );
        assert_eq!(reloaded.role_for_impersonation_target(&UserId::new("dba_cached")), Role::Dba);
        assert_eq!(
            reloaded.role_for_impersonation_target(&UserId::new("regular_cached")),
            Role::User
        );
    }

    #[test]
    fn test_scan_all_users() {
        let provider = create_test_provider();

        // Create multiple users
        for i in 1..=3 {
            let user = create_test_user(&format!("user{}", i));
            provider.create_user(user).unwrap();
        }

        // Scan all
        let batch = provider.scan_all_users().unwrap();
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.num_columns(), 18);
    }

    #[test]
    fn test_table_provider_schema() {
        let provider = create_test_provider();
        let schema = provider.schema();
        assert_eq!(schema.fields().len(), 18);
        assert_eq!(schema.field(0).name(), "user_id");
    }

    #[test]
    fn test_get_active_oidc_invite_by_email_uses_email_index() {
        let provider = create_test_provider();
        let mut invite = create_test_user("invite_alice");
        invite.auth_type = AuthType::OidcInvite;
        invite.email = Some("Alice@Example.com".to_string());
        invite.role = Role::Dba;
        invite.invite_expires_at = Some(2_000);

        provider.create_user(invite.clone()).unwrap();

        let found = provider
            .get_active_oidc_invite_by_email("alice@example.com", 1_500)
            .unwrap()
            .expect("active invite should be found");
        assert_eq!(found.user_id, invite.user_id);
        assert_eq!(found.role, Role::Dba);

        assert!(provider
            .get_active_oidc_invite_by_email("alice@example.com", 2_001)
            .unwrap()
            .is_none());
    }
}
