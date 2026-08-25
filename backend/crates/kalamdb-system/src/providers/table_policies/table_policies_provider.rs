use std::{
    collections::HashSet,
    sync::{Arc, OnceLock},
};

use dashmap::DashMap;
use datafusion::arrow::{array::RecordBatch, datatypes::SchemaRef};
use kalamdb_commons::{
    models::rows::SystemTableRow, KSerializable, PolicyId, SystemTable, TableId, TablePolicy,
};
use kalamdb_store::{entity_store::EntityStore, IndexedEntityStore, StorageBackend};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::TablePolicyRecord;
use super::CompiledTablePolicies;
use crate::{
    error::{SystemError, SystemResultExt},
    providers::base::{system_rows_to_batch, IndexedProviderDefinition},
    system_row_mapper::{model_to_system_row, system_row_to_model},
};

pub type TablePoliciesStore = IndexedEntityStore<PolicyId, SystemTableRow>;
type PolicyGenerationsStore = IndexedEntityStore<TableId, PolicyGenerationRecord>;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PolicyGenerationRecord {
    generation: u64,
}

impl KSerializable for PolicyGenerationRecord {}

#[derive(Clone)]
pub struct TablePoliciesTableProvider {
    store: TablePoliciesStore,
    generations: PolicyGenerationsStore,
    /// In-memory generation mirror so hot-path `compiled_for_table` avoids RocksDB.
    generation_cache: Arc<DashMap<TableId, u64>>,
    reverse_dependencies: Arc<DashMap<TableId, HashSet<PolicyId>>>,
    compiled_cache: Arc<DashMap<TableId, Arc<CompiledTablePolicies>>>,
    mutation_lock: Arc<Mutex<()>>,
}

impl TablePoliciesTableProvider {
    pub fn new(backend: Arc<dyn StorageBackend>) -> Self {
        let store = IndexedEntityStore::new(
            backend.clone(),
            SystemTable::TablePolicies
                .column_family_name()
                .expect("TablePolicies is a table, not a view"),
            Vec::new());
        let generations = IndexedEntityStore::new(
            backend,
            "system_table_policy_generations",
            Vec::new());
        let provider = Self {
            store,
            generations,
            generation_cache: Arc::new(DashMap::new()),
            reverse_dependencies: Arc::new(DashMap::new()),
            compiled_cache: Arc::new(DashMap::new()),
            mutation_lock: Arc::new(Mutex::new(())),
        };
        provider.rebuild_reverse_dependencies();
        provider
    }

    pub async fn create_policy(&self, mut policy: TablePolicy) -> Result<TablePolicy, SystemError> {
        let _guard = self.mutation_lock.lock().await;
        if self.get_policy_sync(&policy.policy_id)?.is_some() {
            return Err(SystemError::AlreadyExists(policy.policy_id.to_string()));
        }
        self.ensure_acyclic(&policy)?;
        policy.policy_generation = self.bump_generation(&policy.table_id)?;
        self.write_policy(&policy)?;
        self.register_dependencies(&policy);
        Ok(policy)
    }

    pub async fn ensure_policy(&self, mut policy: TablePolicy) -> Result<TablePolicy, SystemError> {
        let _guard = self.mutation_lock.lock().await;
        if let Some(existing) = self.get_policy_sync(&policy.policy_id)? {
            return Ok(existing);
        }
        self.ensure_acyclic(&policy)?;
        policy.policy_generation = self.bump_generation(&policy.table_id)?;
        self.write_policy(&policy)?;
        self.register_dependencies(&policy);
        Ok(policy)
    }

    pub async fn replace_policy(&self, mut policy: TablePolicy) -> Result<TablePolicy, SystemError> {
        let _guard = self.mutation_lock.lock().await;
        let previous = self
            .get_policy_sync(&policy.policy_id)?
            .ok_or_else(|| SystemError::NotFound(policy.policy_id.to_string()))?;
        self.ensure_acyclic(&policy)?;
        policy.policy_generation = self.bump_generation(&policy.table_id)?;
        self.write_policy(&policy)?;
        self.unregister_dependencies(&previous);
        self.register_dependencies(&policy);
        Ok(policy)
    }

    pub async fn rename_policy(
        &self,
        policy_id: &PolicyId,
        new_name: &str) -> Result<TablePolicy, SystemError> {
        let _guard = self.mutation_lock.lock().await;
        let previous = self
            .get_policy_sync(policy_id)?
            .ok_or_else(|| SystemError::NotFound(policy_id.to_string()))?;
        let new_id = PolicyId::new(previous.table_id.clone(), new_name)
            .map_err(SystemError::InvalidOperation)?;
        if self.get_policy_sync(&new_id)?.is_some() {
            return Err(SystemError::AlreadyExists(new_id.to_string()));
        }

        let mut renamed = previous.clone();
        renamed.policy_id = new_id;
        renamed.policy_name = new_name.to_string();
        renamed.policy_generation = self.bump_generation(&renamed.table_id)?;
        self.write_policy(&renamed)?;
        if let Err(error) = self.store.delete(policy_id) {
            let _ = self.store.delete(&renamed.policy_id);
            return Err(SystemError::Storage(format!("rename table policy: {error}")));
        }
        self.unregister_dependencies(&previous);
        self.register_dependencies(&renamed);
        Ok(renamed)
    }

    pub async fn get_policy(&self, policy_id: &PolicyId) -> Result<Option<TablePolicy>, SystemError> {
        self.get_policy_sync(policy_id)
    }

    pub fn list_for_table(&self, table_id: &TableId) -> Result<Vec<TablePolicy>, SystemError> {
        let mut policies = self
            .list_all()?
            .into_iter()
            .filter(|policy| &policy.table_id == table_id)
            .collect::<Vec<_>>();
        policies.sort_by(|left, right| left.policy_name.cmp(&right.policy_name));
        Ok(policies)
    }

    pub fn compiled_for_table(
        &self,
        table_id: &TableId,
        schema_generation: u64) -> Result<Arc<CompiledTablePolicies>, SystemError> {
        let policy_generation = self.policy_generation(table_id)?;
        if let Some(compiled) = self.compiled_cache.get(table_id) {
            if compiled.policy_generation == policy_generation
                && compiled.schema_generation == schema_generation
            {
                return Ok(Arc::clone(compiled.value()));
            }
        }

        let policies = self.list_for_table(table_id)?.into();
        let compiled = Arc::new(CompiledTablePolicies {
            table_id: table_id.clone(),
            policy_generation,
            schema_generation,
            policies,
        });
        self.compiled_cache.insert(table_id.clone(), Arc::clone(&compiled));
        Ok(compiled)
    }

    pub async fn delete_policy(
        &self,
        policy_id: &PolicyId,
        if_exists: bool) -> Result<(), SystemError> {
        let _guard = self.mutation_lock.lock().await;
        let Some(policy) = self.get_policy_sync(policy_id)? else {
            return if if_exists {
                Ok(())
            } else {
                Err(SystemError::NotFound(policy_id.to_string()))
            };
        };
        self.bump_generation(&policy.table_id)?;
        self.unregister_dependencies(&policy);
        self.store
            .delete(policy_id)
            .into_system_error("delete table policy")
    }

    pub async fn delete_for_table(&self, table_id: &TableId) -> Result<usize, SystemError> {
        let _guard = self.mutation_lock.lock().await;
        let policies = self.list_for_table(table_id)?;
        if policies.is_empty() {
            return Ok(0);
        }
        self.bump_generation(table_id)?;
        for policy in &policies {
            self.unregister_dependencies(policy);
        }
        let keys = policies.iter().map(|policy| policy.policy_id.clone()).collect::<Vec<_>>();
        self.store
            .delete_batch(&keys)
            .into_system_error("delete table policies")?;
        Ok(keys.len())
    }

    pub fn policy_generation(&self, table_id: &TableId) -> Result<u64, SystemError> {
        if let Some(generation) = self.generation_cache.get(table_id) {
            return Ok(*generation);
        }
        let generation = self
            .generations
            .get(table_id)
            .into_system_error("read table policy generation")?
            .map_or(0, |record| record.generation);
        self.generation_cache.insert(table_id.clone(), generation);
        Ok(generation)
    }

    pub fn dependent_policies(&self, relation_table: &TableId) -> Result<Vec<PolicyId>, SystemError> {
        let mut policies = self
            .reverse_dependencies
            .get(relation_table)
            .map(|entry| entry.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        policies.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        Ok(policies)
    }

    fn bump_generation(&self, table_id: &TableId) -> Result<u64, SystemError> {
        let generation = self.policy_generation(table_id)?.saturating_add(1);
        self.generations
            .insert(table_id, &PolicyGenerationRecord { generation })
            .into_system_error("write table policy generation")?;
        self.generation_cache.insert(table_id.clone(), generation);
        // Drop stale compiled bundles immediately; next bind rebuilds.
        self.compiled_cache.remove(table_id);
        Ok(generation)
    }

    fn write_policy(&self, policy: &TablePolicy) -> Result<(), SystemError> {
        let record = TablePolicyRecord::from(policy.clone());
        let row = model_to_system_row(&record, &TablePolicyRecord::definition())?;
        self.store
            .insert(&policy.policy_id, &row)
            .into_system_error("write table policy")
    }

    fn get_policy_sync(&self, policy_id: &PolicyId) -> Result<Option<TablePolicy>, SystemError> {
        self.store
            .get(policy_id)
            .into_system_error("read table policy")?
            .map(|row| {
                system_row_to_model::<TablePolicyRecord>(&row, &TablePolicyRecord::definition())
                    .map(TablePolicy::from)
            })
            .transpose()
    }

    fn list_all(&self) -> Result<Vec<TablePolicy>, SystemError> {
        self.store
            .scan_all_typed(None, None, None)
            .into_system_error("list table policies")?
            .into_iter()
            .map(|(_, row)| {
                system_row_to_model::<TablePolicyRecord>(&row, &TablePolicyRecord::definition())
                    .map(TablePolicy::from)
            })
            .collect()
    }

    fn register_dependencies(&self, policy: &TablePolicy) {
        for dependency in policy_dependencies(policy) {
            self.reverse_dependencies
                .entry(dependency)
                .or_default()
                .insert(policy.policy_id.clone());
        }
    }

    fn unregister_dependencies(&self, policy: &TablePolicy) {
        for dependency in policy_dependencies(policy) {
            if let Some(mut policies) = self.reverse_dependencies.get_mut(&dependency) {
                policies.remove(&policy.policy_id);
                if policies.is_empty() {
                    drop(policies);
                    self.reverse_dependencies.remove(&dependency);
                }
            }
        }
    }

    fn rebuild_reverse_dependencies(&self) {
        match self.list_all() {
            Ok(policies) => {
                for policy in &policies {
                    self.register_dependencies(policy);
                }
            },
            Err(error) => log::error!("failed rebuilding policy dependency registry: {error}"),
        }
    }

    fn ensure_acyclic(&self, candidate: &TablePolicy) -> Result<(), SystemError> {
        let policies = self
            .list_all()?
            .into_iter()
            .filter(|policy| policy.policy_id != candidate.policy_id)
            .chain(std::iter::once(candidate.clone()))
            .collect::<Vec<_>>();
        let mut pending = policy_dependencies(candidate);
        let mut visited = HashSet::new();

        while let Some(table_id) = pending.pop() {
            if table_id == candidate.table_id {
                return Err(SystemError::InvalidOperation(format!(
                    "policy dependency cycle detected for {}",
                    candidate.table_id
                )));
            }
            if !visited.insert(table_id.clone()) {
                continue;
            }
            pending.extend(
                policies
                    .iter()
                    .filter(|policy| policy.table_id == table_id)
                    .flat_map(policy_dependencies));
        }
        Ok(())
    }

    fn build_batch_from_pairs(
        &self,
        pairs: Vec<(PolicyId, SystemTableRow)>) -> Result<RecordBatch, SystemError> {
        system_rows_to_batch(&Self::schema(), pairs.into_iter().map(|(_, row)| row).collect())
    }
}

fn policy_dependencies(policy: &TablePolicy) -> Vec<TableId> {
    policy
        .using_program
        .iter()
        .chain(policy.check_program.iter())
        .filter_map(|program| match program {
            kalamdb_commons::PolicyProgram::AuthorizationRelation(relation) => {
                Some(relation.dependencies.as_slice())
            },
            kalamdb_commons::PolicyProgram::RowLocal { .. } => None,
        })
        .flatten()
        .cloned()
        .collect()
}

crate::impl_system_table_provider_metadata!(
    indexed,
    provider = TablePoliciesTableProvider,
    key = PolicyId,
    table_name = SystemTable::TablePolicies.table_name(),
    primary_key_column = "policy_id",
    parse_key = |value| value
        .rsplit_once(':')
        .and_then(|(table_id, policy_name)| PolicyId::from_parts(table_id, policy_name).ok()),
    schema = TablePolicyRecord::definition()
        .to_arrow_schema()
        .expect("failed to build table policies schema")
);

crate::impl_indexed_system_table_provider!(
    provider = TablePoliciesTableProvider,
    key = PolicyId,
    value = SystemTableRow,
    store = store,
    definition = provider_definition,
    build_batch = build_batch_from_pairs
);
