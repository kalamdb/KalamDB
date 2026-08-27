//! File metadata helpers for table manifests.

pub use kalamdb_commons::FileRef;
use serde::{Deserialize, Serialize};

const SUBFOLDER_PREFIX: &str = "f";

/// File subfolder tracking for manifest storage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileSubfolderState {
    /// Current subfolder index (1-based: f0001, f0002, ...).
    pub subfolder: u32,
    /// Number of files in current subfolder.
    pub count:     u32,
}

impl FileSubfolderState {
    /// Create new state starting at subfolder 1.
    pub fn new() -> Self {
        Self {
            subfolder: 1,
            count:     0,
        }
    }

    /// Get current subfolder name, for example `"f0001"`.
    pub fn subfolder_name(&self) -> String {
        format!("{}{:04}", SUBFOLDER_PREFIX, self.subfolder)
    }

    /// Increment file count and rotate subfolder if needed.
    ///
    /// Returns the subfolder name to use for the new file.
    pub fn allocate_file(&mut self, max_files_per_folder: u32) -> String {
        if self.count >= max_files_per_folder {
            self.subfolder += 1;
            self.count = 0;
        }
        self.count += 1;
        self.subfolder_name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_ref_reexport_preserves_storage_helpers() {
        let file_ref = FileRef::with_shard(
            "12345".to_string(),
            "f0001".to_string(),
            "test.pdf".to_string(),
            1024,
            "application/pdf".to_string(),
            "abc123".to_string(),
            3,
        );

        assert_eq!(file_ref.relative_path(), "shard-3/f0001/12345-test.pdf");
    }

    #[test]
    fn subfolder_state_rotates_when_limit_is_reached() {
        let mut state = FileSubfolderState::new();
        assert_eq!(state.subfolder_name(), "f0001");

        for _ in 0..5 {
            state.allocate_file(10);
        }
        assert_eq!(state.count, 5);
        assert_eq!(state.subfolder, 1);

        for _ in 0..6 {
            state.allocate_file(10);
        }
        assert_eq!(state.subfolder, 2);
        assert_eq!(state.subfolder_name(), "f0002");
    }
}
