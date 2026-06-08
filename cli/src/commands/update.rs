use std::{
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use kalam_cli::{
    release_download::{
        archive_kind_for_platform, archive_name, copy_file_with_executable_bit, create_temp_dir,
        detect_platform, download_bytes, download_text, extract_archive, find_first_file_matching,
        release_base_url, verify_checksum,
    },
    update_check, CLIError, Result,
};

use crate::args::{Cli, UpdateArgs};

const ARTIFACT_PREFIX: &str = "kalamcli";
const BINARY_NAME: &str = "kalam";
const RELEASE_BASE_URL_ENV: &str = "KALAM_CLI_RELEASE_BASE_URL";

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
    let archive_name = archive_name(ARTIFACT_PREFIX, &version, &platform, archive_kind);
    let base_url = release_base_url(&version, RELEASE_BASE_URL_ENV);
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

    let checksums = download_text(&client, &checksums_url, "checksum file").await?;
    verify_checksum(&archive_name, &archive_bytes, &checksums)?;
    eprintln!("Checksum verified");

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

    println!("Installed kalam {} to {}", version, current_exe.display());
    Ok(true)
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
}
