//! Process-local claims on primary keys for explicit transactions.
//!
//! Two transactions can both read a committed row and then delete it. PostgreSQL
//! makes the second `DELETE` wait, then recheck the row, so it reports zero rows
//! when the first transaction already removed it. These claims are that wait.
//! They last until the owning transaction commits, rolls back, or times out.

use std::{
    collections::HashMap,
    pin::pin,
    sync::{Arc, Mutex, OnceLock},
};

use kalamdb_commons::models::{TableId, TransactionId};
use tokio::sync::Notify;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ClaimKey {
    table_id:    TableId,
    primary_key: String,
}

#[derive(Debug)]
struct Claim {
    owner:  TransactionId,
    notify: Arc<Notify>,
}

/// Primary-key claims shared by every explicit transaction on this process.
#[derive(Clone, Debug, Default)]
pub struct PrimaryKeyClaims {
    inner: Arc<Mutex<HashMap<ClaimKey, Claim>>>,
}

impl PrimaryKeyClaims {
    /// Wait until `owner` is the only transaction holding `(table_id, primary_key)`.
    ///
    /// A second acquire by the same owner returns immediately.
    pub async fn acquire(&self, table_id: &TableId, primary_key: &str, owner: &TransactionId) {
        let key = ClaimKey {
            table_id:    table_id.clone(),
            primary_key: primary_key.to_string(),
        };
        loop {
            let wait = {
                let mut claims = self.lock();
                match claims.get(&key) {
                    Some(claim) if &claim.owner == owner => return,
                    Some(claim) => Arc::clone(&claim.notify),
                    None => {
                        claims.insert(
                            key,
                            Claim {
                                owner:  owner.clone(),
                                notify: Arc::new(Notify::new()),
                            },
                        );
                        return;
                    },
                }
            };
            let mut notified = pin!(wait.notified());
            notified.as_mut().enable();
            let still_blocked = self.lock().get(&key).is_some_and(|claim| &claim.owner != owner);
            if still_blocked {
                notified.await;
            }
        }
    }

    /// Drop one key when the committed row no longer matches the statement.
    pub fn release_key(&self, table_id: &TableId, primary_key: &str, owner: &TransactionId) {
        let key = ClaimKey {
            table_id:    table_id.clone(),
            primary_key: primary_key.to_string(),
        };
        let mut claims = self.lock();
        let owned = claims.get(&key).is_some_and(|claim| &claim.owner == owner);
        if owned {
            if let Some(claim) = claims.remove(&key) {
                claim.notify.notify_waiters();
            }
        }
    }

    /// Drop every key owned by `owner`.
    pub fn release_owner(&self, owner: &TransactionId) {
        let mut claims = self.lock();
        claims.retain(|_, claim| {
            if &claim.owner == owner {
                claim.notify.notify_waiters();
                false
            } else {
                true
            }
        });
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ClaimKey, Claim>> {
        self.inner.lock().expect("primary key claim lock poisoned")
    }
}

/// Claims used by SQL transactions on this process.
pub fn primary_key_claims() -> PrimaryKeyClaims {
    static CLAIMS: OnceLock<PrimaryKeyClaims> = OnceLock::new();
    CLAIMS.get_or_init(PrimaryKeyClaims::default).clone()
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::{
        models::{schemas::TableName, TableId, TransactionId},
        SchemaId,
    };

    use super::*;

    fn table() -> TableId {
        TableId::new(SchemaId::new("app"), TableName::new("sessions"))
    }

    fn transaction_id() -> TransactionId {
        TransactionId::new(uuid::Uuid::now_v7().to_string())
    }

    #[tokio::test]
    async fn second_owner_waits_until_the_first_releases() {
        let claims = PrimaryKeyClaims::default();
        let table_id = table();
        let first = transaction_id();
        let second = transaction_id();
        claims.acquire(&table_id, "token", &first).await;

        let waiting = claims.clone();
        let table_for_waiter = table_id.clone();
        let waiter = tokio::spawn(async move {
            waiting.acquire(&table_for_waiter, "token", &second).await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(!waiter.is_finished());

        claims.release_owner(&first);
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("waiter timed out")
            .expect("waiter task");
    }

    #[tokio::test]
    async fn same_owner_acquires_again_without_waiting() {
        let claims = PrimaryKeyClaims::default();
        let table_id = table();
        let owner = transaction_id();
        claims.acquire(&table_id, "token", &owner).await;
        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            claims.acquire(&table_id, "token", &owner),
        )
        .await
        .expect("same owner should not wait");
        claims.release_owner(&owner);
    }
}
