use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;

use crate::{
    release_download, release_version::ReleaseVersion, CLIError, Result, CLI_BUILD_DATE,
    CLI_VERSION,
};

pub const GITHUB_REPO: &str = release_download::GITHUB_REPO;

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

#[derive(Debug, Deserialize)]
struct VersionsManifest {
    packages: VersionsPackages,
}

#[derive(Debug, Deserialize)]
struct VersionsPackages {
    core_components: CoreComponents,
}

#[derive(Debug, Deserialize)]
struct CoreComponents {
    cli: CliComponentManifest,
}

#[derive(Debug, Deserialize)]
struct CliComponentManifest {
    #[serde(default)]
    build_date: Option<String>,
}

pub fn parse_cli_build_date_from_manifest(manifest: &str) -> Option<String> {
    serde_json::from_str::<VersionsManifest>(manifest)
        .ok()
        .and_then(|manifest| manifest.packages.core_components.cli.build_date)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub async fn fetch_release_cli_build_date(
    client: &reqwest::Client,
    version: &str,
    release_base_url_env: &str,
) -> Result<Option<String>> {
    let base_url = release_download::release_base_url(version, release_base_url_env)?;
    let url = format!("{base_url}/versions.json");
    let response = client.get(url).send().await.map_err(|error| {
        CLIError::ConfigurationError(format!("Failed to reach release manifest: {error}"))
    })?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }

    let manifest = response
        .error_for_status()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("Release manifest lookup failed: {error}"))
        })?
        .text()
        .await
        .map_err(|error| {
            CLIError::ConfigurationError(format!("Failed to read release manifest: {error}"))
        })?;

    Ok(parse_cli_build_date_from_manifest(&manifest))
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
        return Ok(ReleaseVersion::parse(&release.tag_name)?.to_string());
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
    Ok(ReleaseVersion::parse(&release.tag_name)?.to_string())
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
        .map(|tag| ReleaseVersion::parse(&tag.name).map(|version| version.to_string()))
        .transpose()?
        .ok_or_else(|| CLIError::ConfigurationError("No GitHub tags were found".to_string()))
}

pub fn normalize_version_tag(version: &str) -> String {
    ReleaseVersion::normalize_lossy(version)
}

pub fn version_is_newer(candidate: &str, current: &str) -> bool {
    let candidate = ParsedVersion::parse(candidate);
    let current = ParsedVersion::parse(current);
    candidate > current
}

const BUILD_TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S UTC";

pub fn parse_build_timestamp(value: &str) -> Result<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(value.trim(), BUILD_TIMESTAMP_FORMAT)
        .map(|timestamp| timestamp.and_utc())
        .map_err(|error| {
            CLIError::ConfigurationError(format!("invalid build timestamp '{value}': {error}"))
        })
}

pub fn parse_built_line(version_output: &str) -> Option<String> {
    version_output.lines().find_map(|line| {
        line.strip_prefix("Built: ")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

pub fn build_timestamp_is_newer(candidate: &str, current: &str) -> bool {
    match (parse_build_timestamp(candidate), parse_build_timestamp(current)) {
        (Ok(candidate), Ok(current)) => candidate > current,
        _ => false,
    }
}

pub fn update_needed_for_release(latest_version: &str, remote_build_date: Option<&str>) -> bool {
    if version_is_newer(latest_version, CLI_VERSION) {
        return true;
    }

    if latest_version != CLI_VERSION {
        return false;
    }

    remote_build_date.is_some_and(|remote| build_timestamp_is_newer(remote, CLI_BUILD_DATE))
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

    fn adjacent_core_version(delta: i64) -> String {
        let core = CLI_VERSION
            .split_once('-')
            .map_or(CLI_VERSION, |(core, _)| core);
        let mut parts = core
            .split('.')
            .map(|part| part.parse::<i64>().unwrap_or(0))
            .collect::<Vec<_>>();
        parts.resize(3, 0);

        if delta.is_positive() {
            parts[2] += delta;
        } else if parts[2] > 0 {
            parts[2] += delta;
        } else if parts[1] > 0 {
            parts[1] += delta;
        } else {
            parts[0] = (parts[0] + delta).max(0);
        }

        format!("{}.{}.{}", parts[0], parts[1], parts[2])
    }

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

    #[test]
    fn build_timestamp_comparison_orders_same_version_builds() {
        assert!(build_timestamp_is_newer("2026-06-12 10:00:00 UTC", "2026-06-11 18:50:49 UTC"));
        assert!(!build_timestamp_is_newer("2026-06-11 18:50:49 UTC", "2026-06-12 10:00:00 UTC"));
        assert!(!build_timestamp_is_newer("2026-06-11 18:50:49 UTC", "2026-06-11 18:50:49 UTC"));
    }

    #[test]
    fn parse_built_line_reads_version_output() {
        let output = "kalam 0.5.2-rc.2\nCommit: abc (main)\nBuilt: 2026-06-11 18:50:49 UTC\n";
        assert_eq!(parse_built_line(output), Some("2026-06-11 18:50:49 UTC".to_string()));
    }

    #[test]
    fn parse_cli_build_date_from_manifest_reads_cli_component() {
        let manifest = r#"{
            "packages": {
                "core_components": {
                    "cli": {
                        "version": "0.5.2-rc.2",
                        "build_date": "2026-06-12 10:00:00 UTC"
                    }
                }
            }
        }"#;
        assert_eq!(
            parse_cli_build_date_from_manifest(manifest),
            Some("2026-06-12 10:00:00 UTC".to_string())
        );
    }

    #[test]
    fn parse_cli_build_date_from_manifest_returns_none_when_missing() {
        let manifest = r#"{
            "packages": {
                "core_components": {
                    "cli": {
                        "version": "0.5.2-rc.2"
                    }
                }
            }
        }"#;
        assert_eq!(parse_cli_build_date_from_manifest(manifest), None);
    }

    #[test]
    fn update_needed_for_release_prefers_version_then_build_date() {
        assert!(update_needed_for_release(&adjacent_core_version(1), None));
        assert!(!update_needed_for_release(
            &adjacent_core_version(-1),
            Some("2099-01-01 00:00:00 UTC")
        ));
        assert!(update_needed_for_release(CLI_VERSION, Some("2099-01-01 00:00:00 UTC")));
        assert!(!update_needed_for_release(CLI_VERSION, Some("2000-01-01 00:00:00 UTC")));
    }
}
