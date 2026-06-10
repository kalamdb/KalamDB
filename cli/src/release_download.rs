use std::{
    env,
    fs::{self, File},
    io::{self, Cursor},
    path::{Path, PathBuf},
    time::Duration,
};

use flate2::read::GzDecoder;
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};

use crate::{terminal_ui, update_check, CLIError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    TarGz,
    Zip,
}

pub fn release_base_url(version: &str, override_env_var: &str) -> String {
    if let Some(override_url) = env::var_os(override_env_var) {
        let override_url = override_url.to_string_lossy().trim().trim_end_matches('/').to_string();
        if !override_url.is_empty() {
            return override_url;
        }
    }

    format!(
        "https://github.com/{}/releases/download/v{}",
        update_check::GITHUB_REPO,
        version
    )
}

pub fn detect_platform() -> Result<String> {
    let os = match env::consts::OS {
        "linux" => "linux",
        "macos" => "macos",
        "windows" => "windows",
        other => {
            return Err(CLIError::ConfigurationError(format!(
                "Unsupported operating system: {}",
                other
            )))
        },
    };
    let arch = match env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => {
            return Err(CLIError::ConfigurationError(format!(
                "Unsupported architecture: {}",
                other
            )))
        },
    };
    Ok(format!("{}-{}", os, arch))
}

pub fn archive_kind_for_platform(platform: &str) -> ArchiveKind {
    if platform.starts_with("windows-") {
        ArchiveKind::Zip
    } else {
        ArchiveKind::TarGz
    }
}

pub fn archive_extension(kind: ArchiveKind) -> &'static str {
    match kind {
        ArchiveKind::TarGz => "tar.gz",
        ArchiveKind::Zip => "zip",
    }
}

pub fn archive_name(prefix: &str, version: &str, platform: &str, kind: ArchiveKind) -> String {
    format!("{}-{}-{}.{}", prefix, version, platform, archive_extension(kind))
}

pub async fn download_bytes(
    client: &reqwest::Client,
    url: &str,
    archive_name: &str,
    show_progress: bool,
) -> Result<Vec<u8>> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|error| CLIError::ConfigurationError(format!("Download failed: {}", error)))?
        .error_for_status()
        .map_err(|error| CLIError::ConfigurationError(format!("Download failed: {}", error)))?;
    let total_bytes = response.content_length();
    let progress_bar = if show_progress {
        Some(create_download_progress_bar(archive_name, total_bytes))
    } else {
        None
    };

    let mut downloaded =
        Vec::with_capacity(total_bytes.unwrap_or_default().min(usize::MAX as u64) as usize);
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        CLIError::ConfigurationError(format!("Failed to read download: {}", error))
    })? {
        if let Some(progress_bar) = &progress_bar {
            progress_bar.inc(chunk.len() as u64);
        }
        downloaded.extend_from_slice(&chunk);
    }

    if let Some(progress_bar) = progress_bar {
        progress_bar.finish_with_message(format!("Downloaded {}", archive_name));
    }

    Ok(downloaded)
}

pub async fn download_text(client: &reqwest::Client, url: &str, label: &str) -> Result<String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| CLIError::ConfigurationError(format!("{label} download failed: {error}")))?
        .error_for_status()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("{label} download failed: {error}"))
        })?;
    response
        .text()
        .await
        .map_err(|error| CLIError::ConfigurationError(format!("Failed to read {label}: {error}")))
}

pub fn verify_checksum(archive_name: &str, archive_bytes: &[u8], checksums: &str) -> Result<()> {
    let expected = checksums
        .lines()
        .find_map(|line| parse_checksum_line(line, archive_name))
        .ok_or_else(|| {
            CLIError::ConfigurationError(format!(
                "SHA256SUMS does not include an entry for {}",
                archive_name
            ))
        })?;

    let mut hasher = Sha256::new();
    hasher.update(archive_bytes);
    let actual = hex::encode(hasher.finalize());

    if actual != expected {
        return Err(CLIError::ConfigurationError(format!(
            "Checksum mismatch for {} (expected {}, got {})",
            archive_name, expected, actual
        )));
    }

    Ok(())
}

pub fn create_temp_dir(prefix: &str) -> Result<PathBuf> {
    let base = env::temp_dir().join(format!(
        "{}-{}-{}",
        prefix,
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&base).map_err(|error| {
        CLIError::FileError(format!("Failed to create temporary directory: {}", error))
    })?;
    Ok(base)
}

pub fn extract_archive(archive_bytes: &[u8], kind: ArchiveKind, destination: &Path) -> Result<()> {
    match kind {
        ArchiveKind::TarGz => {
            let decoder = GzDecoder::new(Cursor::new(archive_bytes));
            let mut archive = tar::Archive::new(decoder);
            let entries = archive.entries().map_err(|error| {
                CLIError::FileError(format!("Failed to read tar archive: {}", error))
            })?;
            for entry in entries {
                let mut entry = entry.map_err(|error| {
                    CLIError::FileError(format!("Failed to read tar entry: {}", error))
                })?;
                entry.unpack_in(destination).map_err(|error| {
                    CLIError::FileError(format!("Failed to extract tar entry: {}", error))
                })?;
            }
            Ok(())
        },
        ArchiveKind::Zip => extract_zip_archive(archive_bytes, destination),
    }
}

pub fn find_first_file_matching(root: &Path, predicate: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let entries = fs::read_dir(&path).ok()?;
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                stack.push(entry_path);
            } else if predicate(&entry_path) {
                return Some(entry_path);
            }
        }
    }
    None
}

pub fn copy_file_with_executable_bit(source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            CLIError::FileError(format!("Failed to create install directory: {}", error))
        })?;
    }

    fs::copy(source, destination)
        .map_err(|error| CLIError::FileError(format!("Failed to copy file: {}", error)))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(destination, fs::Permissions::from_mode(0o755)).map_err(|error| {
            CLIError::FileError(format!("Failed to mark file executable: {}", error))
        })?;
    }

    Ok(())
}

fn create_download_progress_bar(archive_name: &str, total_bytes: Option<u64>) -> ProgressBar {
    let message = format!("Downloading {}", archive_name);
    let progress_bar = if let Some(total_bytes) = total_bytes {
        ProgressBar::new(total_bytes)
    } else {
        terminal_ui::create_spinner(&message)
    };
    if total_bytes.is_some() {
        progress_bar.set_style(
            ProgressStyle::with_template(
                "{spinner:.cyan} {msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
            )
            .expect("download progress template should be valid")
            .progress_chars("=> "),
        );
    }
    progress_bar.set_message(message);
    progress_bar.enable_steady_tick(Duration::from_millis(80));
    progress_bar
}

fn extract_zip_archive(archive_bytes: &[u8], destination: &Path) -> Result<()> {
    let reader = Cursor::new(archive_bytes);
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|error| CLIError::FileError(format!("Failed to read zip archive: {}", error)))?;

    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|error| CLIError::FileError(format!("Failed to read zip entry: {}", error)))?;
        let Some(enclosed_name) = file.enclosed_name() else {
            continue;
        };
        let output_path = destination.join(enclosed_name);
        if file.is_dir() {
            fs::create_dir_all(&output_path).map_err(|error| {
                CLIError::FileError(format!("Failed to create directory from zip: {}", error))
            })?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                CLIError::FileError(format!("Failed to create directory from zip: {}", error))
            })?;
        }
        let mut output_file = File::create(&output_path).map_err(|error| {
            CLIError::FileError(format!("Failed to create extracted file: {}", error))
        })?;
        io::copy(&mut file, &mut output_file).map_err(|error| {
            CLIError::FileError(format!("Failed to extract zip entry: {}", error))
        })?;
    }

    Ok(())
}

fn parse_checksum_line<'a>(line: &'a str, archive_name: &str) -> Option<&'a str> {
    let mut parts = line.split_whitespace();
    let hash = parts.next()?;
    let name = parts.next()?.trim_start_matches('*');
    if name == archive_name {
        Some(hash)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_line_parser_accepts_standard_and_binary_lines() {
        assert_eq!(
            parse_checksum_line(
                "abc123  kalamcli-1.0.0-macos-aarch64.tar.gz",
                "kalamcli-1.0.0-macos-aarch64.tar.gz"
            ),
            Some("abc123")
        );
        assert_eq!(
            parse_checksum_line(
                "abc123 *kalamcli-1.0.0-windows-x86_64.zip",
                "kalamcli-1.0.0-windows-x86_64.zip"
            ),
            Some("abc123")
        );
    }

    #[test]
    fn checksum_line_parser_ignores_other_archives() {
        assert_eq!(
            parse_checksum_line(
                "abc123  kalamcli-1.0.0-linux-x86_64.tar.gz",
                "kalamcli-1.0.0-macos-aarch64.tar.gz"
            ),
            None
        );
    }

    #[test]
    fn archive_extension_matches_platform() {
        assert_eq!(archive_extension(archive_kind_for_platform("macos-aarch64")), "tar.gz");
        assert_eq!(archive_extension(archive_kind_for_platform("windows-x86_64")), "zip");
    }

    #[test]
    fn download_progress_bar_tracks_known_content_length() {
        let progress_bar = create_download_progress_bar("kalamcli-test.tar.gz", Some(128));
        assert_eq!(progress_bar.length(), Some(128));
        progress_bar.inc(32);
        assert_eq!(progress_bar.position(), 32);
        progress_bar.finish_and_clear();
    }
}
