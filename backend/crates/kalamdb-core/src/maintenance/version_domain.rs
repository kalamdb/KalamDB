//! Operator-controlled move onto a fresh `VersionId` history.
//!
//! Startup never calls this. A fresh domain import does not delete the source
//! data; it only proceeds when the operator sets the consent flag.

/// Steps an operator must run before cutting over to a new version domain.
pub const FRESH_DOMAIN_IMPORT_STEPS: &[&str] = &[
    "take and verify a backup",
    "quiesce writes",
    "export visible rows, business keys, and stream timestamps",
    "import through the new versioning path into a fresh history",
    "invalidate old resume tokens",
    "verify counts, ownership, and schema identity before cutover",
    "keep the old backup for rollback",
];

/// Why a fresh-domain import was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FreshDomainImportError {
    /// The operator did not set the consent flag.
    ConsentRequired,
}

impl std::fmt::Display for FreshDomainImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConsentRequired => write!(
                f,
                "refusing to open a fresh version domain without explicit operator consent"
            ),
        }
    }
}

/// Plan a fresh version-domain import.
///
/// This does not read or delete stored rows. Consent only unlocks the plan.
pub fn plan_fresh_domain_import(confirm: bool) -> Result<&'static [&'static str], FreshDomainImportError> {
    if !confirm {
        return Err(FreshDomainImportError::ConsentRequired);
    }
    Ok(FRESH_DOMAIN_IMPORT_STEPS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_refuses_without_consent() {
        assert_eq!(
            plan_fresh_domain_import(false),
            Err(FreshDomainImportError::ConsentRequired)
        );
    }

    #[test]
    fn consented_plan_does_not_reset_data() {
        let steps = plan_fresh_domain_import(true).unwrap();
        assert!(steps.iter().any(|step| step.contains("backup")));
        assert!(steps.iter().all(|step| !step.contains("delete")));
    }
}
