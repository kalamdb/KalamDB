//! Package versions from the repo `versions.json` manifest.
//!
//! `version` is the in-repo package version. `published` is the last version
//! that was actually shipped. Released `kalam init` pins SDK deps to
//! `published` so an older CLI keeps the SDK set it shipped with.

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
    dart:       HashMap<String, PackageRecord>,
}

#[derive(Debug, Deserialize)]
struct PackageRecord {
    version:   String,
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

fn package_published(package: &PackageRecord) -> &str {
    package
        .published
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or(&package.version)
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

pub fn published_typescript_package_version(package_name: &str) -> &'static str {
    manifest()
        .packages
        .typescript
        .get(package_name)
        .map(package_published)
        .unwrap_or(CLI_VERSION)
}

pub fn published_dart_package_version(package_name: &str) -> &'static str {
    manifest()
        .packages
        .dart
        .get(package_name)
        .map(package_published)
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
        assert_eq!(published_typescript_package_version("@kalamdb/missing"), CLI_VERSION);
    }

    #[test]
    fn published_helpers_prefer_published_field() {
        let client = &manifest().packages.typescript["@kalamdb/client"];
        assert_eq!(
            published_typescript_package_version("@kalamdb/client"),
            package_published(client)
        );
        let sync = &manifest().packages.dart["kalam_sync"];
        assert_eq!(published_dart_package_version("kalam_sync"), package_published(sync));
    }
}
