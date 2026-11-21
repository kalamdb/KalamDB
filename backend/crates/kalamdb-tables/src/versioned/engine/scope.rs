//! What user and shared versioned tables do not share.
//!
//! Row identity, the key prefix, the primary-key index, and the vector hot
//! store stay on the scope. Shared row-level security stays on
//! [`SharedTableAuthorization`] and is absent for user scope.

use std::sync::Arc;

use datafusion::scalar::ScalarValue;
use kalamdb_commons::{
    ids::{SeqId, UserTableRowId, VersionId},
    models::{
        rows::{Row, SharedTableRow, UserTableRow},
        UserId,
    },
    KSerializable, StorageKey, TableId, TableType,
};
use kalamdb_datafusion_sources::exec::VersionedRow;
use kalamdb_rls::BoundTablePolicies;
use kalamdb_store::{IndexedEntityStore, StorageBackend};
use kalamdb_vector::{
    new_indexed_shared_vector_hot_store, new_indexed_user_vector_hot_store, SharedVectorHotOpId,
    UserVectorHotOpId, VectorHotOp,
};

use crate::{
    error::KalamDbError,
    shared_tables::shared_table_authorization::SharedTableAuthorization,
    utils::row_utils::ScanRow,
    versioned::pk_index::{new_shared_pk_index, new_user_pk_index, PkPrefix, VersionedPkIndex},
};

#[derive(Clone)]
pub(crate) struct UserScanContext {
    pub(crate) user_id: UserId,
    pub(crate) allow_all_users: bool,
    pub(crate) snapshot_commit_seq: Option<VersionId>,
}

#[derive(Clone)]
pub(crate) struct SharedScanContext {
    pub(crate) snapshot_commit_seq: Option<VersionId>,
    pub(crate) policies: BoundTablePolicies,
}

pub(crate) trait MutationGuard {
    type Guard: Send;
    fn begin_guard(&self) -> Self::Guard;
}

pub(crate) trait TableScope: Clone + Send + Sync + 'static {
    type RowId: StorageKey + Clone + Send + Sync + 'static;
    type Row: VersionedRow + ScanRow + KSerializable + Clone + Send + Sync + 'static;
    type VectorOpId: StorageKey + Clone + Send + Sync + 'static;
    type Auth: Clone + Send + Sync + 'static;
    type ScanContext: Clone + Send + Sync + 'static;

    const LABEL: &'static str;
    const PROVIDER_NAME: &'static str;
    const TABLE_TYPE: TableType;
    const SCAN_SOURCE: &'static str;
    /// Rows are stored under the acting user. Shared rows are not.
    const USER_SCOPED_ROWS: bool;
    const VALIDATE_DIRECT_NOT_NULL: bool;

    fn new_pk_index(table_id: &TableId, pk_field: &str) -> VersionedPkIndex;
    fn new_vector_store(
        backend: Arc<dyn StorageBackend>,
        table_id: &TableId,
        column_name: &str,
    ) -> IndexedEntityStore<Self::VectorOpId, VectorHotOp>;
    fn new_auth() -> Self::Auth;

    fn storage_user<'a>(actor: Option<&'a UserId>) -> Result<Option<&'a UserId>, KalamDbError>;
    fn make_row_id(user: Option<&UserId>, version: VersionId) -> Self::RowId;
    fn make_row(user: Option<&UserId>, version: VersionId, deleted: bool, fields: Row)
        -> Self::Row;
    fn row_id_version(id: &Self::RowId) -> VersionId;
    fn row_fields(row: &Self::Row) -> &Row;
    fn row_version(row: &Self::Row) -> VersionId;
    fn row_deleted(row: &Self::Row) -> bool;
    fn pk_prefix(
        index: &VersionedPkIndex,
        user: Option<&UserId>,
        pk_value: &ScalarValue,
    ) -> Vec<u8>;
    fn vector_op_id(user: Option<&UserId>, seq: SeqId, pk: String) -> Self::VectorOpId;
    fn decode_version(bytes: &[u8]) -> Option<VersionId>;
    fn row_from_parquet(
        user_id: &UserId,
        row_data: &crate::utils::version_resolution::ParquetRowData,
    ) -> (Self::RowId, Self::Row);
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct UserScope;

#[derive(Clone, Copy, Debug)]
pub(crate) struct SharedScope;

impl TableScope for UserScope {
    type RowId = UserTableRowId;
    type Row = UserTableRow;
    type VectorOpId = UserVectorHotOpId;
    type Auth = ();
    type ScanContext = UserScanContext;

    const LABEL: &'static str = "user";
    const PROVIDER_NAME: &'static str = "UserTableProvider";
    const TABLE_TYPE: TableType = TableType::User;
    const SCAN_SOURCE: &'static str = "user_table_scan";
    const USER_SCOPED_ROWS: bool = true;
    const VALIDATE_DIRECT_NOT_NULL: bool = true;

    fn new_pk_index(table_id: &TableId, pk_field: &str) -> VersionedPkIndex {
        new_user_pk_index(table_id, pk_field)
    }

    fn new_vector_store(
        backend: Arc<dyn StorageBackend>,
        table_id: &TableId,
        column_name: &str,
    ) -> IndexedEntityStore<Self::VectorOpId, VectorHotOp> {
        new_indexed_user_vector_hot_store(backend, table_id, column_name)
    }

    fn new_auth() -> Self::Auth {}

    fn storage_user<'a>(actor: Option<&'a UserId>) -> Result<Option<&'a UserId>, KalamDbError> {
        actor.map(Some).ok_or_else(|| {
            KalamDbError::InvalidOperation("user id is required for a user table".to_string())
        })
    }

    fn make_row_id(user: Option<&UserId>, version: VersionId) -> Self::RowId {
        UserTableRowId::new(user.expect("user scope").clone(), version)
    }

    fn make_row(
        user: Option<&UserId>,
        version: VersionId,
        deleted: bool,
        fields: Row,
    ) -> Self::Row {
        UserTableRow {
            user_id: user.expect("user scope").clone(),
            _version: version,
            _deleted: deleted,
            fields,
        }
    }

    fn row_id_version(id: &Self::RowId) -> VersionId {
        id.version()
    }

    fn row_fields(row: &Self::Row) -> &Row {
        &row.fields
    }

    fn row_version(row: &Self::Row) -> VersionId {
        row._version
    }

    fn row_deleted(row: &Self::Row) -> bool {
        row._deleted
    }

    fn pk_prefix(
        index: &VersionedPkIndex,
        user: Option<&UserId>,
        pk_value: &ScalarValue,
    ) -> Vec<u8> {
        index.build_user_pk_prefix(user.expect("user scope"), pk_value)
    }

    fn vector_op_id(user: Option<&UserId>, seq: SeqId, pk: String) -> Self::VectorOpId {
        UserVectorHotOpId::new(user.expect("user scope").clone(), seq, pk)
    }

    fn decode_version(bytes: &[u8]) -> Option<VersionId> {
        crate::versioned::columnar::decode_user_version(bytes)
    }

    fn row_from_parquet(
        user_id: &UserId,
        row_data: &crate::utils::version_resolution::ParquetRowData,
    ) -> (Self::RowId, Self::Row) {
        let row_key = UserTableRowId::new(user_id.clone(), row_data.version);
        let row = UserTableRow {
            user_id: user_id.clone(),
            _version: row_data.version,
            _deleted: row_data.deleted,
            fields: row_data.fields.clone(),
        };
        (row_key, row)
    }
}

impl TableScope for SharedScope {
    type RowId = kalamdb_commons::ids::SharedTableRowId;
    type Row = SharedTableRow;
    type VectorOpId = SharedVectorHotOpId;
    type Auth = SharedTableAuthorization;
    type ScanContext = SharedScanContext;

    const LABEL: &'static str = "shared";
    const PROVIDER_NAME: &'static str = "SharedTableProvider";
    const TABLE_TYPE: TableType = TableType::Shared;
    const SCAN_SOURCE: &'static str = "shared_table_scan";
    const USER_SCOPED_ROWS: bool = false;
    const VALIDATE_DIRECT_NOT_NULL: bool = false;

    fn new_pk_index(table_id: &TableId, pk_field: &str) -> VersionedPkIndex {
        new_shared_pk_index(table_id, pk_field)
    }

    fn new_vector_store(
        backend: Arc<dyn StorageBackend>,
        table_id: &TableId,
        column_name: &str,
    ) -> IndexedEntityStore<Self::VectorOpId, VectorHotOp> {
        new_indexed_shared_vector_hot_store(backend, table_id, column_name)
    }

    fn new_auth() -> Self::Auth {
        SharedTableAuthorization::new()
    }

    fn storage_user<'a>(_actor: Option<&'a UserId>) -> Result<Option<&'a UserId>, KalamDbError> {
        Ok(None)
    }

    fn make_row_id(_user: Option<&UserId>, version: VersionId) -> Self::RowId {
        version
    }

    fn make_row(
        _user: Option<&UserId>,
        version: VersionId,
        deleted: bool,
        fields: Row,
    ) -> Self::Row {
        SharedTableRow {
            _version: version,
            _deleted: deleted,
            fields,
        }
    }

    fn row_id_version(id: &Self::RowId) -> VersionId {
        *id
    }

    fn row_fields(row: &Self::Row) -> &Row {
        &row.fields
    }

    fn row_version(row: &Self::Row) -> VersionId {
        row._version
    }

    fn row_deleted(row: &Self::Row) -> bool {
        row._deleted
    }

    fn pk_prefix(
        index: &VersionedPkIndex,
        _user: Option<&UserId>,
        pk_value: &ScalarValue,
    ) -> Vec<u8> {
        index.build_shared_pk_prefix(pk_value)
    }

    fn vector_op_id(_user: Option<&UserId>, seq: SeqId, pk: String) -> Self::VectorOpId {
        SharedVectorHotOpId::new(seq, pk)
    }

    fn decode_version(bytes: &[u8]) -> Option<VersionId> {
        crate::versioned::columnar::decode_shared_version(bytes)
    }

    fn row_from_parquet(
        _user_id: &UserId,
        row_data: &crate::utils::version_resolution::ParquetRowData,
    ) -> (Self::RowId, Self::Row) {
        let row = SharedTableRow {
            _version: row_data.version,
            _deleted: row_data.deleted,
            fields: row_data.fields.clone(),
        };
        (row_data.version, row)
    }
}
