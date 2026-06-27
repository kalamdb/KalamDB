//! TOCTOU-safe local file reads and atomic writes for CLI-owned paths.
//!
//! Reads open the target in one step (with `O_NOFOLLOW` on Unix) instead of
//! checking existence separately. Writes go to a same-directory temp file, fsync,
//! then rename into place so concurrent readers never observe partial content.

use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

/// Controls how strictly local file reads are validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileReadPolicy {
    /// Files under the CLI home directory (config, credentials, history).
    LocalSecrets,
    /// User-supplied paths such as SQL `FILE()` arguments.
    UserProvided,
}

/// Options for [`write_atomic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileWriteOptions {
    /// Unix mode bits applied to the temp file before rename (default `0o644`).
    pub unix_mode: Option<u32>,
}

impl FileWriteOptions {
    pub const DEFAULT: Self = Self { unix_mode: None };
    pub const SECRET_FILE: Self = Self { unix_mode: Some(0o600) };
}

/// Read a UTF-8 file, returning `Ok(None)` when the path does not exist.
pub fn read_to_string_if_exists(path: &Path, policy: FileReadPolicy) -> io::Result<Option<String>> {
    match read_to_string(path, policy) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Read a UTF-8 file in one step, rejecting symlinks on Unix.
pub fn read_to_string(path: &Path, policy: FileReadPolicy) -> io::Result<String> {
    let bytes = read_bytes(path, policy)?;
    String::from_utf8(bytes).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("file '{}' is not valid UTF-8: {error}", path.display()),
        )
    })
}

/// Read raw bytes in one step, rejecting symlinks on Unix.
pub fn read_bytes(path: &Path, policy: FileReadPolicy) -> io::Result<Vec<u8>> {
    reject_symlink_before_read(path, policy)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)?;
        return Ok(contents);
    }

    #[cfg(not(unix))]
    {
        fs::read(path)
    }
}

/// Write `contents` atomically via a same-directory temp file and rename.
pub fn write_atomic(path: &Path, contents: &[u8], options: FileWriteOptions) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let staging_path = staging_path_for(path);
    write_staging_file(&staging_path, contents, options)?;

    match fs::rename(&staging_path, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&staging_path);
            Err(error)
        }
    }
}

fn staging_path_for(destination: &Path) -> PathBuf {
    let pid = std::process::id();
    let nonce: u32 = rand::random();
    destination.with_extension(format!("kalam-{pid}-{nonce:08x}.tmp"))
}

fn write_staging_file(path: &Path, contents: &[u8], options: FileWriteOptions) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let mode = options.unix_mode.unwrap_or(0o644);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(path)?;
        file.write_all(contents)?;
        file.sync_all()?;
        return Ok(());
    }

    #[cfg(not(unix))]
    {
        fs::write(path, contents)
    }
}

#[cfg(windows)]
fn reject_symlink_before_read(path: &Path, _policy: FileReadPolicy) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to read symlink '{}'", path.display()),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn reject_symlink_before_read(_path: &Path, _policy: FileReadPolicy) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn read_missing_file_returns_not_found() {
        let temp = TempDir::new().expect("tempdir");
        let missing = temp.path().join("missing.txt");
        let error = read_to_string(&missing, FileReadPolicy::LocalSecrets).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn read_to_string_if_exists_returns_none_for_missing_file() {
        let temp = TempDir::new().expect("tempdir");
        let missing = temp.path().join("missing.txt");
        let contents = read_to_string_if_exists(&missing, FileReadPolicy::LocalSecrets).expect("read");
        assert!(contents.is_none());
    }

    #[test]
    fn write_atomic_round_trips_content() {
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("config.toml");
        write_atomic(path.as_path(), b"hello", FileWriteOptions::DEFAULT).expect("write");
        let contents = read_to_string(&path, FileReadPolicy::LocalSecrets).expect("read");
        assert_eq!(contents, "hello");
    }

    #[test]
    fn write_atomic_overwrites_existing_file() {
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("history");
        write_atomic(path.as_path(), b"old", FileWriteOptions::DEFAULT).expect("write old");
        write_atomic(path.as_path(), b"new", FileWriteOptions::DEFAULT).expect("write new");
        let contents = read_to_string(&path, FileReadPolicy::LocalSecrets).expect("read");
        assert_eq!(contents, "new");
    }

    #[cfg(unix)]
    #[test]
    fn read_rejects_symlink_targets() {
        let temp = TempDir::new().expect("tempdir");
        let target = temp.path().join("secret.toml");
        fs::write(&target, b"token").expect("write target");
        let link = temp.path().join("link.toml");
        symlink(&target, &link).expect("symlink");

        let error = read_to_string(&link, FileReadPolicy::UserProvided).unwrap_err();
        assert_ne!(error.kind(), io::ErrorKind::NotFound);
    }

    #[cfg(unix)]
    #[test]
    fn secret_write_sets_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("credentials.toml");
        write_atomic(path.as_path(), b"secret", FileWriteOptions::SECRET_FILE).expect("write");
        let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
