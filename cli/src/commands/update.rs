use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use kalam_cli::{
    release_download::{
        archive_kind_for_platform, archive_name, copy_file_with_executable_bit, create_temp_dir,
        detect_platform, download_bytes, download_text, extract_archive, find_first_file_matching,
        release_base_url, verify_checksum,
    },
    update_check, CLIError, Result, CLI_BUILD_DATE, CLI_VERSION,
};

use crate::args::{Cli, UpdateArgs};

const ARTIFACT_PREFIX: &str = "kalamcli";
const BINARY_NAME: &str = "kalam";
const RELEASE_BASE_URL_ENV: &str = "KALAM_CLI_RELEASE_BASE_URL";

pub async fn handle_update(cli: &Cli, args: &UpdateArgs) -> Result<bool> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(cli.timeout.max(30)))
        .user_agent(format!("kalam-cli/{}", CLI_VERSION))
        .build()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("Failed to create HTTP client: {}", error))
        })?;

    let version = resolve_version(&client, args).await?;
    let platform = detect_platform()?;
    let archive_kind = archive_kind_for_platform(&platform);
    let archive_name = archive_name(ARTIFACT_PREFIX, &version, &platform, archive_kind);
    let base_url = release_base_url(&version, RELEASE_BASE_URL_ENV);
    let archive_url = format!("{}/{}", base_url, archive_name);
    let checksums_url = format!("{}/SHA256SUMS", base_url);
    let current_exe = env::current_exe().map_err(|error| {
        CLIError::ConfigurationError(format!("Failed to locate current executable: {}", error))
    })?;

    if args.dry_run {
        println!("Current version: {}", CLI_VERSION);
        println!("Current build date: {}", CLI_BUILD_DATE);
        println!("Target version: {}", version);
        println!("Platform: {}", platform);
        println!("Archive: {}", archive_name);
        println!("Download URL: {}", archive_url);
        println!("Install path: {}", current_exe.display());
        return Ok(true);
    }

    let (archive_bytes, checksums, remote_build_date) = if version == CLI_VERSION && !args.force {
        match resolve_same_version_update(
            &client,
            &current_exe,
            &archive_url,
            &checksums_url,
            &archive_name,
            archive_kind,
            !cli.no_spinner,
        )
        .await?
        {
            SameVersionUpdate::UpToDate => {
                println!("kalam is already at version {} (built {})", version, CLI_BUILD_DATE);
                return Ok(true);
            },
            SameVersionUpdate::NeedsInstall {
                archive_bytes,
                checksums,
                remote_build_date,
            } => (archive_bytes, checksums, remote_build_date),
        }
    } else {
        let archive_bytes =
            download_bytes(&client, &archive_url, &archive_name, !cli.no_spinner).await?;
        let checksums = download_text(&client, &checksums_url, "checksum file").await?;
        verify_checksum(&archive_name, &archive_bytes, &checksums)?;
        (archive_bytes, checksums, None)
    };

    verify_checksum(&archive_name, &archive_bytes, &checksums)?;
    eprintln!("Checksum verified");

    if version == CLI_VERSION {
        if let Some(remote_build_date) = remote_build_date.as_deref() {
            println!(
                "Updating kalam {} from build {} to build {}",
                version, CLI_BUILD_DATE, remote_build_date
            );
        } else {
            println!("Updating kalam {} to a newer build", version);
        }
    } else {
        println!("Updating kalam from {} to {}", CLI_VERSION, version);
    }

    eprintln!("Extracting binary");
    let temp_dir = create_temp_dir("kalam-update")?;
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

    if let Some(remote_build_date) = remote_build_date {
        println!(
            "Installed kalam {} (built {}) to {}",
            version,
            remote_build_date,
            current_exe.display()
        );
    } else {
        println!("Installed kalam {} to {}", version, current_exe.display());
    }
    Ok(true)
}

enum SameVersionUpdate {
    UpToDate,
    NeedsInstall {
        archive_bytes: Vec<u8>,
        checksums: String,
        remote_build_date: Option<String>,
    },
}

async fn resolve_same_version_update(
    client: &reqwest::Client,
    current_exe: &Path,
    archive_url: &str,
    checksums_url: &str,
    archive_name: &str,
    archive_kind: kalam_cli::release_download::ArchiveKind,
    show_progress: bool,
) -> Result<SameVersionUpdate> {
    let checksums = download_text(client, checksums_url, "checksum file").await?;
    if update_check::local_binary_matches_release_checksum(current_exe, &checksums, archive_name)?
    {
        return Ok(SameVersionUpdate::UpToDate);
    }

    let archive_bytes = download_bytes(client, archive_url, archive_name, show_progress).await?;
    verify_checksum(archive_name, &archive_bytes, &checksums)?;

    let temp_dir = create_temp_dir("kalam-update-check")?;
    let remote_build_date = read_remote_build_date(&archive_bytes, archive_kind, &temp_dir);
    let _ = fs::remove_dir_all(&temp_dir);

    if let Some(remote_build_date) = remote_build_date.as_deref() {
        if !update_check::build_timestamp_is_newer(remote_build_date, CLI_BUILD_DATE) {
            return Ok(SameVersionUpdate::UpToDate);
        }
    }

    Ok(SameVersionUpdate::NeedsInstall {
        archive_bytes,
        checksums,
        remote_build_date,
    })
}

fn read_remote_build_date(
    archive_bytes: &[u8],
    archive_kind: kalam_cli::release_download::ArchiveKind,
    temp_dir: &Path,
) -> Option<String> {
    extract_archive(archive_bytes, archive_kind, temp_dir).ok()?;
    let binary_path = find_binary(temp_dir)?;
    let output = Command::new(&binary_path).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    update_check::parse_built_line(&stdout)
}

async fn resolve_version(client: &reqwest::Client, args: &UpdateArgs) -> Result<String> {
    if let Some(version) = &args.version {
        return Ok(update_check::normalize_version_tag(version));
    }

    update_check::resolve_release_version(client, args.pre_release).await
}

fn find_binary(root: &Path) -> Option<PathBuf> {
    find_first_file_matching(root, is_candidate_binary)
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
    copy_file_with_executable_bit(new_binary, &temp_path)?;

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
    fn candidate_binary_accepts_release_archive_names() {
        assert!(is_candidate_binary(Path::new("kalamcli-0.5.1-beta.2-macos-aarch64")));
        assert!(is_candidate_binary(Path::new("kalamcli-0.5.1-beta.2-windows-x86_64.exe")));
    }

    #[test]
    fn parse_built_line_from_version_output() {
        let output = "kalam 0.5.2-rc.2\nCommit: abc (main)\nBuilt: 2026-06-12 10:00:00 UTC\n";
        assert_eq!(
            update_check::parse_built_line(output),
            Some("2026-06-12 10:00:00 UTC".to_string())
        );
    }
}
