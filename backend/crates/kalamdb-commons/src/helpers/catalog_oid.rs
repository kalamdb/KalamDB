//! Stable PostgreSQL catalog OIDs shared by the SQL dialect and `pg_catalog` views.
//!
//! `'name'::regnamespace` and `pg_namespace.oid` must hash the same key. The
//! hasher is process-local; both callers have to use this function.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

/// 31-bit positive OID derived from catalog key parts.
#[inline]
pub fn stable_catalog_oid(parts: &[&str]) -> i64 {
    let mut hasher = DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
    (hasher.finish() & 0x7fff_ffff) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_catalog_oid_is_a_positive_31_bit_value() {
        let namespace = stable_catalog_oid(&["namespace", "directus"]);
        assert!(namespace > 0);
        assert!(namespace <= 0x7fff_ffff);
        assert_eq!(namespace, stable_catalog_oid(&["namespace", "directus"]));
        assert_ne!(namespace, stable_catalog_oid(&["directus"]));
    }
}
