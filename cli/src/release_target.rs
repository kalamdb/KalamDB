use std::path::{Path, PathBuf};

use crate::{
    release_download::{
        archive_extension, archive_kind_for_platform, find_first_file_matching, release_base_url,
        ArchiveKind,
    },
    release_version::ReleaseVersion,
    CLIError, Result,
};

pub const CLI_ARTIFACT_PREFIX: &str = "kalamcli";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseTarget {
    artifact_prefix: String,
    version: ReleaseVersion,
    platform: String,
    archive_kind: ArchiveKind,
}

impl ReleaseTarget {
    pub fn new(
        artifact_prefix: impl Into<String>,
        version: ReleaseVersion,
        platform: impl AsRef<str>,
    ) -> Result<Self> {
        let artifact_prefix = artifact_prefix.into();
        validate_artifact_prefix(&artifact_prefix)?;
        let platform = platform.as_ref().to_string();
        validate_platform(&platform)?;
        let archive_kind = archive_kind_for_platform(&platform);

        Ok(Self {
            artifact_prefix,
            version,
            platform,
            archive_kind,
        })
    }

    pub fn version(&self) -> &ReleaseVersion {
        &self.version
    }

    pub fn platform(&self) -> &str {
        &self.platform
    }

    pub fn archive_kind(&self) -> ArchiveKind {
        self.archive_kind
    }

    pub fn archive_name(&self) -> String {
        format!(
            "{}-{}-{}.{}",
            self.artifact_prefix,
            self.version,
            self.platform,
            archive_extension(self.archive_kind)
        )
    }

    pub fn binary_name(&self) -> String {
        if self.platform.starts_with("windows-") {
            format!("{}-{}-{}.exe", self.artifact_prefix, self.version, self.platform)
        } else {
            format!("{}-{}-{}", self.artifact_prefix, self.version, self.platform)
        }
    }

    pub fn archive_url(&self, override_env_var: &str) -> Result<String> {
        let base_url = self.base_url(override_env_var)?;
        Ok(format!("{}/{}", base_url, self.archive_name()))
    }

    pub fn checksums_url(&self, override_env_var: &str) -> Result<String> {
        let base_url = self.base_url(override_env_var)?;
        Ok(format!("{base_url}/SHA256SUMS"))
    }

    pub fn manifest_url(&self, override_env_var: &str) -> Result<String> {
        let base_url = self.base_url(override_env_var)?;
        Ok(format!("{base_url}/versions.json"))
    }

    pub fn find_binary(&self, extracted_root: &Path) -> Option<PathBuf> {
        let expected_binary_name = self.binary_name();
        find_first_file_matching(extracted_root, |path| {
            path.file_name().and_then(|name| name.to_str()) == Some(expected_binary_name.as_str())
        })
    }

    fn base_url(&self, override_env_var: &str) -> Result<String> {
        release_base_url(self.version.as_str(), override_env_var)
    }
}

fn validate_artifact_prefix(prefix: &str) -> Result<()> {
    if prefix.is_empty() || !prefix.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(CLIError::ConfigurationError(format!(
            "invalid release artifact prefix '{prefix}'"
        )));
    }

    Ok(())
}

fn validate_platform(platform: &str) -> Result<()> {
    if platform.is_empty()
        || !platform.contains('-')
        || !platform.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
    {
        return Err(CLIError::ConfigurationError(format!("invalid release platform '{platform}'")));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release_version::ReleaseVersion;

    #[test]
    fn release_target_uses_exact_archive_and_binary_names() {
        let version = ReleaseVersion::parse("0.5.2-rc.2").unwrap();
        let target = ReleaseTarget::new(CLI_ARTIFACT_PREFIX, version, "macos-aarch64").unwrap();

        assert_eq!(target.archive_name(), "kalamcli-0.5.2-rc.2-macos-aarch64.tar.gz");
        assert_eq!(target.binary_name(), "kalamcli-0.5.2-rc.2-macos-aarch64");
    }

    #[test]
    fn windows_release_target_uses_zip_archive_and_exe_binary() {
        let version = ReleaseVersion::parse("0.5.2").unwrap();
        let target = ReleaseTarget::new(CLI_ARTIFACT_PREFIX, version, "windows-x86_64").unwrap();

        assert_eq!(target.archive_name(), "kalamcli-0.5.2-windows-x86_64.zip");
        assert_eq!(target.binary_name(), "kalamcli-0.5.2-windows-x86_64.exe");
    }
}
