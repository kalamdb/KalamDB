//! Unified ManifestService for batch file metadata tracking.
//!
//! Provides manifest.json management with one read-through access path:
//! 1. Memory cache for active manifest entries
//! 2. RocksDB manifest copy for local crash recovery and fast restart
//! 3. Cold storage manifest.json for portable storage metadata
//!
//! Shared-scope manifest mutations update the in-process cache and the RocksDB manifest copy.
//! User-scoped manifest mutations update RocksDB only so high-cardinality user workloads do not
//! grow process memory with one manifest per user.
//! Cold storage `manifest.json` is written only by explicit persist/flush paths after the
//! corresponding Parquet or metadata change is ready to commit.
//!
//! Key type: (TableId, Option<UserId>) for type-safe cache access.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use dashmap::{DashMap, DashSet};
use kalamdb_commons::{ids::SeqId, ManifestId, TableId, UserId};
use kalamdb_configs::ManifestCacheSettings;
use kalamdb_filestore::{FilestoreError, StorageCached, StorageRegistry};
use kalamdb_store::{StorageBackend, StorageError};
use kalamdb_system::{
    providers::ManifestTableProvider, FileSubfolderState, Manifest, ManifestCacheEntry,
    ManifestService as ManifestServiceTrait, SchemaRegistry as SchemaRegistryTrait,
    SegmentMetadata, SyncState,
};
use kalamdb_tables::TableError;
use log::{debug, info, warn};

const MAX_MANIFEST_SCAN_LIMIT: usize = 100000;

/// Unified ManifestService with memory + RocksDB persistence + cold storage.
///
/// Architecture:
/// - Memory cache: process-local acceleration layer for hot manifest entries
/// - RocksDB: local persistent manifest index for crash recovery and fast restart
/// - Cold store: manifest.json files in filestore (S3/local filesystem)
pub struct ManifestService {
    /// Process-local memory cache for shared-scope active manifests.
    ///
    /// User-scoped manifests intentionally stay out of this cache; RocksDB is their hot manifest
    /// layer to keep memory bounded for high-cardinality user tables.
    memory_cache: DashMap<ManifestId, Arc<ManifestCacheEntry>>,

    /// Per-scope flush serialization guards.
    ///
    /// Flushes for the same table/user scope must not race batch-number allocation or manifest
    /// persistence, otherwise later flushes can overwrite earlier segment metadata.
    flush_scope_locks: DashMap<ManifestId, Arc<Mutex<()>>>,

    /// Per-scope compaction guards.
    ///
    /// Compaction intentionally does not hold the flush manifest lock while it reads and writes
    /// Parquet data. This guard prevents duplicate compaction jobs for the same scope from doing
    /// duplicate heavy work; the final manifest swap is still protected by `flush_scope_locks`.
    active_compactions: Arc<DashSet<ManifestId>>,

    /// Provider wrapping the store
    provider: Arc<ManifestTableProvider>,

    /// Configuration settings
    config: ManifestCacheSettings,

    /// Optional registries for path/object store resolution.
    ///
    /// In production these are injected via `new_with_registries()`
    schema_registry:  Option<Arc<dyn SchemaRegistryTrait<Error = TableError>>>,
    storage_registry: Option<Arc<StorageRegistry>>,
}

pub(crate) struct ManifestCompactionScopeGuard {
    active_compactions: Arc<DashSet<ManifestId>>,
    manifest_id:        ManifestId,
}

impl Drop for ManifestCompactionScopeGuard {
    fn drop(&mut self) {
        self.active_compactions.remove(&self.manifest_id);
    }
}

impl ManifestService {
    pub(crate) fn try_begin_compaction_scope(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Option<ManifestCompactionScopeGuard> {
        let manifest_id = Self::manifest_id(table_id, user_id);
        if !self.active_compactions.insert(manifest_id.clone()) {
            return None;
        }

        Some(ManifestCompactionScopeGuard {
            active_compactions: Arc::clone(&self.active_compactions),
            manifest_id,
        })
    }

    pub(crate) fn with_flush_scope_lock<T, F>(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
        f: F,
    ) -> Result<T, StorageError>
    where
        F: FnOnce() -> Result<T, StorageError>,
    {
        let scope_lock = self.flush_scope_lock(table_id, user_id);
        let _guard = scope_lock.lock().map_err(|_| {
            StorageError::Other(format!(
                "flush scope lock poisoned for {} (user_id={:?})",
                table_id,
                user_id.map(UserId::as_str)
            ))
        })?;

        f()
    }

    fn delete_manifest_ids(&self, keys: Vec<ManifestId>) -> Result<usize, StorageError> {
        let deleted = keys.len();
        if deleted > 0 {
            self.provider.delete_manifest_ids_batch(&keys)?;
            for key in keys {
                self.memory_cache.remove(&key);
            }
            kalamdb_observability::decrement_manifest_cache_rocksdb_entries(deleted);
            self.publish_manifest_memory_metrics();
        }
        Ok(deleted)
    }

    fn publish_manifest_memory_metrics(&self) {
        kalamdb_observability::set_manifest_cache_memory_entries(self.memory_cache.len());
    }

    /// Create a new ManifestService
    pub fn new(provider: Arc<ManifestTableProvider>, config: ManifestCacheSettings) -> Self {
        let service = Self {
            memory_cache: DashMap::with_capacity(config.max_entries.min(1024)),
            flush_scope_locks: DashMap::with_capacity(128),
            active_compactions: Arc::new(DashSet::with_capacity(128)),
            provider,
            config,
            schema_registry: None,
            storage_registry: None,
        };
        if let Ok(entries) = service.provider.count_entries() {
            kalamdb_observability::initialize_manifest_cache_rocksdb_entries(entries);
        }
        service.publish_manifest_memory_metrics();
        service
    }

    /// Create a ManifestService with injected registries (compat helper for tests).
    pub fn new_with_registries(
        backend: Arc<dyn StorageBackend>,
        _base_path: String,
        config: ManifestCacheSettings,
        schema_registry: Arc<dyn SchemaRegistryTrait<Error = TableError>>,
        storage_registry: Arc<StorageRegistry>,
    ) -> Self {
        let provider = Arc::new(ManifestTableProvider::new(backend));
        let mut service = Self::new(provider, config);
        service.set_schema_registry(schema_registry);
        service.set_storage_registry(storage_registry);
        service
    }

    /// Set SchemaRegistry (break circular dependency)
    pub fn set_schema_registry(
        &mut self,
        registry: Arc<dyn SchemaRegistryTrait<Error = TableError>>,
    ) {
        self.schema_registry = Some(registry);
    }

    /// Set StorageRegistry
    pub fn set_storage_registry(&mut self, registry: Arc<StorageRegistry>) {
        self.storage_registry = Some(registry);
    }

    // Internal helper to get registries (panics if not set in production flows)
    fn get_schema_registry(&self) -> &Arc<dyn SchemaRegistryTrait<Error = TableError>> {
        self.schema_registry
            .as_ref()
            .expect("SchemaRegistry not initialized in ManifestService")
    }

    fn get_storage_registry(&self) -> &Arc<StorageRegistry> {
        self.storage_registry
            .as_ref()
            .expect("StorageRegistry not initialized in ManifestService")
    }

    // ========== Cache Operations ==========

    /// Get a manifest entry through the canonical read path.
    ///
    /// Lookup order is memory -> RocksDB -> storage manifest.json. When a lower
    /// layer has the manifest, this method hydrates every faster layer above it.
    pub fn get_or_load(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<Option<Arc<ManifestCacheEntry>>, StorageError> {
        let manifest_id = Self::manifest_id(table_id, user_id);

        if let Some(entry) = self.memory_cache.get(&manifest_id) {
            return Ok(Some(Arc::clone(entry.value())));
        }

        match self.provider.get_cache_entry(&manifest_id) {
            Ok(Some(entry)) => {
                let entry = Arc::new(entry);
                self.insert_memory_entry(manifest_id, Arc::clone(&entry));
                Ok(Some(entry))
            },
            Ok(None) => self.load_from_storage_and_hydrate(table_id, user_id),
            Err(StorageError::SerializationError(err)) => {
                warn!(
                    "Manifest cache entry corrupted for key {}: {} (dropping)",
                    manifest_id.as_str(),
                    err
                );
                self.memory_cache.remove(&manifest_id);
                let _ = self.provider.delete_cache_entry(&manifest_id);
                self.load_from_storage_and_hydrate(table_id, user_id)
            },
            Err(err) => Err(err),
        }
    }

    /// Async version of get_or_load to avoid blocking the tokio runtime for RocksDB reads.
    pub async fn get_or_load_async(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<Option<Arc<ManifestCacheEntry>>, StorageError> {
        let manifest_id = Self::manifest_id(table_id, user_id);

        if let Some(entry) = self.memory_cache.get(&manifest_id) {
            return Ok(Some(Arc::clone(entry.value())));
        }

        match self.provider.get_cache_entry_async(&manifest_id).await {
            Ok(Some(entry)) => {
                let entry = Arc::new(entry);
                self.insert_memory_entry(manifest_id, Arc::clone(&entry));
                Ok(Some(entry))
            },
            Ok(None) => self.load_from_storage_and_hydrate_async(table_id, user_id).await,
            Err(StorageError::SerializationError(err)) => {
                warn!(
                    "Manifest cache entry corrupted for key {}: {} (dropping)",
                    manifest_id.as_str(),
                    err
                );
                self.memory_cache.remove(&manifest_id);
                let _ = self.provider.delete_cache_entry_async(&manifest_id).await;
                self.load_from_storage_and_hydrate_async(table_id, user_id).await
            },
            Err(err) => Err(err),
        }
    }

    /// Count all cached manifest entries.
    pub fn count(&self) -> Result<usize, StorageError> {
        self.provider.count_entries()
    }

    /// Update manifest cache after successful flush.
    ///
    /// Sets sync_state to InSync. Index automatically updated by IndexedEntityStore.
    pub fn update_after_flush(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
        manifest: &Manifest,
        etag: Option<String>,
    ) -> Result<(), StorageError> {
        // Index automatically updated by IndexedEntityStore when state changes
        self.upsert_cache_entry(table_id, user_id, manifest, etag, SyncState::InSync)
    }

    /// Stage manifest metadata in the cache before the first flush writes manifest.json to disk.
    pub fn stage_before_flush(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
        manifest: &Manifest,
    ) -> Result<(), StorageError> {
        self.upsert_cache_entry(table_id, user_id, manifest, None, SyncState::InSync)
    }

    /// Mark a cache entry as stale.
    pub fn mark_as_stale(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<(), StorageError> {
        self.update_cached_entry(table_id, user_id, |entry| entry.mark_stale())
    }

    /// Mark a cache entry as syncing (flush in progress).
    pub fn mark_syncing(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<(), StorageError> {
        self.with_flush_scope_lock(table_id, user_id, || {
            self.mark_syncing_in_locked_scope(table_id, user_id)
        })
    }

    pub(crate) fn mark_syncing_in_locked_scope(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<(), StorageError> {
        self.update_cached_entry(table_id, user_id, |entry| entry.mark_syncing())
    }

    /// Mark a cache entry as having pending writes (hot data not yet flushed to cold storage).
    ///
    /// This should be called after any write operation (INSERT, UPDATE, DELETE) to indicate
    /// that the RocksDB hot store has data that needs to be flushed to Parquet cold storage.
    /// The sync_state will transition from InSync to PendingWrite.
    ///
    /// Index automatically updated by IndexedEntityStore for O(1) flush job discovery.
    pub fn mark_pending_write(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<(), StorageError> {
        self.with_flush_scope_lock(table_id, user_id, || {
            self.mark_pending_write_in_locked_scope(table_id, user_id)
        })
    }

    fn mark_pending_write_in_locked_scope(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<(), StorageError> {
        let rocksdb_key = Self::manifest_id(table_id, user_id);

        if !self.should_cache_in_memory(&rocksdb_key)
            && self
                .provider
                .pending_exists(&rocksdb_key)
                .map_err(|e| StorageError::Other(e.to_string()))?
        {
            return Ok(());
        }

        match self.cached_entry_snapshot(&rocksdb_key) {
            Ok(Some(old_entry)) => {
                if old_entry.sync_state == SyncState::PendingWrite {
                    self.insert_memory_entry(rocksdb_key, Arc::new(old_entry));
                    return Ok(());
                }

                let mut new_entry = old_entry.clone();
                new_entry.mark_pending_write();
                self.provider
                    .update_cache_entry_with_old(&rocksdb_key, &old_entry, &new_entry)?;
                self.insert_memory_entry(rocksdb_key, Arc::new(new_entry));

                // Index automatically updated by IndexedEntityStore

                debug!(
                    "Marked manifest entry as pending_write: table={}, user={:?}",
                    table_id,
                    user_id.map(|u| u.as_str())
                );
            },
            Ok(None) => {
                // If no cache entry exists yet, create one with PendingWrite state
                // This shouldn't happen in normal flow since ensure_manifest_ready is called first
                warn!(
                    "mark_pending_write called but no cache entry exists: table={}, user={:?}",
                    table_id,
                    user_id.map(|u| u.as_str())
                );
            },
            Err(StorageError::SerializationError(err)) => {
                warn!(
                    "Manifest cache entry corrupted for key {}: {} (dropping)",
                    rocksdb_key.as_str(),
                    err
                );
                let _ = self.provider.delete_cache_entry(&rocksdb_key);
                self.memory_cache.remove(&rocksdb_key);
            },
            Err(err) => return Err(err),
        }

        Ok(())
    }

    /// Validate freshness of cached entry based on TTL.
    pub fn validate_freshness(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<bool, StorageError> {
        let rocksdb_key = Self::manifest_id(table_id, user_id);

        if let Some(entry) = self.memory_cache.get(&rocksdb_key) {
            let now = chrono::Utc::now().timestamp_millis();
            Ok(!entry.value().is_stale(self.config.ttl_millis(), now))
        } else if let Some(entry) = self.provider.get_cache_entry(&rocksdb_key)? {
            let now = chrono::Utc::now().timestamp_millis();
            Ok(!entry.is_stale(self.config.ttl_millis(), now))
        } else {
            Ok(false)
        }
    }

    /// Invalidate (delete) a cache entry.
    pub fn invalidate(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<(), StorageError> {
        let rocksdb_key = Self::manifest_id(table_id, user_id);
        let existed = matches!(
            self.cached_entry_snapshot(&rocksdb_key),
            Ok(Some(_)) | Err(StorageError::SerializationError(_))
        );
        self.memory_cache.remove(&rocksdb_key);
        self.publish_manifest_memory_metrics();
        let result = self.provider.delete_cache_entry(&rocksdb_key);
        if result.is_ok() && existed {
            kalamdb_observability::decrement_manifest_cache_rocksdb_entries(1);
        }
        result
    }

    /// Invalidate all cache entries for a table (all users + shared).
    pub fn invalidate_table(&self, table_id: &TableId) -> Result<usize, StorageError> {
        // Use table prefix to include ALL scopes (shared + all users)
        let prefix = ManifestId::table_prefix(table_id);
        let keys = self.provider.scan_manifest_ids_with_raw_prefix(
            &prefix,
            None,
            MAX_MANIFEST_SCAN_LIMIT,
        )?;
        let invalidated = self.delete_manifest_ids(keys)?;

        debug!("Invalidated {} manifest cache entries for table {}", invalidated, table_id);

        Ok(invalidated)
    }

    /// Check if a cache key is currently in the memory cache.
    pub fn is_in_hot_cache(&self, table_id: &TableId, user_id: Option<&UserId>) -> bool {
        let rocksdb_key = Self::manifest_id(table_id, user_id);
        self.memory_cache.contains_key(&rocksdb_key)
    }

    /// Check if a cache key string is in memory cache.
    pub fn is_in_hot_cache_by_string(&self, cache_key_str: &str) -> bool {
        let rocksdb_key = ManifestId::from(cache_key_str);
        self.memory_cache.contains_key(&rocksdb_key)
    }

    /// Evict stale manifest entries from RocksDB.
    ///
    /// TODO: Optimize with secondary index on `last_refreshed` timestamp.
    /// Currently scans all manifests - at scale, maintain a BTree/skip-list
    /// or RocksDB secondary index to find stale entries efficiently O(log N)
    /// instead of O(N) full scan.
    pub fn evict_stale_entries(&self, ttl_seconds: i64) -> Result<usize, StorageError> {
        let now = chrono::Utc::now().timestamp_millis();
        let cutoff = now - (ttl_seconds * 1000);
        let entries = self.provider.scan_manifest_entries(MAX_MANIFEST_SCAN_LIMIT)?;

        let delete_keys: Vec<ManifestId> = entries
            .into_iter()
            .filter_map(|(key, entry)| {
                if entry.last_refreshed_millis() < cutoff {
                    Some(key)
                } else {
                    None
                }
            })
            .collect();

        let evicted_count = self.delete_manifest_ids(delete_keys)?;

        info!(
            "Manifest eviction: removed {} stale entries (ttl_seconds={}, cutoff={})",
            evicted_count, ttl_seconds, cutoff
        );

        Ok(evicted_count)
    }

    /// Get cache configuration.
    pub fn config(&self) -> &ManifestCacheSettings {
        &self.config
    }

    // ========== Pending Write Index Operations ==========

    /// Iterator over all manifests with pending writes.
    ///
    /// Uses the pending-write index (index 0) for O(1) discovery.
    pub fn pending_manifest_ids_iter(
        &self,
    ) -> Result<Box<dyn Iterator<Item = Result<ManifestId, StorageError>> + Send + '_>, StorageError>
    {
        let iter = self
            .provider
            .pending_manifest_ids_iter(None, None)
            .map_err(|e| StorageError::Other(e.to_string()))?;

        let mapped = iter.map(|res| res.map_err(|e| StorageError::Other(e.to_string())));
        Ok(Box::new(mapped))
    }

    /// Get all manifests with pending writes.
    ///
    /// Uses the pending-write index (index 0) for O(1) discovery.
    pub fn get_pending_manifests(&self) -> Result<Vec<ManifestId>, StorageError> {
        self.provider
            .pending_manifest_ids()
            .map_err(|e| StorageError::Other(e.to_string()))
    }

    /// Get pending manifests for a specific table.
    pub fn get_pending_for_table(
        &self,
        table_id: &TableId,
    ) -> Result<Vec<ManifestId>, StorageError> {
        self.provider
            .pending_manifest_ids_for_table(table_id)
            .map_err(|e| StorageError::Other(e.to_string()))
    }

    /// Check if a manifest has pending writes.
    pub fn has_pending_writes(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<bool, StorageError> {
        let manifest_id = Self::manifest_id(table_id, user_id);
        self.provider
            .pending_exists(&manifest_id)
            .map_err(|e| StorageError::Other(e.to_string()))
    }

    /// Get count of pending manifests.
    pub fn pending_count(&self) -> Result<usize, StorageError> {
        self.provider.pending_count().map_err(|e| StorageError::Other(e.to_string()))
    }

    // ========== Cold Storage Operations (formerly ManifestService) ==========

    /// Create an in-memory manifest for a table scope.
    pub fn create_manifest(&self, table_id: &TableId, user_id: Option<&UserId>) -> Manifest {
        Manifest::new(table_id.clone(), user_id.cloned())
    }

    /// Ensure a manifest exists through the canonical read path, otherwise create a new one.
    pub fn ensure_manifest_initialized(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<Manifest, StorageError> {
        if let Some(entry) = self.get_or_load(table_id, user_id)? {
            return Ok(entry.manifest.clone());
        }

        Ok(self.create_manifest(table_id, user_id))
    }

    /// Update manifest in the local cache layers and mark it dirty.
    ///
    /// This does not write cold `manifest.json`; periodic flush paths should call
    /// `persist_flushed_segment()` after the Parquet batch is durable.
    pub fn update_manifest(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
        segment: SegmentMetadata,
    ) -> Result<Manifest, StorageError> {
        // Ensure manifest is loaded/initialized
        let mut manifest = self.ensure_manifest_initialized(table_id, user_id)?;

        // Add segment
        manifest.add_segment(segment);

        self.upsert_cache_entry(table_id, user_id, &manifest, None, SyncState::PendingWrite)?;

        Ok(manifest)
    }

    /// Commit a flush-time segment addition through all layers.
    ///
    /// This is the single-step flush path: mutate the manifest in memory, then persist the
    /// resulting manifest to cold storage and refresh the RocksDB + memory copies as in-sync.
    pub fn persist_flushed_segment(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
        segment: SegmentMetadata,
    ) -> Result<Manifest, StorageError> {
        self.with_flush_scope_lock(table_id, user_id, move || {
            self.persist_flushed_segment_in_locked_scope(table_id, user_id, segment)
        })
    }

    /// Update manifest in cache using a caller-provided mutator.
    ///
    /// This is used for metadata updates that are not segment appends
    /// (for example vector index watermark/snapshot pointers). This updates only the
    /// cache layers; explicit persist/flush paths should write `manifest.json` when ready.
    pub fn update_manifest_with<F>(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
        mutator: F,
    ) -> Result<Manifest, StorageError>
    where
        F: FnOnce(&mut Manifest),
    {
        let mut manifest = self.ensure_manifest_initialized(table_id, user_id)?;
        mutator(&mut manifest);
        self.upsert_cache_entry(table_id, user_id, &manifest, None, SyncState::PendingWrite)?;
        Ok(manifest)
    }

    /// Persist a manifest to cold storage and mark cache state as in-sync.
    pub fn persist_manifest(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
        manifest: &Manifest,
    ) -> Result<(), StorageError> {
        self.write_manifest_to_storage(table_id, user_id, manifest)?;
        self.upsert_cache_entry(table_id, user_id, manifest, None, SyncState::InSync)
    }

    /// Clear all manifest segments for a table scope and delete their associated Parquet files.
    ///
    /// This is the first step of cold-storage compaction cleanup for scopes that
    /// resolve to zero live rows after flush. It keeps an empty manifest on disk
    /// so future flushes can continue with a clean state.
    pub fn clear_segments_and_delete_files(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<usize, StorageError> {
        let (table_type, storage_cached) = self.storage_cached_for_table(table_id)?;

        let mut manifest = self.ensure_manifest_initialized(table_id, user_id)?;
        if manifest.segments.is_empty() {
            return Ok(0);
        }

        let segment_paths =
            manifest.segments.iter().map(|segment| segment.path.clone()).collect::<Vec<_>>();

        manifest.segments.clear();
        manifest.last_sequence_number = 0;
        manifest.updated_at = chrono::Utc::now().timestamp_millis();
        manifest.version += 1;

        self.persist_manifest(table_id, user_id, &manifest)?;

        let mut deleted_files = 0;
        for path in segment_paths {
            let delete_result = storage_cached
                .delete_sync(table_type, table_id, user_id, &path)
                .map_err(|e| StorageError::IoError(e.to_string()))?;
            if delete_result.existed {
                deleted_files += 1;
            }
        }

        Ok(deleted_files)
    }

    /// Rebuild manifest from Parquet footers.
    pub fn rebuild_manifest(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<Manifest, StorageError> {
        let (table_type, storage_cached) = self.storage_cached_for_table(table_id)?;

        let mut manifest = Manifest::new(table_id.clone(), user_id.cloned());

        // List all parquet files using the optimized method
        let mut batch_files = storage_cached
            .list_parquet_files_sync(table_type, table_id, user_id)
            .map_err(|e| StorageError::IoError(e.to_string()))?;

        // Filter only batch files (exclude compaction temp files etc)
        batch_files.retain(|f| f.contains("batch-"));
        batch_files.sort();

        // Batch fetch metadata if possible (TODO: Add bulk head to filestore)
        // For now, sequential head
        for file_name in &batch_files {
            let id = file_name.clone();

            // Get file size via head operation
            let file_info = storage_cached
                .head_sync(table_type, table_id, user_id, file_name)
                .map_err(|e| StorageError::IoError(e.to_string()))?;

            let size_bytes = file_info.size as u64;

            // Create segment metadata (we don't parse full footer for rebuild, just size)
            let segment = SegmentMetadata::new(
                id,
                file_name.clone(),
                HashMap::new(),
                SeqId::from(0i64),
                SeqId::from(0i64),
                0,
                size_bytes,
            );
            manifest.add_segment(segment);
        }

        self.persist_manifest(table_id, user_id, &manifest)?;

        Ok(manifest)
    }

    /// Validate manifest consistency.
    pub fn validate_manifest(&self, _manifest: &Manifest) -> Result<(), StorageError> {
        // Basic validation - can be expanded
        Ok(())
    }

    /// Public helper for consumers that need the resolved manifest.json path.
    pub fn manifest_path(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<String, StorageError> {
        let (table_type, storage_cached) = self.storage_cached_for_table(table_id)?;
        let manifest_path_result = storage_cached.get_manifest_path(table_type, table_id, user_id);

        Ok(manifest_path_result.full_path)
    }

    pub fn get_manifest_user_ids(&self, table_id: &TableId) -> Result<Vec<UserId>, StorageError> {
        // Use storekey-encoded prefix for proper RocksDB scan
        let prefix = ManifestId::table_prefix(table_id);
        log::debug!(
            "[MANIFEST_CACHE_DEBUG] get_manifest_user_ids: table={} prefix_len={}",
            table_id,
            prefix.len()
        );
        // Use scan_keys_with_raw_prefix to only fetch keys (no value deserialization)
        let keys: Vec<ManifestId> = self.provider.scan_manifest_ids_with_raw_prefix(
            &prefix,
            None,
            MAX_MANIFEST_SCAN_LIMIT,
        )?;

        let mut user_ids = HashSet::new();

        for manifest_id in keys {
            log::debug!(
                "[MANIFEST_CACHE_DEBUG] get_manifest_user_ids: found manifest_id={}",
                manifest_id.as_str()
            );
            if let Some(user_id) = manifest_id.user_id() {
                user_ids.insert(user_id.clone());
            }
        }

        log::debug!(
            "[MANIFEST_CACHE_DEBUG] get_manifest_user_ids: result user_ids={:?}",
            user_ids.iter().map(|u| u.as_str()).collect::<Vec<_>>()
        );

        Ok(user_ids.into_iter().collect())
    }

    // ========== Private Helper Methods ==========

    fn upsert_cache_entry(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
        manifest: &Manifest,
        etag: Option<String>,
        sync_state: SyncState,
    ) -> Result<(), StorageError> {
        let rocksdb_key = Self::manifest_id(table_id, user_id);
        let now = chrono::Utc::now().timestamp_millis();

        log::debug!(
            "[MANIFEST_CACHE_DEBUG] upsert_cache_entry: key={} segments={} sync_state={:?}",
            rocksdb_key.as_str(),
            manifest.segments.len(),
            sync_state
        );

        let entry = ManifestCacheEntry::new(manifest.clone(), etag, now, sync_state);
        self.upsert_entry(rocksdb_key, entry).map(|_| ())
    }

    fn manifest_id(table_id: &TableId, user_id: Option<&UserId>) -> ManifestId {
        ManifestId::new(table_id.clone(), user_id.cloned())
    }

    fn insert_memory_entry(&self, manifest_id: ManifestId, entry: Arc<ManifestCacheEntry>) {
        if !self.should_cache_in_memory(&manifest_id) {
            self.memory_cache.remove(&manifest_id);
            self.publish_manifest_memory_metrics();
            return;
        }

        self.memory_cache.insert(manifest_id, entry);
        self.prune_memory_cache_if_needed();
        self.publish_manifest_memory_metrics();
    }

    fn should_cache_in_memory(&self, manifest_id: &ManifestId) -> bool {
        self.config.max_entries > 0 && manifest_id.user_id().is_none()
    }

    fn prune_memory_cache_if_needed(&self) {
        let max_entries = self.config.max_entries;
        if max_entries == 0 {
            self.memory_cache.clear();
            self.publish_manifest_memory_metrics();
            return;
        }
        if self.memory_cache.len() <= max_entries {
            return;
        }

        let now = chrono::Utc::now().timestamp_millis();
        let ttl_millis = self.config.ttl_millis();
        let expired_keys = self
            .memory_cache
            .iter()
            .filter(|entry| entry.value().is_stale(ttl_millis, now))
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        for key in expired_keys {
            self.memory_cache.remove(&key);
        }

        if self.memory_cache.len() <= max_entries {
            return;
        }

        let mut candidates = self
            .memory_cache
            .iter()
            .filter(|entry| {
                !matches!(entry.value().sync_state, SyncState::PendingWrite | SyncState::Syncing)
            })
            .map(|entry| {
                let key = entry.key().clone();
                let age = now.saturating_sub(entry.value().last_refreshed_millis());
                (key, age)
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| right.1.cmp(&left.1));

        let overflow = self.memory_cache.len().saturating_sub(max_entries);
        for (key, _) in candidates.into_iter().take(overflow) {
            self.memory_cache.remove(&key);
        }
        self.publish_manifest_memory_metrics();
    }

    fn upsert_entry(
        &self,
        manifest_id: ManifestId,
        entry: ManifestCacheEntry,
    ) -> Result<Arc<ManifestCacheEntry>, StorageError> {
        let inserted_new_entry = match self.cached_entry_snapshot(&manifest_id) {
            Ok(Some(old_entry)) => {
                self.provider.update_cache_entry_with_old(&manifest_id, &old_entry, &entry)?;
                false
            },
            Ok(None) => {
                self.provider.put_cache_entry(&manifest_id, &entry)?;
                true
            },
            Err(StorageError::SerializationError(err)) => {
                warn!(
                    "Manifest cache entry corrupted for key {}: {} (overwriting)",
                    manifest_id.as_str(),
                    err
                );
                let _ = self.provider.delete_cache_entry(&manifest_id);
                self.provider.put_cache_entry(&manifest_id, &entry)?;
                false
            },
            Err(err) => return Err(err),
        };
        if inserted_new_entry {
            kalamdb_observability::increment_manifest_cache_rocksdb_entries(1);
        }

        let entry = Arc::new(entry);
        self.insert_memory_entry(manifest_id, Arc::clone(&entry));
        Ok(entry)
    }

    pub(crate) fn persist_flushed_segment_in_locked_scope(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
        segment: SegmentMetadata,
    ) -> Result<Manifest, StorageError> {
        let mut manifest = self.ensure_manifest_initialized(table_id, user_id)?;
        manifest.add_segment(segment);

        self.write_manifest_to_storage(table_id, user_id, &manifest)?;

        let next_sync_state = self
            .cached_entry_snapshot(&Self::manifest_id(table_id, user_id))?
            .map(|entry| match entry.sync_state {
                SyncState::PendingWrite => SyncState::PendingWrite,
                _ => SyncState::InSync,
            })
            .unwrap_or(SyncState::InSync);

        self.upsert_cache_entry(table_id, user_id, &manifest, None, next_sync_state)?;
        Ok(manifest)
    }

    pub(crate) fn replace_segments_with_compacted_segment_in_locked_scope(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
        expected_segments: &[SegmentMetadata],
        replacement: Option<SegmentMetadata>,
    ) -> Result<bool, StorageError> {
        let mut manifest = self.ensure_manifest_initialized(table_id, user_id)?;

        if manifest.segments.len() < expected_segments.len() {
            return Ok(false);
        }

        let start = manifest.segments.len() - expected_segments.len();
        for (current, expected) in manifest.segments[start..].iter().zip(expected_segments.iter()) {
            if current.id != expected.id
                || current.path != expected.path
                || current.min_seq != expected.min_seq
                || current.max_seq != expected.max_seq
                || current.row_count != expected.row_count
                || current.size_bytes != expected.size_bytes
                || current.schema_version != expected.schema_version
                || current.status != expected.status
            {
                return Ok(false);
            }
        }

        manifest.segments.truncate(start);
        if let Some(replacement) = replacement {
            manifest.add_segment(replacement);
        }

        self.write_manifest_to_storage(table_id, user_id, &manifest)?;

        let next_sync_state = self
            .cached_entry_snapshot(&Self::manifest_id(table_id, user_id))?
            .map(|entry| match entry.sync_state {
                SyncState::PendingWrite => SyncState::PendingWrite,
                _ => SyncState::InSync,
            })
            .unwrap_or(SyncState::InSync);

        self.upsert_cache_entry(table_id, user_id, &manifest, None, next_sync_state)?;
        Ok(true)
    }

    fn flush_scope_lock(&self, table_id: &TableId, user_id: Option<&UserId>) -> Arc<Mutex<()>> {
        let manifest_id = Self::manifest_id(table_id, user_id);
        let entry = self
            .flush_scope_locks
            .entry(manifest_id)
            .or_insert_with(|| Arc::new(Mutex::new(())));
        Arc::clone(entry.value())
    }

    fn cached_entry_snapshot(
        &self,
        manifest_id: &ManifestId,
    ) -> Result<Option<ManifestCacheEntry>, StorageError> {
        if self.should_cache_in_memory(manifest_id) {
            if let Some(entry) = self.memory_cache.get(manifest_id) {
                return Ok(Some(entry.value().as_ref().clone()));
            }
        }

        self.provider.get_cache_entry(manifest_id)
    }

    fn update_cached_entry<F>(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
        update: F,
    ) -> Result<(), StorageError>
    where
        F: FnOnce(&mut ManifestCacheEntry),
    {
        let rocksdb_key = Self::manifest_id(table_id, user_id);

        match self.cached_entry_snapshot(&rocksdb_key) {
            Ok(Some(old_entry)) => {
                let mut new_entry = old_entry.clone();
                update(&mut new_entry);
                self.provider
                    .update_cache_entry_with_old(&rocksdb_key, &old_entry, &new_entry)?;
                self.insert_memory_entry(rocksdb_key, Arc::new(new_entry));
            },
            Ok(None) => {
                self.memory_cache.remove(&rocksdb_key);
            },
            Err(StorageError::SerializationError(err)) => {
                warn!(
                    "Manifest cache entry corrupted for key {}: {} (dropping)",
                    rocksdb_key.as_str(),
                    err
                );
                let _ = self.provider.delete_cache_entry(&rocksdb_key);
                self.memory_cache.remove(&rocksdb_key);
            },
            Err(err) => return Err(err),
        }

        Ok(())
    }

    pub(crate) fn storage_cached_for_table(
        &self,
        table_id: &TableId,
    ) -> Result<(kalamdb_commons::schemas::TableType, Arc<StorageCached>), StorageError> {
        let schema_registry = self.get_schema_registry();
        let storage_registry = self.get_storage_registry();

        let table = schema_registry
            .get_table_if_exists(table_id)
            .map_err(|e| StorageError::Other(e.to_string()))?
            .ok_or_else(|| StorageError::Other(format!("Table not found: {}", table_id)))?;
        let storage_id = schema_registry
            .get_storage_id(table_id)
            .map_err(|e| StorageError::Other(e.to_string()))?;
        let storage_cached = storage_registry
            .get_cached(&storage_id)
            .map_err(|e| StorageError::Other(e.to_string()))?
            .ok_or_else(|| {
                StorageError::Other(format!("Storage '{}' not found in registry", storage_id))
            })?;

        Ok((table.table_type, storage_cached))
    }

    fn try_storage_cached_for_table(
        &self,
        table_id: &TableId,
    ) -> Result<Option<(kalamdb_commons::schemas::TableType, Arc<StorageCached>)>, StorageError>
    {
        if self.schema_registry.is_none() || self.storage_registry.is_none() {
            return Ok(None);
        }

        self.storage_cached_for_table(table_id).map(Some)
    }

    fn read_manifest_from_storage(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<Option<Manifest>, StorageError> {
        let Some((table_type, storage_cached)) = self.try_storage_cached_for_table(table_id)?
        else {
            return Ok(None);
        };

        let Some(manifest_value) = storage_cached
            .read_manifest_sync(table_type, table_id, user_id)
            .map_err(|e| StorageError::IoError(e.to_string()))?
        else {
            return Ok(None);
        };

        let manifest = serde_json::from_value(manifest_value).map_err(|e| {
            StorageError::SerializationError(format!(
                "failed to deserialize manifest.json for {}: {}",
                Self::manifest_id(table_id, user_id),
                e
            ))
        })?;

        kalamdb_observability::record_manifest_read();
        Ok(Some(manifest))
    }

    async fn read_manifest_from_storage_async(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<Option<Manifest>, StorageError> {
        let Some((table_type, storage_cached)) = self.try_storage_cached_for_table(table_id)?
        else {
            return Ok(None);
        };

        let result = match storage_cached.get(table_type, table_id, user_id, "manifest.json").await
        {
            Ok(result) => result,
            Err(FilestoreError::NotFound(_)) => return Ok(None),
            Err(err) => return Err(StorageError::IoError(err.to_string())),
        };

        let manifest = serde_json::from_slice(&result.data).map_err(|e| {
            StorageError::SerializationError(format!(
                "failed to deserialize manifest.json for {}: {}",
                Self::manifest_id(table_id, user_id),
                e
            ))
        })?;

        kalamdb_observability::record_manifest_read();
        Ok(Some(manifest))
    }

    fn load_from_storage_and_hydrate(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<Option<Arc<ManifestCacheEntry>>, StorageError> {
        let Some(manifest) = self.read_manifest_from_storage(table_id, user_id)? else {
            return Ok(None);
        };
        let entry = ManifestCacheEntry::new(
            manifest,
            None,
            chrono::Utc::now().timestamp_millis(),
            SyncState::InSync,
        );
        self.upsert_entry(Self::manifest_id(table_id, user_id), entry).map(Some)
    }

    async fn load_from_storage_and_hydrate_async(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<Option<Arc<ManifestCacheEntry>>, StorageError> {
        let Some(manifest) = self.read_manifest_from_storage_async(table_id, user_id).await? else {
            return Ok(None);
        };
        let entry = ManifestCacheEntry::new(
            manifest,
            None,
            chrono::Utc::now().timestamp_millis(),
            SyncState::InSync,
        );
        self.upsert_entry(Self::manifest_id(table_id, user_id), entry).map(Some)
    }

    fn write_manifest_to_storage(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
        manifest: &Manifest,
    ) -> Result<(), StorageError> {
        let (table_type, storage_cached) = self.storage_cached_for_table(table_id)?;
        storage_cached
            .write_manifest_sync(table_type, table_id, user_id, manifest)
            .map_err(|e| StorageError::IoError(e.to_string()))?;
        kalamdb_observability::record_manifest_write();
        Ok(())
    }

    // ========== File Subfolder State Methods ==========

    /// Get the file subfolder state for a shared table (user_id = None).
    /// Returns None if files are not enabled for this table.
    pub fn get_file_subfolder_state(
        &self,
        table_id: &TableId,
    ) -> Result<Option<FileSubfolderState>, StorageError> {
        let entry = self.get_or_load(table_id, None)?;
        match entry {
            Some(cache_entry) => Ok(cache_entry.manifest.files.clone()),
            None => Ok(None),
        }
    }

    /// Update the file subfolder state for a shared table.
    /// This is used when files are uploaded and the subfolder needs rotation.
    pub fn update_file_subfolder_state(
        &self,
        table_id: &TableId,
        state: FileSubfolderState,
    ) -> Result<(), StorageError> {
        let entry = self.get_or_load(table_id, None)?;
        let mut manifest = match entry {
            Some(cache_entry) => cache_entry.manifest.clone(),
            None => {
                // Create a minimal manifest for files tracking
                Manifest::new(table_id.clone(), None)
            },
        };
        manifest.files = Some(state);
        self.upsert_cache_entry(table_id, None, &manifest, None, SyncState::PendingWrite)
    }

    // Private helper methods removed - now using StorageCached operations directly
}

#[async_trait::async_trait]
impl ManifestServiceTrait for ManifestService {
    fn get_or_load(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<Option<Arc<ManifestCacheEntry>>, StorageError> {
        self.get_or_load(table_id, user_id)
    }

    async fn get_or_load_async(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<Option<Arc<ManifestCacheEntry>>, StorageError> {
        self.get_or_load_async(table_id, user_id).await
    }

    fn validate_manifest(&self, manifest: &Manifest) -> Result<(), StorageError> {
        self.validate_manifest(manifest)
    }

    fn mark_as_stale(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<(), StorageError> {
        self.mark_as_stale(table_id, user_id)
    }

    fn rebuild_manifest(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<Manifest, StorageError> {
        self.rebuild_manifest(table_id, user_id)
    }

    fn mark_pending_write(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<(), StorageError> {
        self.mark_pending_write(table_id, user_id)
    }

    fn ensure_manifest_initialized(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<Manifest, StorageError> {
        self.ensure_manifest_initialized(table_id, user_id)
    }

    fn stage_before_flush(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
        manifest: &Manifest,
    ) -> Result<(), StorageError> {
        self.stage_before_flush(table_id, user_id, manifest)
    }

    fn get_manifest_user_ids(&self, table_id: &TableId) -> Result<Vec<UserId>, StorageError> {
        self.get_manifest_user_ids(table_id)
    }
}

#[cfg(test)]
mod tests {
    use datafusion::arrow::datatypes::{Schema, SchemaRef};
    use kalamdb_commons::{
        schemas::{TableDefinition, TableOptions, TableType},
        NamespaceId, StorageId, TableName,
    };
    use kalamdb_configs::RemoteStorageTimeouts;
    use kalamdb_store::{test_utils::InMemoryBackend, StorageBackend};
    use kalamdb_system::{Storage, StorageType, StoragesTableProvider};
    use tempfile::TempDir;

    use super::*;

    #[derive(Debug, Clone)]
    struct TestSchemaRegistry {
        table_id:   TableId,
        table_def:  Arc<TableDefinition>,
        storage_id: StorageId,
    }

    impl SchemaRegistryTrait for TestSchemaRegistry {
        type Error = TableError;

        fn get_arrow_schema(&self, _table_id: &TableId) -> Result<SchemaRef, Self::Error> {
            Ok(Arc::new(Schema::empty()))
        }

        fn get_table_if_exists(
            &self,
            table_id: &TableId,
        ) -> Result<Option<Arc<TableDefinition>>, Self::Error> {
            if table_id == &self.table_id {
                Ok(Some(Arc::clone(&self.table_def)))
            } else {
                Ok(None)
            }
        }

        fn get_arrow_schema_for_version(
            &self,
            table_id: &TableId,
            _schema_version: u32,
        ) -> Result<SchemaRef, Self::Error> {
            self.get_arrow_schema(table_id)
        }

        fn get_storage_id(&self, table_id: &TableId) -> Result<StorageId, Self::Error> {
            if table_id == &self.table_id {
                Ok(self.storage_id.clone())
            } else {
                Err(TableError::TableNotFound(table_id.to_string()))
            }
        }
    }

    fn test_config() -> ManifestCacheSettings {
        ManifestCacheSettings {
            eviction_interval_seconds: 300,
            max_entries:               1000,
            eviction_ttl_days:         7,
        }
    }

    fn create_test_service_with_backend(backend: Arc<dyn StorageBackend>) -> ManifestService {
        let provider = Arc::new(ManifestTableProvider::new(backend));
        ManifestService::new(provider, test_config())
    }

    fn create_test_service() -> ManifestService {
        create_test_service_with_backend(Arc::new(InMemoryBackend::new()))
    }

    fn create_test_manifest(table_id: &TableId, user_id: Option<&UserId>) -> Manifest {
        Manifest::new(table_id.clone(), user_id.cloned())
    }

    fn build_table_id(ns: &str, tbl: &str) -> TableId {
        TableId::new(NamespaceId::new(ns), TableName::new(tbl))
    }

    fn test_segment(path: &str, min_seq: i64, max_seq: i64, row_count: u64) -> SegmentMetadata {
        SegmentMetadata::new(
            path.to_string(),
            path.to_string(),
            HashMap::new(),
            SeqId::from(min_seq),
            SeqId::from(max_seq),
            row_count,
            row_count.saturating_mul(128),
        )
    }

    #[test]
    fn compaction_scope_guard_rejects_duplicate_active_scope() {
        let service = create_test_service();
        let table_id = build_table_id("app", "events");
        let user_id = UserId::from("user-1");

        let guard = service
            .try_begin_compaction_scope(&table_id, Some(&user_id))
            .expect("first compaction should acquire scope");
        assert!(service.try_begin_compaction_scope(&table_id, Some(&user_id)).is_none());
        assert!(service.try_begin_compaction_scope(&table_id, None).is_some());

        drop(guard);
        assert!(service.try_begin_compaction_scope(&table_id, Some(&user_id)).is_some());
    }

    #[test]
    fn compacted_segment_replacement_requires_unchanged_trailing_suffix() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let table_id = build_table_id("app", "events");
        let (service, _storage_registry) =
            create_test_service_with_storage(backend, &table_id, TableType::Shared, &temp_dir);
        let first = test_segment("batch-1.parquet", 1, 10, 10);
        let second = test_segment("batch-2.parquet", 11, 20, 10);
        let appended = test_segment("batch-3.parquet", 21, 30, 10);
        let replacement = test_segment("compact-1.parquet", 11, 30, 20);

        service
            .persist_flushed_segment_in_locked_scope(&table_id, None, first.clone())
            .expect("persist first segment");
        service
            .persist_flushed_segment_in_locked_scope(&table_id, None, second.clone())
            .expect("persist second segment");
        service
            .persist_flushed_segment_in_locked_scope(&table_id, None, appended.clone())
            .expect("persist appended segment");

        let stale_swap = service
            .replace_segments_with_compacted_segment_in_locked_scope(
                &table_id,
                None,
                &[first.clone(), second.clone()],
                Some(replacement.clone()),
            )
            .expect("attempt stale compacted swap");
        assert!(!stale_swap);

        let manifest_after_stale_swap = service
            .get_or_load(&table_id, None)
            .expect("load manifest")
            .expect("manifest exists")
            .manifest
            .clone();
        assert_eq!(manifest_after_stale_swap.segments.len(), 3);
        assert_eq!(manifest_after_stale_swap.segments[0].path, first.path);
        assert_eq!(manifest_after_stale_swap.segments[1].path, second.path);
        assert_eq!(manifest_after_stale_swap.segments[2].path, appended.path);

        let current_tail_swap = service
            .replace_segments_with_compacted_segment_in_locked_scope(
                &table_id,
                None,
                &[second, appended],
                Some(replacement.clone()),
            )
            .expect("replace current compacted suffix");
        assert!(current_tail_swap);

        let final_manifest = service
            .get_or_load(&table_id, None)
            .expect("load final manifest")
            .expect("manifest exists")
            .manifest
            .clone();
        assert_eq!(final_manifest.segments.len(), 2);
        assert_eq!(final_manifest.segments[0].path, first.path);
        assert_eq!(final_manifest.segments[1].path, replacement.path);
    }

    fn create_test_storage_registry(
        temp_dir: &TempDir,
        backend: Arc<dyn StorageBackend>,
    ) -> Arc<StorageRegistry> {
        let storages_provider = Arc::new(StoragesTableProvider::new(backend));
        let storage_id = StorageId::local();
        let base_directory = temp_dir.path().to_string_lossy().into_owned();

        storages_provider
            .create_storage(Storage {
                storage_id,
                storage_name: "Local Storage".to_string(),
                description: Some("manifest service test storage".to_string()),
                storage_type: StorageType::Filesystem,
                base_directory: base_directory.clone(),
                credentials: None,
                config_json: None,
                shared_tables_template: "shared/{namespace}/{tableName}".to_string(),
                user_tables_template: "user/{namespace}/{tableName}/{userId}".to_string(),
                created_at: 1_000,
                updated_at: 1_000,
            })
            .expect("seed local storage");

        Arc::new(StorageRegistry::new(
            storages_provider,
            base_directory,
            RemoteStorageTimeouts::default(),
            Default::default(),
        ))
    }

    fn create_test_table(table_id: &TableId, table_type: TableType) -> Arc<TableDefinition> {
        Arc::new(
            TableDefinition::new(
                table_id.namespace_id().clone(),
                table_id.table_name().clone(),
                table_type,
                Vec::new(),
                match table_type {
                    TableType::User => TableOptions::user(),
                    TableType::Shared => TableOptions::shared(),
                    TableType::Stream => TableOptions::stream(86400),
                    TableType::System => TableOptions::system(),
                },
                None,
            )
            .expect("create test table"),
        )
    }

    fn create_test_service_with_storage(
        backend: Arc<dyn StorageBackend>,
        table_id: &TableId,
        table_type: TableType,
        temp_dir: &TempDir,
    ) -> (ManifestService, Arc<StorageRegistry>) {
        let storage_registry = create_test_storage_registry(temp_dir, Arc::clone(&backend));
        let schema_registry = Arc::new(TestSchemaRegistry {
            table_id:   table_id.clone(),
            table_def:  create_test_table(table_id, table_type),
            storage_id: StorageId::local(),
        });
        let service = ManifestService::new_with_registries(
            backend,
            temp_dir.path().to_string_lossy().into_owned(),
            test_config(),
            schema_registry,
            Arc::clone(&storage_registry),
        );

        (service, storage_registry)
    }

    #[test]
    fn test_create_manifest() {
        let service = create_test_service();
        let table_id = build_table_id("ns1", "products");

        let manifest = service.create_manifest(&table_id, None);

        assert_eq!(manifest.table_id, table_id);
        assert_eq!(manifest.user_id, None);
        assert_eq!(manifest.segments.len(), 0);
    }

    #[test]
    fn test_get_or_load_miss() {
        let service = create_test_service();
        let table_id = build_table_id("ns1", "tbl1");

        let result = service.get_or_load(&table_id, Some(&UserId::from("u_123"))).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_update_after_flush() {
        let service = create_test_service();
        let table_id = build_table_id("ns1", "tbl1");
        let manifest = create_test_manifest(&table_id, Some(&UserId::from("u_123")));

        service
            .update_after_flush(
                &table_id,
                Some(&UserId::from("u_123")),
                &manifest,
                Some("etag123".to_string()),
            )
            .unwrap();

        let cached = service.get_or_load(&table_id, Some(&UserId::from("u_123"))).unwrap();
        assert!(cached.is_some());
        let entry = cached.unwrap();
        assert_eq!(entry.etag, Some("etag123".to_string()));
        assert_eq!(entry.sync_state, SyncState::InSync);
    }

    #[test]
    fn test_hot_cache_hit() {
        let service = create_test_service();
        let table_id = build_table_id("ns1", "tbl1");
        let manifest = create_test_manifest(&table_id, None);

        service.update_after_flush(&table_id, None, &manifest, None).unwrap();

        let result = service.get_or_load(&table_id, None).unwrap();
        assert!(result.is_some());

        assert!(service.is_in_hot_cache(&table_id, None));
    }

    #[test]
    fn test_memory_hit_reuses_cached_arc() {
        let service = create_test_service();
        let table_id = build_table_id("ns1", "tbl1");
        let manifest = create_test_manifest(&table_id, None);

        service.update_after_flush(&table_id, None, &manifest, None).unwrap();

        let first = service.get_or_load(&table_id, None).unwrap().unwrap();
        let second = service.get_or_load(&table_id, None).unwrap().unwrap();

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn test_rocksdb_fallback_hydrates_memory() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
        let table_id = build_table_id("ns1", "tbl1");
        let manifest = create_test_manifest(&table_id, None);

        let service1 = create_test_service_with_backend(Arc::clone(&backend));
        service1.update_after_flush(&table_id, None, &manifest, None).unwrap();

        let service2 = create_test_service_with_backend(backend);
        assert!(!service2.is_in_hot_cache(&table_id, None));

        let entry = service2.get_or_load(&table_id, None).unwrap();

        assert!(entry.is_some());
        assert!(service2.is_in_hot_cache(&table_id, None));
    }

    #[test]
    fn test_user_manifest_uses_rocksdb_not_memory() {
        let service = create_test_service();
        let table_id = build_table_id("ns1", "user_tbl");
        let user_id = UserId::from("u_123");
        let manifest = create_test_manifest(&table_id, Some(&user_id));

        service.update_after_flush(&table_id, Some(&user_id), &manifest, None).unwrap();

        let entry = service.get_or_load(&table_id, Some(&user_id)).unwrap();
        assert!(entry.is_some());
        assert!(!service.is_in_hot_cache(&table_id, Some(&user_id)));
    }

    #[test]
    fn test_storage_fallback_hydrates_rocksdb_and_memory() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let table_id = build_table_id("ns1", "tbl1");
        let (service, storage_registry) = create_test_service_with_storage(
            Arc::clone(&backend),
            &table_id,
            TableType::Shared,
            &temp_dir,
        );
        let storage_cached = storage_registry
            .get_cached(&StorageId::local())
            .expect("lookup storage")
            .expect("local storage exists");
        let manifest = create_test_manifest(&table_id, None);

        storage_cached
            .write_manifest_sync(TableType::Shared, &table_id, None, &manifest)
            .expect("write storage manifest");

        let entry = service.get_or_load(&table_id, None).unwrap().expect("manifest found");

        assert_eq!(entry.manifest.table_id, table_id);
        assert!(service.is_in_hot_cache(&table_id, None));
        assert_eq!(service.count().unwrap(), 1);
    }

    #[test]
    fn test_user_storage_fallback_hydrates_rocksdb_only() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let table_id = build_table_id("ns1", "user_tbl");
        let user_id = UserId::from("u_123");
        let (service, storage_registry) = create_test_service_with_storage(
            Arc::clone(&backend),
            &table_id,
            TableType::User,
            &temp_dir,
        );
        let storage_cached = storage_registry
            .get_cached(&StorageId::local())
            .expect("lookup storage")
            .expect("local storage exists");
        let manifest = create_test_manifest(&table_id, Some(&user_id));

        storage_cached
            .write_manifest_sync(TableType::User, &table_id, Some(&user_id), &manifest)
            .expect("write storage manifest");

        let entry =
            service.get_or_load(&table_id, Some(&user_id)).unwrap().expect("manifest found");

        assert_eq!(entry.manifest.user_id, Some(user_id.clone()));
        assert!(!service.is_in_hot_cache(&table_id, Some(&user_id)));
        assert_eq!(service.count().unwrap(), 1);
    }

    #[test]
    fn test_missing_manifest_returns_none() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let table_id = build_table_id("ns1", "missing_tbl");
        let (service, _storage_registry) =
            create_test_service_with_storage(backend, &table_id, TableType::Shared, &temp_dir);

        assert!(service.get_or_load(&table_id, None).unwrap().is_none());
        assert!(!service.is_in_hot_cache(&table_id, None));
        assert_eq!(service.count().unwrap(), 0);
    }

    #[test]
    fn test_persist_manifest_writes_storage_rocksdb_and_memory() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let table_id = build_table_id("ns1", "save_tbl");
        let (service, storage_registry) = create_test_service_with_storage(
            Arc::clone(&backend),
            &table_id,
            TableType::Shared,
            &temp_dir,
        );
        let storage_cached = storage_registry
            .get_cached(&StorageId::local())
            .expect("lookup storage")
            .expect("local storage exists");
        let manifest = create_test_manifest(&table_id, None);

        service.persist_manifest(&table_id, None, &manifest).unwrap();

        assert!(service.is_in_hot_cache(&table_id, None));
        assert!(service.get_or_load(&table_id, None).unwrap().is_some());
        assert!(storage_cached
            .read_manifest_sync(TableType::Shared, &table_id, None)
            .expect("read storage manifest")
            .is_some());
    }

    #[test]
    fn test_persist_user_manifest_writes_storage_and_rocksdb_only() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let table_id = build_table_id("ns1", "user_save_tbl");
        let user_id = UserId::from("u_123");
        let (service, storage_registry) = create_test_service_with_storage(
            Arc::clone(&backend),
            &table_id,
            TableType::User,
            &temp_dir,
        );
        let storage_cached = storage_registry
            .get_cached(&StorageId::local())
            .expect("lookup storage")
            .expect("local storage exists");
        let manifest = create_test_manifest(&table_id, Some(&user_id));

        service.persist_manifest(&table_id, Some(&user_id), &manifest).unwrap();

        assert!(!service.is_in_hot_cache(&table_id, Some(&user_id)));
        assert!(service.get_or_load(&table_id, Some(&user_id)).unwrap().is_some());
        assert!(storage_cached
            .read_manifest_sync(TableType::User, &table_id, Some(&user_id))
            .expect("read storage manifest")
            .is_some());
    }

    #[test]
    fn test_update_manifest_keeps_storage_cold_until_flush() {
        let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let table_id = build_table_id("ns1", "dirty_tbl");
        let (service, storage_registry) = create_test_service_with_storage(
            Arc::clone(&backend),
            &table_id,
            TableType::Shared,
            &temp_dir,
        );
        let storage_cached = storage_registry
            .get_cached(&StorageId::local())
            .expect("lookup storage")
            .expect("local storage exists");
        let segment = SegmentMetadata::new(
            "batch-1.parquet".to_string(),
            "batch-1.parquet".to_string(),
            HashMap::new(),
            SeqId::from(1i64),
            SeqId::from(10i64),
            10,
            256,
        );

        let manifest = service.update_manifest(&table_id, None, segment).unwrap();

        let cached = service.get_or_load(&table_id, None).unwrap().expect("cached manifest");
        assert_eq!(cached.sync_state, SyncState::PendingWrite);
        assert_eq!(cached.manifest.segments.len(), 1);
        assert_eq!(manifest.segments.len(), 1);
        assert!(storage_cached
            .read_manifest_sync(TableType::Shared, &table_id, None)
            .expect("read storage manifest")
            .is_none());
    }

    #[test]
    fn test_invalidate() {
        let service = create_test_service();
        let namespace = NamespaceId::new("ns1");
        let table = TableName::new("tbl1");
        let table_id = TableId::new(namespace.clone(), table.clone());
        let manifest = create_test_manifest(&table_id, Some(&UserId::from("u_123")));

        service
            .update_after_flush(&table_id, Some(&UserId::from("u_123")), &manifest, None)
            .unwrap();

        assert!(service.get_or_load(&table_id, Some(&UserId::from("u_123"))).unwrap().is_some());

        service.invalidate(&table_id, Some(&UserId::from("u_123"))).unwrap();

        assert!(service.get_or_load(&table_id, Some(&UserId::from("u_123"))).unwrap().is_none());
    }

    #[test]
    fn test_mark_syncing_updates_state() {
        let service = create_test_service();
        let table_id = build_table_id("ns1", "tbl1");
        let manifest = create_test_manifest(&table_id, Some(&UserId::from("u_123")));

        service
            .update_after_flush(&table_id, Some(&UserId::from("u_123")), &manifest, None)
            .unwrap();

        let cached = service.get_or_load(&table_id, Some(&UserId::from("u_123"))).unwrap().unwrap();
        assert_eq!(cached.sync_state, SyncState::InSync);

        service.mark_syncing(&table_id, Some(&UserId::from("u_123"))).unwrap();

        let cached_after =
            service.get_or_load(&table_id, Some(&UserId::from("u_123"))).unwrap().unwrap();
        assert_eq!(cached_after.sync_state, SyncState::Syncing);
    }

    #[test]
    fn test_pending_write_index_integration() {
        let service = create_test_service();
        let table_id = build_table_id("ns1", "tbl1");
        let user_id = UserId::from("u_123");
        let manifest = create_test_manifest(&table_id, Some(&user_id));

        // Stage manifest first (creates entry in cache)
        service.stage_before_flush(&table_id, Some(&user_id), &manifest).unwrap();

        // Initially, pending index should be empty
        assert_eq!(service.pending_count().unwrap(), 0);
        assert!(!service.has_pending_writes(&table_id, Some(&user_id)).unwrap());

        // Mark as pending write
        service.mark_pending_write(&table_id, Some(&user_id)).unwrap();

        // Now should be in pending index
        assert_eq!(service.pending_count().unwrap(), 1);
        assert!(service.has_pending_writes(&table_id, Some(&user_id)).unwrap());

        // Get all pending - should return our entry
        let pending = service.get_pending_manifests().unwrap();
        assert_eq!(pending.len(), 1);

        // After flush, pending should be removed
        service.update_after_flush(&table_id, Some(&user_id), &manifest, None).unwrap();
        assert_eq!(service.pending_count().unwrap(), 0);
        assert!(!service.has_pending_writes(&table_id, Some(&user_id)).unwrap());
    }

    #[test]
    fn test_mark_pending_write_is_idempotent() {
        let service = create_test_service();
        let table_id = build_table_id("ns1", "tbl1");
        let user_id = UserId::from("u_123");
        let manifest = create_test_manifest(&table_id, Some(&user_id));

        service.stage_before_flush(&table_id, Some(&user_id), &manifest).unwrap();

        service.mark_pending_write(&table_id, Some(&user_id)).unwrap();
        let pending = service.get_or_load(&table_id, Some(&user_id)).unwrap().unwrap();
        let pending_last_refreshed = pending.last_refreshed_millis();
        assert_eq!(pending.sync_state, SyncState::PendingWrite);
        assert!(service.has_pending_writes(&table_id, Some(&user_id)).unwrap());
        assert!(!service.is_in_hot_cache(&table_id, Some(&user_id)));

        service.mark_pending_write(&table_id, Some(&user_id)).unwrap();

        let pending_again = service.get_or_load(&table_id, Some(&user_id)).unwrap().unwrap();
        assert_eq!(pending_again.sync_state, SyncState::PendingWrite);
        assert_eq!(pending_again.last_refreshed_millis(), pending_last_refreshed);
        assert_eq!(service.pending_count().unwrap(), 1);
        assert!(!service.is_in_hot_cache(&table_id, Some(&user_id)));
    }

    #[test]
    fn test_get_pending_for_table() {
        let service = create_test_service();
        let table_id = build_table_id("ns1", "user_table");
        let user1 = UserId::from("user1");
        let user2 = UserId::from("user2");
        let other_table = build_table_id("ns1", "other_table");

        // Stage manifests for multiple users
        let manifest1 = create_test_manifest(&table_id, Some(&user1));
        let manifest2 = create_test_manifest(&table_id, Some(&user2));
        let other_manifest = create_test_manifest(&other_table, None);

        service.stage_before_flush(&table_id, Some(&user1), &manifest1).unwrap();
        service.stage_before_flush(&table_id, Some(&user2), &manifest2).unwrap();
        service.stage_before_flush(&other_table, None, &other_manifest).unwrap();

        // Mark all as pending
        service.mark_pending_write(&table_id, Some(&user1)).unwrap();
        service.mark_pending_write(&table_id, Some(&user2)).unwrap();
        service.mark_pending_write(&other_table, None).unwrap();

        // Get pending for specific table
        let pending = service.get_pending_for_table(&table_id).unwrap();
        assert_eq!(pending.len(), 2);

        // Total should be 3
        assert_eq!(service.pending_count().unwrap(), 3);
    }

    // #[test]
    // fn test_restore_from_rocksdb() {
    //     let backend: Arc<dyn StorageBackend> = Arc::new(InMemoryBackend::new());
    //     let config = ManifestCacheSettings::default();

    //     let service1 = ManifestService::new(Arc::clone(&backend), config.clone());
    //     let table_id = build_table_id("ns1", "tbl1");
    //     let manifest = create_test_manifest(&table_id, Some(&UserId::from("u_123")));

    //     service1
    //         .update_after_flush(
    //             &table_id,
    //             Some(&UserId::from("u_123")),
    //             &manifest,
    //             None,
    //         )
    //         .unwrap();

    //     // Create new service (simulating restart)
    //     let service2 = ManifestService::new(backend, config);
    //     service2.restore_from_rocksdb().unwrap();

    //     let cached = service2
    //         .get_or_load(&table_id, Some(&UserId::from("u_123")))
    //         .unwrap();
    //     assert!(cached.is_some());
    // }
}
