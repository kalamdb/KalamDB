//! Package versions from the repo `versions.json` manifest.
//!
//! Scaffold dependencies prefer a published stable SDK from the configured
//! major/minor line. A new release line or prerelease uses the configured SDK
//! version, so an older publication cannot leak into that release's projects.

use std::{collections::HashMap, sync::OnceLock};

use serde::Deserialize;

use crate::CLI_VERSION;

const VERSIONS_JSON: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../versions.json"));

#[derive(Debug, Deserialize)]
struct VersionsManifest {
    packages: ManifestPackages,
}

#[derive(Debug, Deserialize)]
struct ManifestPackages {
    #[serde(default)]
    typescript: HashMap<String, PackageRecord>,
    #[serde(default)]
    dart: HashMap<String, PackageRecord>,
}

#[derive(Debug, Deserialize)]
struct PackageRecord {
    version: String,
    #[serde(default)]
    published: Option<String>,
}

fn manifest() -> &'static VersionsManifest {
    static MANIFEST: OnceLock<VersionsManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        serde_json::from_str(VERSIONS_JSON)
            .expect("repo versions.json must be valid JSON with package versions")
    })
}

fn scaffold_package_version(package: &PackageRecord) -> &str {
    let version = package.version.as_str();
    package
        .published
        .as_deref()
        .filter(|published| {
            !version.contains('-')
                && !published.contains('-')
                && version.split('.').take(2).eq(published.split('.').take(2))
        })
        .unwrap_or(version)
}

pub fn typescript_package_version(package_name: &str) -> Option<&'static str> {
    manifest()
        .packages
        .typescript
        .get(package_name)
        .map(|package| package.version.as_str())
}

pub fn dart_package_version(package_name: &str) -> Option<&'static str> {
    manifest()
        .packages
        .dart
        .get(package_name)
        .map(|package| package.version.as_str())
}

pub fn scaffold_typescript_package_version(package_name: &str) -> &'static str {
    manifest()
        .packages
        .typescript
        .get(package_name)
        .map(scaffold_package_version)
        .unwrap_or(CLI_VERSION)
}

pub fn scaffold_dart_package_version(package_name: &str) -> &'static str {
    manifest()
        .packages
        .dart
        .get(package_name)
        .map(scaffold_package_version)
        .unwrap_or(CLI_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_manifest_exposes_typescript_and_dart_package_versions() {
        assert_eq!(
            typescript_package_version("@kalamdb/client"),
            Some(manifest().packages.typescript["@kalamdb/client"].version.as_str())
        );
        assert!(typescript_package_version("@kalamdb/orm").is_some());
        assert!(dart_package_version("kalam_sync").is_some());
        assert!(typescript_package_version("@kalamdb/missing").is_none());
        assert_eq!(scaffold_typescript_package_version("@kalamdb/missing"), CLI_VERSION);
    }

    #[test]
    fn scaffold_versions_stay_on_the_configured_release_line() {
        let cases = [
            ("0.8.1", Some("0.8.0"), "0.8.0"),
            ("0.9.0", Some("0.8.0"), "0.9.0"),
            ("0.9.0", Some("0.9.0-rc.1"), "0.9.0"),
            ("0.9.0-rc.2", Some("0.9.0-rc.1"), "0.9.0-rc.2"),
            ("0.9.0", None, "0.9.0"),
        ];

        for (version, published, expected) in cases {
            let record = PackageRecord {
                version: version.to_string(),
                published: published.map(str::to_string),
            };
            assert_eq!(scaffold_package_version(&record), expected);
        }
    }

    #[test]
    fn scaffold_helpers_use_the_manifest() {
        let client = &manifest().packages.typescript["@kalamdb/client"];
        assert_eq!(
            scaffold_typescript_package_version("@kalamdb/client"),
            scaffold_package_version(client)
        );
        let sync = &manifest().packages.dart["kalam_sync"];
        assert_eq!(scaffold_dart_package_version("kalam_sync"), scaffold_package_version(sync));
    }
}
