//! SNOWFLAKE_ID() function implementation
//!
//! This module provides a user-defined function for DataFusion that generates
//! distributed unique 64-bit IDs following the Snowflake ID format:
//! - 41 bits: timestamp in milliseconds since epoch
//! - 10 bits: node/machine ID
//! - 12 bits: sequence number
//!
//! This ensures time-ordered, unique IDs suitable for PRIMARY KEY columns.

use std::{
    any::Any,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use datafusion::{
    arrow::array::{ArrayRef, Int64Array},
    error::{DataFusionError, Result as DataFusionResult},
    logical_expr::{ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature, Volatility},
};
use kalamdb_commons::arrow_utils::{arrow_int64, ArrowDataType};

// Snowflake ID format constants
const TIMESTAMP_BITS: u64 = 41;
const NODE_ID_BITS: u64 = 10;
const SEQUENCE_BITS: u64 = 12;

const MAX_NODE_ID: u16 = (1 << NODE_ID_BITS) - 1; // 1023
const MAX_SEQUENCE: u16 = (1 << SEQUENCE_BITS) - 1; // 4095

const TIMESTAMP_SHIFT: u64 = NODE_ID_BITS + SEQUENCE_BITS; // 22
const NODE_ID_SHIFT: u64 = SEQUENCE_BITS; // 12

/// Global sequence counter for Snowflake ID generation
static LAST_TIMESTAMP_AND_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// SNOWFLAKE_ID() scalar function implementation
///
/// Generates a 64-bit distributed unique identifier with time-ordering properties.
/// Format: [41-bit timestamp][10-bit node_id][12-bit sequence]
///
/// # Returns
/// - BIGINT (Int64) - A unique 64-bit identifier
///
/// # Properties
/// - VOLATILE: Generates a new ID on each invocation
/// - Thread-safe: Uses atomic operations for sequence counter
/// - Time-ordered: IDs increase monotonically with time
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SnowflakeIdFunction {
    node_id: u16,
}

impl SnowflakeIdFunction {
    /// Create a new SNOWFLAKE_ID function with default node ID (0)
    pub fn new() -> Self {
        Self { node_id: 0 }
    }

    /// Create a SNOWFLAKE_ID function with a specific node ID
    ///
    /// # Arguments
    /// * `node_id` - Node identifier (0-1023)
    ///
    /// # Panics
    /// Panics if node_id exceeds MAX_NODE_ID (1023)
    pub fn with_node_id(node_id: u16) -> Self {
        assert!(node_id <= MAX_NODE_ID, "Node ID must be between 0 and {}", MAX_NODE_ID);
        Self { node_id }
    }

    /// Generate a single Snowflake ID
    fn generate_id(&self) -> i64 {
        let (timestamp, sequence) = next_timestamp_and_sequence();

        // Combine components into 64-bit ID
        let id = ((timestamp & ((1 << TIMESTAMP_BITS) - 1)) << TIMESTAMP_SHIFT)
            | ((self.node_id as u64) << NODE_ID_SHIFT)
            | (sequence as u64);

        id as i64
    }
}

fn current_timestamp_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis() as u64
}

fn pack_timestamp_and_sequence(timestamp: u64, sequence: u16) -> u64 {
    (timestamp << SEQUENCE_BITS) | sequence as u64
}

fn unpack_timestamp_and_sequence(value: u64) -> (u64, u16) {
    (value >> SEQUENCE_BITS, (value & MAX_SEQUENCE as u64) as u16)
}

fn wait_for_next_timestamp(last_timestamp: u64) -> u64 {
    loop {
        let timestamp = current_timestamp_millis();
        if timestamp > last_timestamp {
            return timestamp;
        }
        thread::yield_now();
    }
}

fn next_timestamp_and_sequence() -> (u64, u16) {
    loop {
        let now = current_timestamp_millis();
        let observed = LAST_TIMESTAMP_AND_SEQUENCE.load(Ordering::Acquire);
        let (last_timestamp, last_sequence) = unpack_timestamp_and_sequence(observed);

        let (next_timestamp, next_sequence) = if now > last_timestamp {
            (now, 0)
        } else if last_sequence < MAX_SEQUENCE {
            (last_timestamp, last_sequence + 1)
        } else {
            (wait_for_next_timestamp(last_timestamp), 0)
        };

        let next_state = pack_timestamp_and_sequence(next_timestamp, next_sequence);

        if LAST_TIMESTAMP_AND_SEQUENCE
            .compare_exchange(observed, next_state, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return (next_timestamp, next_sequence);
        }
    }
}

impl Default for SnowflakeIdFunction {
    fn default() -> Self {
        Self::new()
    }
}

impl ScalarUDFImpl for SnowflakeIdFunction {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn name(&self) -> &str {
        "snowflake_id"
    }

    fn signature(&self) -> &Signature {
        // Static signature with no arguments
        static SIGNATURE: std::sync::OnceLock<Signature> = std::sync::OnceLock::new();
        SIGNATURE.get_or_init(|| Signature::exact(vec![], Volatility::Volatile))
    }

    fn return_type(&self, _args: &[ArrowDataType]) -> DataFusionResult<ArrowDataType> {
        Ok(arrow_int64())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DataFusionResult<ColumnarValue> {
        if !args.args.is_empty() {
            return Err(DataFusionError::Plan("SNOWFLAKE_ID() takes no arguments".to_string()));
        }
        let mut ids = Vec::with_capacity(args.number_rows);
        for _ in 0..args.number_rows {
            ids.push(self.generate_id());
        }
        let array = Int64Array::from(ids);
        Ok(ColumnarValue::Array(Arc::new(array) as ArrayRef))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use datafusion::logical_expr::ScalarUDF;

    use super::*;

    #[test]
    fn test_snowflake_id_function_creation() {
        let func_impl = SnowflakeIdFunction::new();
        let func = ScalarUDF::new_from_impl(func_impl);
        assert_eq!(func.name(), "snowflake_id");
    }

    #[test]
    fn test_snowflake_id_with_node_id() {
        let func_impl = SnowflakeIdFunction::with_node_id(123);
        assert_eq!(func_impl.node_id, 123);
    }

    #[test]
    #[should_panic(expected = "Node ID must be between 0 and 1023")]
    fn test_snowflake_id_invalid_node_id() {
        SnowflakeIdFunction::with_node_id(1024);
    }

    #[test]
    fn test_snowflake_id_generation() {
        let func_impl = SnowflakeIdFunction::new();
        let id1 = func_impl.generate_id();
        let id2 = func_impl.generate_id();

        assert!(id1 > 0);
        assert!(id2 > id1, "IDs should be monotonically increasing");
    }

    #[test]
    fn test_snowflake_id_uniqueness() {
        let func_impl = SnowflakeIdFunction::new();
        let mut ids = HashSet::new();

        // Generate 10000 IDs and ensure no duplicates
        for _ in 0..10000 {
            let id = func_impl.generate_id();
            assert!(ids.insert(id), "Duplicate ID detected: {}", id);
        }
    }

    #[test]
    fn test_snowflake_id_timestamp_component() {
        let func_impl = SnowflakeIdFunction::new();
        let id = func_impl.generate_id();

        // Extract timestamp (top 41 bits)
        let timestamp = (id as u64) >> TIMESTAMP_SHIFT;

        // Current timestamp in milliseconds
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;

        // Timestamp should be close to now (within 1 second)
        assert!(
            timestamp.abs_diff(now) < 1000,
            "Timestamp component should be close to current time"
        );
    }

    #[test]
    fn test_snowflake_id_node_component() {
        let node_id = 456;
        let func_impl = SnowflakeIdFunction::with_node_id(node_id);
        let id = func_impl.generate_id();

        // Extract node ID (bits 12-21)
        let extracted_node = ((id as u64) >> NODE_ID_SHIFT) & MAX_NODE_ID as u64;
        assert_eq!(extracted_node, node_id as u64);
    }

    #[test]
    fn test_snowflake_id_invoke() {
        let func_impl = SnowflakeIdFunction::new();
        let id = func_impl.generate_id();
        assert!(id > 0);
    }

    #[test]
    // Removed: direct invoke with arguments test due to DataFusion API changes
    fn test_snowflake_id_return_type() {
        let func_impl = SnowflakeIdFunction::new();
        let return_type = func_impl.return_type(&[]);
        assert!(return_type.is_ok());
        assert_eq!(return_type.unwrap(), ArrowDataType::Int64);
    }
}
