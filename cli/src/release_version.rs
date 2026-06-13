use std::fmt;

use crate::{CLIError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReleaseVersion(String);

impl ReleaseVersion {
    pub fn parse(value: &str) -> Result<Self> {
        let normalized = normalize_release_version(value);
        validate_release_version(&normalized)?;
        Ok(Self(normalized))
    }

    pub fn normalize_lossy(value: &str) -> String {
        normalize_release_version(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ReleaseVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ReleaseVersion {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

fn normalize_release_version(value: &str) -> String {
    value.trim().strip_prefix('v').unwrap_or(value.trim()).to_string()
}

fn validate_release_version(version: &str) -> Result<()> {
    if version.is_empty() {
        return Err(invalid_release_version(version, "must not be empty"));
    }

    if version.len() > 128 {
        return Err(invalid_release_version(version, "is too long"));
    }

    if version.contains('+') {
        return Err(invalid_release_version(
            version,
            "build metadata is not supported in release artifact names",
        ));
    }

    let (core, prerelease) =
        version.split_once('-').map_or((version, None), |(core, pre)| (core, Some(pre)));

    validate_core_version(version, core)?;
    if let Some(prerelease) = prerelease {
        validate_prerelease(version, prerelease)?;
    }

    Ok(())
}

fn validate_core_version(original: &str, core: &str) -> Result<()> {
    let parts = core.split('.').collect::<Vec<_>>();
    if parts.len() != 3 {
        return Err(invalid_release_version(original, "must use major.minor.patch format"));
    }

    for part in parts {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid_release_version(original, "core version parts must be numeric"));
        }
        if part.len() > 1 && part.starts_with('0') {
            return Err(invalid_release_version(
                original,
                "core version parts must not contain leading zeroes",
            ));
        }
        part.parse::<u64>()
            .map_err(|_| invalid_release_version(original, "core version part is too large"))?;
    }

    Ok(())
}

fn validate_prerelease(original: &str, prerelease: &str) -> Result<()> {
    if prerelease.is_empty() {
        return Err(invalid_release_version(original, "prerelease identifier must not be empty"));
    }

    for part in prerelease.split('.') {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(invalid_release_version(
                original,
                "prerelease identifiers may contain only ASCII letters, digits, and hyphens",
            ));
        }
    }

    Ok(())
}

fn invalid_release_version(version: &str, reason: &str) -> CLIError {
    CLIError::ConfigurationError(format!("invalid release version '{version}': {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_version_accepts_published_tag_shapes() {
        assert_eq!(ReleaseVersion::parse("v0.5.2-rc.2").unwrap().as_str(), "0.5.2-rc.2");
        assert_eq!(ReleaseVersion::parse("0.3.0-alpha2").unwrap().as_str(), "0.3.0-alpha2");
    }

    #[test]
    fn release_version_rejects_path_and_url_control_characters() {
        for value in [
            "../0.5.2",
            "0.5.2/evil",
            "0.5.2 evil",
            "https://evil/0.5.2",
            "",
        ] {
            assert!(ReleaseVersion::parse(value).is_err(), "{value:?} should be rejected");
        }
    }
}
