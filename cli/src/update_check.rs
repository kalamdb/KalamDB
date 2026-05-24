use std::time::Duration;

use serde::Deserialize;

use crate::{CLIError, Result, CLI_VERSION};

pub const GITHUB_REPO: &str = "kalamdb/KalamDB";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAvailability {
    pub current_version: String,
    pub latest_version: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
}

#[derive(Debug, Deserialize)]
struct GitHubTag {
    name: String,
}

pub async fn check_for_update(timeout: Duration) -> Result<Option<UpdateAvailability>> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(format!("kalam-cli/{}", CLI_VERSION))
        .build()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("Failed to create HTTP client: {}", error))
        })?;
    let latest_version = resolve_release_version(&client, false).await?;
    if version_is_newer(&latest_version, CLI_VERSION) {
        Ok(Some(UpdateAvailability {
            current_version: CLI_VERSION.to_string(),
            latest_version,
        }))
    } else {
        Ok(None)
    }
}

pub async fn resolve_release_version(
    client: &reqwest::Client,
    pre_release: bool,
) -> Result<String> {
    if pre_release {
        let url = format!("https://api.github.com/repos/{}/releases?per_page=20", GITHUB_REPO);
        let releases = client
            .get(url)
            .send()
            .await
            .map_err(|error| {
                CLIError::ConfigurationError(format!("Failed to reach GitHub: {}", error))
            })?
            .error_for_status()
            .map_err(|error| {
                CLIError::ConfigurationError(format!("GitHub release lookup failed: {}", error))
            })?
            .json::<Vec<GitHubRelease>>()
            .await
            .map_err(|error| {
                CLIError::ConfigurationError(format!("Failed to parse GitHub response: {}", error))
            })?;
        let release = releases
            .into_iter()
            .find(|release| release.prerelease)
            .ok_or_else(|| CLIError::ConfigurationError("No prerelease was found".to_string()))?;
        return Ok(normalize_version_tag(&release.tag_name));
    }

    let url = format!("https://api.github.com/repos/{}/releases/latest", GITHUB_REPO);
    let response = client.get(url).send().await.map_err(|error| {
        CLIError::ConfigurationError(format!("Failed to reach GitHub: {}", error))
    })?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return resolve_latest_tag(client).await;
    }

    let release = response
        .error_for_status()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("GitHub release lookup failed: {}", error))
        })?
        .json::<GitHubRelease>()
        .await
        .map_err(|error| {
            CLIError::ConfigurationError(format!("Failed to parse GitHub response: {}", error))
        })?;
    Ok(normalize_version_tag(&release.tag_name))
}

async fn resolve_latest_tag(client: &reqwest::Client) -> Result<String> {
    let url = format!("https://api.github.com/repos/{}/tags", GITHUB_REPO);
    let tags = client
        .get(url)
        .send()
        .await
        .map_err(|error| {
            CLIError::ConfigurationError(format!("Failed to reach GitHub: {}", error))
        })?
        .error_for_status()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("GitHub tag lookup failed: {}", error))
        })?
        .json::<Vec<GitHubTag>>()
        .await
        .map_err(|error| {
            CLIError::ConfigurationError(format!("Failed to parse GitHub tags: {}", error))
        })?;
    tags.into_iter()
        .next()
        .map(|tag| normalize_version_tag(&tag.name))
        .ok_or_else(|| CLIError::ConfigurationError("No GitHub tags were found".to_string()))
}

pub fn normalize_version_tag(version: &str) -> String {
    version.trim().trim_start_matches('v').to_string()
}

pub fn version_is_newer(candidate: &str, current: &str) -> bool {
    let candidate = ParsedVersion::parse(candidate);
    let current = ParsedVersion::parse(current);
    candidate > current
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedVersion {
    core: Vec<u64>,
    prerelease: Option<Vec<PrereleasePart>>,
}

impl ParsedVersion {
    fn parse(version: &str) -> Self {
        let normalized = normalize_version_tag(version);
        let (core, prerelease) = normalized
            .split_once('-')
            .map_or((normalized.as_str(), None), |(core, prerelease)| (core, Some(prerelease)));
        let core = core.split('.').map(|part| part.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
        let prerelease = prerelease.map(|pre| {
            pre.split('.')
                .map(|part| {
                    part.parse::<u64>()
                        .map(PrereleasePart::Number)
                        .unwrap_or_else(|_| PrereleasePart::Text(part.to_ascii_lowercase()))
                })
                .collect()
        });
        Self { core, prerelease }
    }
}

impl Ord for ParsedVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let max_len = self.core.len().max(other.core.len());
        for index in 0..max_len {
            let left = self.core.get(index).copied().unwrap_or(0);
            let right = other.core.get(index).copied().unwrap_or(0);
            match left.cmp(&right) {
                std::cmp::Ordering::Equal => {},
                ordering => return ordering,
            }
        }

        match (&self.prerelease, &other.prerelease) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(_), None) => std::cmp::Ordering::Less,
            (Some(left), Some(right)) => left.cmp(right),
        }
    }
}

impl PartialOrd for ParsedVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrereleasePart {
    Number(u64),
    Text(String),
}

impl Ord for PrereleasePart {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Number(left), Self::Number(right)) => left.cmp(right),
            (Self::Text(left), Self::Text(right)) => left.cmp(right),
            (Self::Number(_), Self::Text(_)) => std::cmp::Ordering::Less,
            (Self::Text(_), Self::Number(_)) => std::cmp::Ordering::Greater,
        }
    }
}

impl PartialOrd for PrereleasePart {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison_handles_core_versions() {
        assert!(version_is_newer("0.5.2", "0.5.1"));
        assert!(version_is_newer("1.0.0", "0.9.9"));
        assert!(!version_is_newer("0.5.1", "0.5.1"));
        assert!(!version_is_newer("0.5.0", "0.5.1"));
    }

    #[test]
    fn version_comparison_handles_prereleases() {
        assert!(version_is_newer("0.5.1-beta.3", "0.5.1-beta.2"));
        assert!(version_is_newer("0.5.1", "0.5.1-beta.2"));
        assert!(!version_is_newer("0.5.1-beta.1", "0.5.1-beta.2"));
        assert!(!version_is_newer("0.5.0", "0.5.1-beta.2"));
    }

    #[test]
    fn normalize_version_tag_strips_leading_v() {
        assert_eq!(normalize_version_tag("v0.5.1-beta.2"), "0.5.1-beta.2");
    }
}
