//! File and path helper utilities
//!
//! Centralized utilities for file system operations including:
//! - Path normalization (relative to absolute)
//! - Path joining with proper separators
//! - Directory creation

use std::path::{Path, PathBuf};

/// Normalize a directory path to absolute form.
///
/// - If path is already absolute, return as-is
/// - If path is relative, resolve against current working directory
/// - Empty paths are returned unchanged
///
/// # Examples
///
/// ```
/// use std::path::Path;
///
/// use kalamdb_configs::file_helpers::normalize_dir_path;
///
/// let abs = normalize_dir_path("./data");
/// assert!(abs.is_empty() || Path::new(&abs).is_absolute());
/// ```
pub fn normalize_dir_path(path: &str) -> String {
    if path.trim().is_empty() {
        return path.to_string();
    }

    let p = Path::new(path);
    if p.is_absolute() {
        path.to_string()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(p)
            .to_string_lossy()
            .into_owned()
    }
}

/// Join a base directory with a subdirectory name.
///
/// Returns a PathBuf with the subdirectory appended to the base path.
///
/// # Examples
///
/// ```
/// use kalamdb_configs::file_helpers::join_path;
///
/// let result = join_path("./data", "rocksdb");
/// assert_eq!(result, std::path::Path::new("./data").join("rocksdb"));
/// ```
pub fn join_path<P: AsRef<Path>, S: AsRef<Path>>(base: P, subdir: S) -> PathBuf {
    base.as_ref().join(subdir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_empty_path() {
        assert_eq!(normalize_dir_path(""), "");
        assert_eq!(normalize_dir_path("   "), "   ");
    }

    #[test]
    fn test_normalize_absolute_path() {
        #[cfg(unix)]
        {
            let abs_path = "/tmp/data";
            assert_eq!(normalize_dir_path(abs_path), abs_path);
        }

        #[cfg(windows)]
        {
            let abs_path = "C:/tmp/data";
            assert_eq!(normalize_dir_path(abs_path), abs_path);
        }
    }

    #[test]
    fn test_join_path() {
        let base = "./data";
        let subdir = "rocksdb";
        let result = join_path(base, subdir);
        assert_eq!(result, PathBuf::from("./data/rocksdb"));
    }
}
