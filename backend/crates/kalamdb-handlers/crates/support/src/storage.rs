use kalamdb_core::error::KalamDbError;

/// Ensure a filesystem directory exists for filesystem-backed storage definitions.
pub fn ensure_filesystem_directory(path: &str) -> Result<(), KalamDbError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(KalamDbError::InvalidOperation(
            "Filesystem storage requires a non-empty base directory".to_string(),
        ));
    }

    std::fs::create_dir_all(trimmed).map_err(|e| {
        KalamDbError::InvalidOperation(format!(
            "Failed to create filesystem storage directory '{}': {}",
            trimmed, e
        ))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{env, fs};

    use kalamdb_core::error::KalamDbError;
    use uuid::Uuid;

    use super::ensure_filesystem_directory;

    #[test]
    fn ensure_filesystem_directory_rejects_empty_path() {
        let err = ensure_filesystem_directory("   ").expect_err("empty path must fail");
        assert!(matches!(err, KalamDbError::InvalidOperation(_)));
    }

    #[test]
    fn ensure_filesystem_directory_creates_and_is_idempotent() {
        let test_dir = env::temp_dir().join(format!("kalamdb-storage-test-{}", Uuid::new_v4()));
        let test_dir_str = test_dir.to_string_lossy().to_string();

        // First invocation creates the directory.
        ensure_filesystem_directory(&test_dir_str).expect("directory creation should succeed");
        assert!(test_dir.is_dir(), "directory should exist after creation");

        // Second invocation should be harmless.
        ensure_filesystem_directory(&test_dir_str).expect("second creation should also succeed");

        fs::remove_dir_all(&test_dir).expect("cleanup temporary directory");
    }
}
