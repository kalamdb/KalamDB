use std::{
    fs::{self, File},
    path::{Component, Path, PathBuf},
};

use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use kalamdb_core::error::KalamDbError;
use tar::{Archive, Builder};
use uuid::Uuid;

pub fn is_tar_gz_path(path: &Path) -> bool {
    let value = path.to_string_lossy().trim().trim_end_matches(['/', '\\']).to_ascii_lowercase();
    value.ends_with(".tar.gz") || value.ends_with(".tgz")
}

pub fn create_archive_staging_dir(
    archive_path: &Path,
    purpose: &str,
) -> Result<PathBuf, KalamDbError> {
    let parent = archive_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        KalamDbError::InvalidOperation(format!(
            "Failed to create archive staging parent '{}': {}",
            parent.display(),
            error
        ))
    })?;

    let file_name = archive_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("kalamdb-transfer");
    let staging_dir = parent.join(format!(".{}.{}-{}", file_name, purpose, Uuid::new_v4()));

    fs::create_dir_all(&staging_dir).map_err(|error| {
        KalamDbError::InvalidOperation(format!(
            "Failed to create archive staging directory '{}': {}",
            staging_dir.display(),
            error
        ))
    })?;

    Ok(staging_dir)
}

pub fn create_tar_gz_archive(src_dir: &Path, archive_path: &Path) -> Result<(), KalamDbError> {
    if let Some(parent) = archive_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "Failed to create archive output directory '{}': {}",
                parent.display(),
                error
            ))
        })?;
    }

    if archive_path.exists() {
        if archive_path.is_dir() {
            return Err(KalamDbError::InvalidOperation(format!(
                "Archive output path '{}' is a directory",
                archive_path.display()
            )));
        }

        fs::remove_file(archive_path).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "Failed to replace existing archive '{}': {}",
                archive_path.display(),
                error
            ))
        })?;
    }

    let archive_file = File::create(archive_path).map_err(|error| {
        KalamDbError::InvalidOperation(format!(
            "Failed to create archive '{}': {}",
            archive_path.display(),
            error
        ))
    })?;
    let encoder = GzEncoder::new(archive_file, Compression::default());
    let mut builder = Builder::new(encoder);

    for entry_result in fs::read_dir(src_dir).map_err(|error| {
        KalamDbError::InvalidOperation(format!(
            "Failed to read archive staging directory '{}': {}",
            src_dir.display(),
            error
        ))
    })? {
        let entry = entry_result.map_err(|error| {
            KalamDbError::InvalidOperation(format!("Directory entry error: {}", error))
        })?;
        let src_path = entry.path();
        let name = entry.file_name();
        let archive_name = Path::new(name.as_os_str());

        if src_path.is_dir() {
            builder.append_dir_all(archive_name, &src_path).map_err(|error| {
                KalamDbError::InvalidOperation(format!(
                    "Failed to archive directory '{}': {}",
                    src_path.display(),
                    error
                ))
            })?;
        } else {
            let mut file = File::open(&src_path).map_err(|error| {
                KalamDbError::InvalidOperation(format!(
                    "Failed to open archive input '{}': {}",
                    src_path.display(),
                    error
                ))
            })?;
            builder.append_file(archive_name, &mut file).map_err(|error| {
                KalamDbError::InvalidOperation(format!(
                    "Failed to archive file '{}': {}",
                    src_path.display(),
                    error
                ))
            })?;
        }
    }

    builder.finish().map_err(|error| {
        KalamDbError::InvalidOperation(format!(
            "Failed to finish archive '{}': {}",
            archive_path.display(),
            error
        ))
    })?;
    let encoder = builder.into_inner().map_err(|error| {
        KalamDbError::InvalidOperation(format!(
            "Failed to finalize archive '{}': {}",
            archive_path.display(),
            error
        ))
    })?;
    encoder.finish().map_err(|error| {
        KalamDbError::InvalidOperation(format!(
            "Failed to flush archive '{}': {}",
            archive_path.display(),
            error
        ))
    })?;

    Ok(())
}

pub fn extract_tar_gz_archive(archive_path: &Path, output_dir: &Path) -> Result<(), KalamDbError> {
    fs::create_dir_all(output_dir).map_err(|error| {
        KalamDbError::InvalidOperation(format!(
            "Failed to create archive extraction directory '{}': {}",
            output_dir.display(),
            error
        ))
    })?;

    let archive_file = File::open(archive_path).map_err(|error| {
        KalamDbError::InvalidOperation(format!(
            "Failed to open archive '{}': {}",
            archive_path.display(),
            error
        ))
    })?;
    let decoder = GzDecoder::new(archive_file);
    let mut archive = Archive::new(decoder);

    for entry_result in archive.entries().map_err(|error| {
        KalamDbError::InvalidOperation(format!(
            "Failed to enumerate archive '{}': {}",
            archive_path.display(),
            error
        ))
    })? {
        let mut entry = entry_result.map_err(|error| {
            KalamDbError::InvalidOperation(format!("Archive entry error: {}", error))
        })?;

        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            return Err(KalamDbError::InvalidOperation(
                "Archive cannot contain symlinks".to_string(),
            ));
        }

        let relative_path = entry
            .path()
            .map_err(|error| {
                KalamDbError::InvalidOperation(format!("Invalid archive path: {}", error))
            })?
            .into_owned();

        if relative_path.components().any(|component| {
            matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_))
        }) {
            return Err(KalamDbError::InvalidOperation(
                "Archive contains an unsafe path".to_string(),
            ));
        }

        let output_path = output_dir.join(&relative_path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                KalamDbError::InvalidOperation(format!(
                    "Failed to create extraction path '{}': {}",
                    parent.display(),
                    error
                ))
            })?;
        }

        entry.unpack(&output_path).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "Failed to extract '{}' from archive: {}",
                relative_path.display(),
                error
            ))
        })?;
    }

    Ok(())
}

pub fn copy_dir_to_dir(src: &Path, dst: &Path) -> Result<u64, KalamDbError> {
    if !src.exists() {
        return Ok(0);
    }

    fs::create_dir_all(dst).map_err(|error| {
        KalamDbError::InvalidOperation(format!(
            "Failed to create directory '{}': {}",
            dst.display(),
            error
        ))
    })?;

    let mut bytes_copied = 0u64;
    for entry_result in fs::read_dir(src).map_err(|error| {
        KalamDbError::InvalidOperation(format!(
            "Failed to read directory '{}': {}",
            src.display(),
            error
        ))
    })? {
        let entry = entry_result.map_err(|error| {
            KalamDbError::InvalidOperation(format!("Directory entry error: {}", error))
        })?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            bytes_copied += copy_dir_to_dir(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path).map_err(|error| {
                KalamDbError::InvalidOperation(format!(
                    "Failed to copy '{}' to '{}': {}",
                    src_path.display(),
                    dst_path.display(),
                    error
                ))
            })?;
            bytes_copied += src_path.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        }
    }

    Ok(bytes_copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn tar_gz_path_accepts_trailing_separator() {
        assert!(is_tar_gz_path(Path::new("/tmp/kalamdb.tar.gz/")));
        assert!(is_tar_gz_path(Path::new("/tmp/kalamdb.tgz/")));
    }

    #[test]
    fn extract_archive_rejects_parent_paths() {
        let temp_dir = tempdir().expect("temp dir");
        let archive_path = temp_dir.path().join("bad.tar.gz");
        let archive_file = File::create(&archive_path).expect("archive file");
        let encoder = GzEncoder::new(archive_file, Compression::default());
        let mut builder = Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_cksum();
        builder
            .append_data(&mut header, "../bad", &b"oops"[..])
            .expect("append bad path");
        builder.finish().expect("finish archive");
        let encoder = builder.into_inner().expect("encoder");
        encoder.finish().expect("finish gzip");

        let output_dir = temp_dir.path().join("out");
        assert!(extract_tar_gz_archive(&archive_path, &output_dir).is_err());
    }
}
