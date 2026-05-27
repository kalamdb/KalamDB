use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::error::{Result, StreamLogError};

pub(crate) fn visit_dirs<F>(path: &Path, mut visitor: F) -> Result<bool>
where
    F: FnMut(PathBuf) -> Result<bool>,
{
    for entry in fs::read_dir(path).map_err(|e| StreamLogError::Io(e.to_string()))? {
        let entry = entry.map_err(|e| StreamLogError::Io(e.to_string()))?;
        let path = entry.path();
        if path.is_dir() {
            if !visitor(path)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

pub(crate) fn cleanup_empty_dir(path: &Path) {
    if let Ok(mut entries) = fs::read_dir(path) {
        if entries.next().is_none() {
            let _ = fs::remove_dir(path);
        }
    }
}
