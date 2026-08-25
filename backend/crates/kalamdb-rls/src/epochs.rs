use kalamdb_commons::TableId;

/// Generation source for a shared membership / relation table.
pub trait MembershipEpoch: Send + Sync {
    fn authorization_generation(&self) -> u64;
    fn has_active_authorization_mutations(&self) -> bool;
}

/// Generation source for the policy catalog of a protected table.
pub trait PolicyCatalogEpoch: Send + Sync {
    fn policy_generation(&self, table_id: &TableId) -> Result<u64, String>;
}
