use std::{
    env,
    fs::{self, File},
    io::{self, Cursor},
    path::{Path, PathBuf},
    time::Duration,
};

use flate2::read::GzDecoder;
use indicatif::{ProgressBar, ProgressStyle};
use kalam_cli::{update_check, CLIError, Result};
use sha2::{Digest, Sha256};

use crate::args::{Cli, UpdateArgs};

const ARTIFACT_PREFIX: &str = "kalamcli";
const BINARY_NAME: &str = "kalam";
const RELEASE_BASE_URL_ENV: &str = "KALAM_CLI_RELEASE_BASE_URL";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveKind {
    TarGz,
    Zip,
}

pub async fn handle_update(cli: &Cli, args: &UpdateArgs) -> Result<bool> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(cli.timeout.max(30)))
        .user_agent(format!("kalam-cli/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("Failed to create HTTP client: {}", error))
        })?;

    let version = resolve_version(&client, args).await?;
    let platform = detect_platform()?;
    let archive_kind = archive_kind_for_platform(&platform);
    let archive_name = format!(
        "{}-{}-{}.{}",
        ARTIFACT_PREFIX,
        version,
        platform,
        archive_extension(archive_kind)
    );
    let base_url = release_base_url(&version);
    let archive_url = format!("{}/{}", base_url, archive_name);
    let checksums_url = format!("{}/SHA256SUMS", base_url);
    let current_exe = env::current_exe().map_err(|error| {
        CLIError::ConfigurationError(format!("Failed to locate current executable: {}", error))
    })?;

    if args.dry_run {
        println!("Current version: {}", env!("CARGO_PKG_VERSION"));
        println!("Target version: {}", version);
        println!("Platform: {}", platform);
        println!("Archive: {}", archive_name);
        println!("Download URL: {}", archive_url);
        println!("Install path: {}", current_exe.display());
        return Ok(true);
    }

    if version == env!("CARGO_PKG_VERSION") && !args.force {
        println!("kalam is already at version {}", version);
        return Ok(true);
    }

    println!("Updating kalam from {} to {}", env!("CARGO_PKG_VERSION"), version);
    let archive_bytes =
        download_bytes(&client, &archive_url, &archive_name, !cli.no_spinner).await?;

    let checksums = download_text(&client, &checksums_url).await?;
    verify_checksum(&archive_name, &archive_bytes, &checksums)?;
    eprintln!("Checksum verified");

    eprintln!("Extracting binary");
    let temp_dir = create_temp_dir()?;
    let cleanup_dir = temp_dir.clone();
    let install_result = async {
        extract_archive(&archive_bytes, archive_kind, &temp_dir)?;
        let binary_path = find_binary(&temp_dir).ok_or_else(|| {
            CLIError::ConfigurationError(format!(
                "Could not find '{}' binary in archive",
                BINARY_NAME
            ))
        })?;
        replace_current_binary(&current_exe, &binary_path)?;
        Ok::<(), CLIError>(())
    }
    .await;
    let _ = fs::remove_dir_all(cleanup_dir);
    install_result?;

    println!("Installed kalam {} to {}", version, current_exe.display());
    Ok(true)
}

fn release_base_url(version: &str) -> String {
    if let Some(override_url) = env::var_os(RELEASE_BASE_URL_ENV) {
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

async fn resolve_version(client: &reqwest::Client, args: &UpdateArgs) -> Result<String> {
    if let Some(version) = &args.version {
        return Ok(update_check::normalize_version_tag(version));
    }

    update_check::resolve_release_version(client, args.pre_release).await
}

async fn download_bytes(
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

fn create_download_progress_bar(archive_name: &str, total_bytes: Option<u64>) -> ProgressBar {
    let progress_bar = total_bytes.map_or_else(ProgressBar::new_spinner, ProgressBar::new);
    if total_bytes.is_some() {
        progress_bar.set_style(
            ProgressStyle::with_template(
                "{spinner:.cyan} {msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
            )
            .expect("download progress template should be valid")
            .progress_chars("=> "),
        );
    } else {
        progress_bar.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg} {bytes}")
                .expect("download spinner template should be valid"),
        );
    }
    progress_bar.set_message(format!("Downloading {}", archive_name));
    progress_bar.enable_steady_tick(Duration::from_millis(80));
    progress_bar
}

async fn download_text(client: &reqwest::Client, url: &str) -> Result<String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| {
            CLIError::ConfigurationError(format!("Checksum download failed: {}", error))
        })?
        .error_for_status()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("Checksum download failed: {}", error))
        })?;
    response.text().await.map_err(|error| {
        CLIError::ConfigurationError(format!("Failed to read checksum file: {}", error))
    })
}

fn verify_checksum(archive_name: &str, archive_bytes: &[u8], checksums: &str) -> Result<()> {
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

fn detect_platform() -> Result<String> {
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

fn archive_kind_for_platform(platform: &str) -> ArchiveKind {
    if platform.starts_with("windows-") {
        ArchiveKind::Zip
    } else {
        ArchiveKind::TarGz
    }
}

fn archive_extension(kind: ArchiveKind) -> &'static str {
    match kind {
        ArchiveKind::TarGz => "tar.gz",
        ArchiveKind::Zip => "zip",
    }
}

fn create_temp_dir() -> Result<PathBuf> {
    let base = env::temp_dir().join(format!(
        "kalam-update-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&base).map_err(|error| {
        CLIError::FileError(format!("Failed to create temporary directory: {}", error))
    })?;
    Ok(base)
}

fn extract_archive(archive_bytes: &[u8], kind: ArchiveKind, destination: &Path) -> Result<()> {
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

fn find_binary(root: &Path) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let entries = fs::read_dir(&path).ok()?;
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                stack.push(entry_path);
            } else if is_candidate_binary(&entry_path) {
                return Some(entry_path);
            }
        }
    }
    None
}

fn is_candidate_binary(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    file_name == BINARY_NAME
        || file_name == ARTIFACT_PREFIX
        || file_name == "kalam.exe"
        || file_name.starts_with("kalamcli-")
}

fn replace_current_binary(current_exe: &Path, new_binary: &Path) -> Result<()> {
    let temp_path = current_exe.with_extension("kalam-update-tmp");
    fs::copy(new_binary, &temp_path).map_err(|error| {
        CLIError::FileError(format!("Failed to stage updated binary: {}", error))
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o755)).map_err(|error| {
            CLIError::FileError(format!("Failed to mark staged binary executable: {}", error))
        })?;
    }

    let replace_result = fs::rename(&temp_path, current_exe);
    if let Err(error) = replace_result {
        let _ = fs::remove_file(&temp_path);
        return Err(CLIError::FileError(format!(
            "Failed to replace {}: {}",
            current_exe.display(),
            error
        )));
    }

    Ok(())
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
    fn candidate_binary_accepts_release_archive_names() {
        assert!(is_candidate_binary(Path::new("kalamcli-0.5.1-beta.2-macos-aarch64")));
        assert!(is_candidate_binary(Path::new("kalamcli-0.5.1-beta.2-windows-x86_64.exe")));
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
