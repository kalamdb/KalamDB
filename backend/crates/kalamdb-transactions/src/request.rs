use std::{
    collections::hash_map::DefaultHasher,
    error::Error,
    fmt,
    future::Future,
    hash::{Hash, Hasher},
    sync::Arc,
};

use kalamdb_commons::models::TransactionId;

use crate::ExecutionOwnerKey;

pub const OPEN_REQUEST_TRANSACTION_ROLLED_BACK_MESSAGE: &str =
    "Request completed with an open explicit transaction; rolled back automatically";

pub trait RequestTransactionCoordinator {
    type Error;

    fn active_for_owner(&self, owner_key: &ExecutionOwnerKey) -> Option<TransactionId>;

    fn begin_sql_request(
        &self,
        owner_key: ExecutionOwnerKey,
        owner_id: Arc<str>,
    ) -> Result<TransactionId, Self::Error>;

    fn commit_request_transaction<'a>(
        &'a self,
        transaction_id: &'a TransactionId,
    ) -> impl Future<Output = Result<TransactionId, Self::Error>> + Send + 'a;

    fn rollback_request_transaction(
        &self,
        transaction_id: &TransactionId,
    ) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestTransactionError<E> {
    AlreadyActive {
        owner_id: String,
        transaction_id: TransactionId,
    },
    NoActiveTransaction {
        operation: &'static str,
    },
    RequestCompletedOpen,
    Coordinator(E),
}

impl<E: fmt::Display> fmt::Display for RequestTransactionError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyActive {
                owner_id,
                transaction_id,
            } => write!(
                f,
                "request owner '{}' already has an active transaction '{}'",
                owner_id, transaction_id
            ),
            Self::NoActiveTransaction { operation } => write!(
                f,
                "{} requires an active explicit SQL transaction",
                operation
            ),
            Self::RequestCompletedOpen => f.write_str(OPEN_REQUEST_TRANSACTION_ROLLED_BACK_MESSAGE),
            Self::Coordinator(error) => write!(f, "{}", error),
        }
    }
}

impl<E> Error for RequestTransactionError<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Coordinator(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RequestTransactionState<'a> {
    owner_key: ExecutionOwnerKey,
    request_id: &'a str,
    active_transaction_id: Option<TransactionId>,
}

impl<'a> RequestTransactionState<'a> {
    pub fn from_request_id(request_id: Option<&'a str>) -> Option<Self> {
        request_id.map(|request_id| Self {
            owner_key: Self::owner_key_for_request_id(request_id),
            request_id,
            active_transaction_id: None,
        })
    }

    pub fn owner_key_for_request_id(request_id: &str) -> ExecutionOwnerKey {
        let mut hasher = DefaultHasher::new();
        request_id.hash(&mut hasher);
        ExecutionOwnerKey::sql_request(hasher.finish())
    }

    #[inline]
    pub fn owner_key(&self) -> ExecutionOwnerKey {
        self.owner_key
    }

    #[inline]
    pub fn active_transaction_id(&self) -> Option<&TransactionId> {
        self.active_transaction_id.as_ref()
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.active_transaction_id.is_some()
    }

    pub fn sync<C>(&mut self, coordinator: &C)
    where
        C: RequestTransactionCoordinator,
    {
        self.active_transaction_id = coordinator.active_for_owner(&self.owner_key);
    }

    pub fn begin<C>(
        &mut self,
        coordinator: &C,
    ) -> Result<TransactionId, RequestTransactionError<C::Error>>
    where
        C: RequestTransactionCoordinator,
    {
        if let Some(transaction_id) = self.active_transaction_id.clone() {
            return Err(RequestTransactionError::AlreadyActive {
                owner_id: self.owner_id_string(),
                transaction_id,
            });
        }

        let transaction_id = coordinator
            .begin_sql_request(self.owner_key, self.owner_id())
            .map_err(RequestTransactionError::Coordinator)?;
        self.active_transaction_id = Some(transaction_id.clone());
        Ok(transaction_id)
    }

    pub async fn commit<C>(
        &mut self,
        coordinator: &C,
    ) -> Result<TransactionId, RequestTransactionError<C::Error>>
    where
        C: RequestTransactionCoordinator,
    {
        let transaction_id = self.active_transaction_id.clone().ok_or(
            RequestTransactionError::NoActiveTransaction {
                operation: "COMMIT",
            },
        )?;

        let committed = coordinator
            .commit_request_transaction(&transaction_id)
            .await
            .map_err(RequestTransactionError::Coordinator)?;
        self.active_transaction_id = None;
        Ok(committed)
    }

    pub fn rollback<C>(
        &mut self,
        coordinator: &C,
    ) -> Result<TransactionId, RequestTransactionError<C::Error>>
    where
        C: RequestTransactionCoordinator,
    {
        let transaction_id = self.active_transaction_id.clone().ok_or(
            RequestTransactionError::NoActiveTransaction {
                operation: "ROLLBACK",
            },
        )?;

        coordinator
            .rollback_request_transaction(&transaction_id)
            .map_err(RequestTransactionError::Coordinator)?;
        self.active_transaction_id = None;
        Ok(transaction_id)
    }

    pub fn rollback_if_active<C>(
        &mut self,
        coordinator: &C,
    ) -> Result<Option<TransactionId>, RequestTransactionError<C::Error>>
    where
        C: RequestTransactionCoordinator,
    {
        if !self.is_active() {
            return Ok(None);
        }

        self.rollback(coordinator).map(Some)
    }

    #[inline]
    fn owner_id(&self) -> Arc<str> {
        Arc::<str>::from(self.owner_id_string())
    }

    #[inline]
    fn owner_id_string(&self) -> String {
        format!("sql-req-{}", self.request_id)
    }
}

#[derive(Debug)]
pub struct RequestTransactionBatchGuard<'a> {
    state: Option<RequestTransactionState<'a>>,
}

impl<'a> RequestTransactionBatchGuard<'a> {
    pub fn from_request_id<C>(request_id: Option<&'a str>, coordinator: &C) -> Self
    where
        C: RequestTransactionCoordinator,
    {
        let mut state = RequestTransactionState::from_request_id(request_id);
        if let Some(request_state) = state.as_mut() {
            request_state.sync(coordinator);
        }

        Self { state }
    }

    #[inline]
    pub fn active_transaction_id(&self) -> Option<&TransactionId> {
        self.state
            .as_ref()
            .and_then(RequestTransactionState::active_transaction_id)
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.state.as_ref().is_some_and(RequestTransactionState::is_active)
    }

    pub fn sync<C>(&mut self, coordinator: &C)
    where
        C: RequestTransactionCoordinator,
    {
        if let Some(state) = self.state.as_mut() {
            state.sync(coordinator);
        }
    }

    pub fn rollback_if_active<C>(
        &mut self,
        coordinator: &C,
    ) -> Result<Option<TransactionId>, RequestTransactionError<C::Error>>
    where
        C: RequestTransactionCoordinator,
    {
        match self.state.as_mut() {
            Some(state) => state.rollback_if_active(coordinator),
            None => Ok(None),
        }
    }

    pub fn ensure_closed<C>(
        &mut self,
        coordinator: &C,
    ) -> Result<(), RequestTransactionError<C::Error>>
    where
        C: RequestTransactionCoordinator,
    {
        if !self.is_active() {
            return Ok(());
        }

        self.rollback_if_active(coordinator)?;
        Err(RequestTransactionError::RequestCompletedOpen)
    }
}
