use std::{env, fs, path::Path, time::Duration};

use kalam_cli::{
    release_download::{
        copy_file_with_executable_bit, create_temp_dir, detect_platform, download_bytes,
        download_text, extract_archive, verify_checksum,
    },
    release_target::{ReleaseTarget, CLI_ARTIFACT_PREFIX},
    release_version::ReleaseVersion,
    update_check, CLIError, Result, CLI_BUILD_DATE, CLI_VERSION,
};

use crate::args::{Cli, UpdateArgs};

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
    let target = ReleaseTarget::new(CLI_ARTIFACT_PREFIX, version, &platform)?;
    let archive_name = target.archive_name();
    let archive_url = target.archive_url(RELEASE_BASE_URL_ENV)?;
    let checksums_url = target.checksums_url(RELEASE_BASE_URL_ENV)?;
    let current_exe = env::current_exe().map_err(|error| {
        CLIError::ConfigurationError(format!("Failed to locate current executable: {}", error))
    })?;

    if args.dry_run {
        println!("Current version: {}", CLI_VERSION);
        println!("Current build date: {}", CLI_BUILD_DATE);
        println!("Target version: {}", target.version());
        println!("Platform: {}", target.platform());
        println!("Archive: {}", archive_name);
        println!("Download URL: {}", archive_url);
        println!("Install path: {}", current_exe.display());
        return Ok(true);
    }

    let (archive_bytes, checksums, remote_build_date) =
        if target.version().as_str() == CLI_VERSION && !args.force {
            match resolve_same_version_update(
                &client,
                target.version().as_str(),
                &archive_url,
                &checksums_url,
                &archive_name,
                !cli.no_spinner,
            )
            .await?
            {
                SameVersionUpdate::UpToDate => {
                    println!(
                        "kalam is already at version {} (built {})",
                        target.version(),
                        CLI_BUILD_DATE
                    );
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

    if target.version().as_str() == CLI_VERSION {
        if let Some(remote_build_date) = remote_build_date.as_deref() {
            println!(
                "Updating kalam {} from build {} to build {}",
                target.version(),
                CLI_BUILD_DATE,
                remote_build_date
            );
        } else {
            println!("Updating kalam {} to a newer build", target.version());
        }
    } else {
        println!("Updating kalam from {} to {}", CLI_VERSION, target.version());
    }

    eprintln!("Extracting binary");
    let temp_dir = create_temp_dir("kalam-update")?;
    let cleanup_dir = temp_dir.clone();
    let install_result = async {
        extract_archive(&archive_bytes, target.archive_kind(), &temp_dir)?;
        let binary_path = target.find_binary(&temp_dir).ok_or_else(|| {
            CLIError::ConfigurationError(format!(
                "Could not find '{}' binary in archive",
                target.binary_name()
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
            target.version(),
            remote_build_date,
            current_exe.display()
        );
    } else {
        println!("Installed kalam {} to {}", target.version(), current_exe.display());
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
    version: &str,
    archive_url: &str,
    checksums_url: &str,
    archive_name: &str,
    show_progress: bool,
) -> Result<SameVersionUpdate> {
    let remote_build_date =
        update_check::fetch_release_cli_build_date(client, version, RELEASE_BASE_URL_ENV).await?;

    if let Some(remote_build_date) = remote_build_date.as_deref() {
        if !update_check::build_timestamp_is_newer(remote_build_date, CLI_BUILD_DATE) {
            return Ok(SameVersionUpdate::UpToDate);
        }
    } else {
        return Ok(SameVersionUpdate::UpToDate);
    }

    let checksums = download_text(client, checksums_url, "checksum file").await?;
    let archive_bytes = download_bytes(client, archive_url, archive_name, show_progress).await?;
    verify_checksum(archive_name, &archive_bytes, &checksums)?;

    Ok(SameVersionUpdate::NeedsInstall {
        archive_bytes,
        checksums,
        remote_build_date,
    })
}

async fn resolve_version(client: &reqwest::Client, args: &UpdateArgs) -> Result<ReleaseVersion> {
    if let Some(version) = &args.version {
        return ReleaseVersion::parse(version);
    }

    let version = update_check::resolve_release_version(client, args.pre_release).await?;
    ReleaseVersion::parse(&version)
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
    fn parse_built_line_from_version_output() {
        let output = "kalam 0.5.2-rc.2\nCommit: abc (main)\nBuilt: 2026-06-12 10:00:00 UTC\n";
        assert_eq!(
            update_check::parse_built_line(output),
            Some("2026-06-12 10:00:00 UTC".to_string())
        );
    }
}
