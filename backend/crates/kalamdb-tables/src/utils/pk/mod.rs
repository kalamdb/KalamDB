//! Primary Key Existence Check Service
//!
//! Provides optimized PK uniqueness validation for INSERT/UPDATE operations.
//!
//! ## Flow:
//! 1. Check if PK column is AUTO_INCREMENT → skip check (system generates unique value)
//! 2. Check ManifestService memory cache for manifest
//! 3. Check RocksDB persisted manifest copy
//! 4. Fall back to ManifestService storage manifest.json lookup
//! 5. Use column_stats min/max to prune segments that can't contain the PK
//! 6. Only scan relevant Parquet files if PK is in range
//!
//! ## Performance Characteristics:
//! - Auto-increment PKs: O(1) - immediate skip
//! - Memory cache hit: O(1) - manifest in memory
//! - Warm cache hit: O(1) - manifest in RocksDB (fast SSD read)
//! - Cold manifest: O(n) where n = manifest.json file size
//! - Segment scan: O(m) where m = rows in matching segments

mod existence_checker;
pub mod helpers;

pub use existence_checker::{PkCheckResult, PkExistenceChecker};
pub use helpers::parse_pk_value;
