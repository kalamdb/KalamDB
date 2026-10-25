use serde::{Deserialize, Serialize};

/// Trait for extracting index keys from entities.
///
/// Implement this trait to define how to extract an index key from your entity type.
///
/// ## Type Parameters
/// - `T`: Entity type (must be Serialize + Deserialize + Send + Sync)
/// - `K`: Index key type (must be Clone + AsRef<[u8]> + Send + Sync)
pub trait IndexKeyExtractor<T, K>: Send + Sync
where
    T: Serialize + for<'de> Deserialize<'de> + Send + Sync,
    K: Clone + AsRef<[u8]> + Send + Sync,
{
    /// Extracts the index key from an entity.
    ///
    /// # Arguments
    /// * `entity` - The entity to extract the key from
    ///
    /// # Returns
    /// The extracted index key
    fn extract(&self, entity: &T) -> K;
}

/// Function-based index key extractor.
///
/// Allows using closures/functions as key extractors without manual trait implementation.
pub struct FunctionExtractor<T, K, F>
where
    T: Serialize + for<'de> Deserialize<'de> + Send + Sync,
    K: Clone + AsRef<[u8]> + Send + Sync,
    F: Fn(&T) -> K + Send + Sync,
{
    func: F,
    _phantom_t: std::marker::PhantomData<T>,
    _phantom_k: std::marker::PhantomData<K>,
}

impl<T, K, F> FunctionExtractor<T, K, F>
where
    T: Serialize + for<'de> Deserialize<'de> + Send + Sync,
    K: Clone + AsRef<[u8]> + Send + Sync,
    F: Fn(&T) -> K + Send + Sync,
{
    /// Creates a new function-based extractor.
    pub fn new(func: F) -> Self {
        Self {
            func,
            _phantom_t: std::marker::PhantomData,
            _phantom_k: std::marker::PhantomData,
        }
    }
}

impl<T, K, F> IndexKeyExtractor<T, K> for FunctionExtractor<T, K, F>
where
    T: Serialize + for<'de> Deserialize<'de> + Send + Sync,
    K: Clone + AsRef<[u8]> + Send + Sync,
    F: Fn(&T) -> K + Send + Sync,
{
    fn extract(&self, entity: &T) -> K {
        (self.func)(entity)
    }
}
