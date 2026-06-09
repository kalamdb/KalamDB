//! Project configuration types and discovery for `kalam.toml`.

use std::{
    collections::HashMap,
    fs,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::error::{CLIError, Result};

pub const KALAM_TOML: &str = "kalam.toml";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KalamProjectConfig {
    pub project: ProjectSection,
    #[serde(default)]
    pub connection: HashMap<String, ConnectionEnv>,
    pub schema: SchemaSection,
    #[serde(default)]
    pub migrations: MigrationsSection,
    #[serde(default)]
    pub dev: DevSection,
    #[serde(default)]
    pub logging: LoggingSection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectSection {
    pub name: String,
    #[serde(default = "default_env_name")]
    pub default_env: String,
    #[serde(
        default = "default_kalam_dir",
        skip_serializing_if = "is_default_kalam_dir"
    )]
    pub kalam_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionEnv {
    pub url: String,
    pub namespace: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaSection {
    pub mode: SchemaMode,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default = "default_true")]
    pub watch: bool,
    #[serde(default = "default_languages")]
    pub languages: Vec<String>,
    #[serde(default)]
    pub targets: HashMap<String, SchemaTarget>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SchemaMode {
    Sql,
    Remote,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaTarget {
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MigrationsSection {
    #[serde(default = "default_migrations_dir", skip_serializing)]
    pub dir: String,
    #[serde(default = "default_true")]
    pub auto_create: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DevSection {
    #[serde(default = "default_true")]
    pub auto_start_db: bool,
    #[serde(default = "default_true")]
    pub apply_schema: bool,
    #[serde(default = "default_true")]
    pub generate_types: bool,
    #[serde(default = "default_true")]
    pub watch: bool,
    #[serde(default)]
    pub processes: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoggingSection {
    #[serde(default = "default_true")]
    pub file: bool,
    #[serde(default = "default_log_path", skip_serializing)]
    pub path: String,
    #[serde(default = "default_true")]
    pub capture_process_output: bool,
}

fn default_env_name() -> String {
    "dev".to_string()
}

fn default_kalam_dir() -> String {
    "kalam".to_string()
}

fn is_default_kalam_dir(value: &str) -> bool {
    value == default_kalam_dir()
}

fn default_true() -> bool {
    true
}

fn default_languages() -> Vec<String> {
    vec!["typescript".to_string()]
}

fn default_migrations_dir() -> String {
    "kalam/migrations".to_string()
}

fn default_log_path() -> String {
    "kalam/cli/logs/kalam.log".to_string()
}

impl KalamProjectConfig {
    pub fn discover(start: &Path, explicit_project_dir: Option<&Path>) -> Result<(PathBuf, Self)> {
        let root = if let Some(dir) = explicit_project_dir {
            dir.canonicalize().map_err(|e| {
                CLIError::ConfigurationError(format!(
                    "invalid project directory '{}': {}",
                    dir.display(),
                    e
                ))
            })?
        } else {
            discover_project_root(start)?
        };

        let config_path = root.join(KALAM_TOML);
        if !config_path.is_file() {
            return Err(CLIError::ConfigurationError(format!(
                "no {} found in project root '{}'",
                KALAM_TOML,
                root.display()
            )));
        }

        let config = Self::load_from_path(&config_path)?;
        Ok((root, config))
    }

    pub fn load_from_path(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path).map_err(|e| {
            CLIError::ConfigurationError(format!("failed to read '{}': {}", path.display(), e))
        })?;
        Self::parse(&contents)
    }

    pub fn parse(contents: &str) -> Result<Self> {
        let config: Self = toml::from_str(contents)?;
        config.validate()?;
        Ok(config)
    }

    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        let contents = toml::to_string_pretty(self).map_err(|e| {
            CLIError::ConfigurationError(format!("failed to serialize kalam.toml: {}", e))
        })?;
        fs::write(path, contents)?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.project.name.trim().is_empty() {
            return Err(CLIError::ConfigurationError("project.name must not be empty".into()));
        }

        validate_relative_project_path("project.kalam_dir", &self.project.kalam_dir)?;

        match self.schema.mode {
            SchemaMode::Sql => {
                let path = self.schema.path.as_deref().unwrap_or("").trim();
                if path.is_empty() {
                    return Err(CLIError::ConfigurationError(
                        "schema.path is required when schema.mode is 'sql'".into(),
                    ));
                }
            },
            SchemaMode::Remote => {},
        }

        let mut outputs = std::collections::HashSet::new();
        for language in &self.schema.languages {
            let Some(target) = self.schema.targets.get(language) else {
                return Err(CLIError::ConfigurationError(format!(
                    "schema.targets.{language} is required for language '{language}'"
                )));
            };
            if !outputs.insert(target.output.clone()) {
                return Err(CLIError::ConfigurationError(format!(
                    "duplicate schema target output path '{}'",
                    target.output
                )));
            }
        }

        Ok(())
    }

    pub fn schema_source_path(&self, project_root: &Path) -> Option<PathBuf> {
        self.schema.path.as_ref().map(|p| project_root.join(p))
    }

    pub fn kalam_dir(&self, project_root: &Path) -> PathBuf {
        project_root.join(&self.project.kalam_dir)
    }

    pub fn cli_dir(&self, project_root: &Path) -> PathBuf {
        self.kalam_dir(project_root).join("cli")
    }

    pub fn workflow_log_path(&self, project_root: &Path) -> PathBuf {
        self.cli_dir(project_root).join("logs").join("kalam.log")
    }

    pub fn ensure_cli_log_dir(&self, project_root: &Path) -> Result<PathBuf> {
        let log_dir = self.cli_dir(project_root).join("logs");
        fs::create_dir_all(&log_dir).map_err(|error| {
            CLIError::FileError(format!(
                "failed to create KalamDB CLI log directory '{}': {error}",
                log_dir.display()
            ))
        })?;
        Ok(log_dir)
    }

    pub fn ensure_kalam_gitignore(&self, project_root: &Path) -> Result<PathBuf> {
        let kalam_dir = self.kalam_dir(project_root);
        fs::create_dir_all(&kalam_dir).map_err(|error| {
            CLIError::FileError(format!(
                "failed to create KalamDB project directory '{}': {error}",
                kalam_dir.display()
            ))
        })?;

        let gitignore_path = kalam_dir.join(".gitignore");
        if !gitignore_path.exists() {
            fs::write(
                &gitignore_path,
                "# KalamDB generated local state\n/cli/logs/\n/server/\n/.schema-baseline.sql\n",
            )
            .map_err(|error| {
                CLIError::FileError(format!(
                    "failed to write '{}': {error}",
                    gitignore_path.display()
                ))
            })?;
        }
        Ok(gitignore_path)
    }

    pub fn local_server_dir(&self, project_root: &Path) -> PathBuf {
        self.kalam_dir(project_root).join("server")
    }

    pub fn local_server_config_path(&self, project_root: &Path) -> PathBuf {
        self.local_server_dir(project_root).join("server.toml")
    }

    pub fn relative_local_server_data_path(&self) -> String {
        format!("{}/server/data", normalize_project_dir_for_config(&self.project.kalam_dir))
    }

    pub fn relative_local_server_logs_path(&self) -> String {
        format!("{}/server/logs", normalize_project_dir_for_config(&self.project.kalam_dir))
    }

    pub fn migrations_dir(&self, project_root: &Path) -> PathBuf {
        self.kalam_dir(project_root).join("migrations")
    }

    /// Baseline schema snapshot used as the "before" side of migration diffs.
    pub fn schema_baseline_path(&self, project_root: &Path) -> PathBuf {
        self.kalam_dir(project_root).join(".schema-baseline.sql")
    }
}

fn validate_relative_project_path(field: &str, value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CLIError::ConfigurationError(format!("{field} must not be empty")));
    }

    let path = Path::new(trimmed);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_))
        })
    {
        return Err(CLIError::ConfigurationError(format!(
            "{field} must be a relative path inside the project"
        )));
    }

    Ok(())
}

fn normalize_project_dir_for_config(value: &str) -> String {
    value.trim().trim_matches('/').to_string()
}

fn discover_project_root(start: &Path) -> Result<PathBuf> {
    let start = if start.is_file() {
        start
            .parent()
            .ok_or_else(|| CLIError::ConfigurationError("invalid start path".into()))?
    } else {
        start
    };

    let mut current = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());

    loop {
        if current.join(KALAM_TOML).is_file() {
            return Ok(current);
        }
        if !current.pop() {
            break;
        }
    }

    Err(CLIError::ConfigurationError(format!(
        "could not find {} walking up from '{}'",
        KALAM_TOML,
        start.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_minimal_config() {
        let toml = r#"
[project]
name = "demo"
default_env = "dev"

[connection.dev]
url = "http://localhost:2900"
namespace = "app"

[schema]
mode = "sql"
path = "schema.sql"
languages = ["typescript"]

[schema.targets.typescript]
output = "src/generated/kalam.ts"
"#;
        let config = KalamProjectConfig::parse(toml).expect("parse");
        assert_eq!(config.project.name, "demo");
        assert_eq!(config.project.kalam_dir, "kalam");
        assert_eq!(config.schema.mode, SchemaMode::Sql);
    }

    #[test]
    fn project_paths_use_configured_kalam_dir() {
        let toml = r#"
[project]
name = "demo"
kalam_dir = "db"

[schema]
mode = "sql"
path = "schema.sql"

[schema.targets.typescript]
output = "src/generated/kalam.ts"
"#;
        let config = KalamProjectConfig::parse(toml).expect("parse");
        let root = Path::new("/tmp/demo");

        assert_eq!(config.kalam_dir(root), root.join("db"));
        assert_eq!(config.migrations_dir(root), root.join("db/migrations"));
        assert_eq!(config.workflow_log_path(root), root.join("db/cli/logs/kalam.log"));
        assert_eq!(config.schema_baseline_path(root), root.join("db/.schema-baseline.sql"));
        assert_eq!(config.local_server_config_path(root), root.join("db/server/server.toml"));
    }

    #[test]
    fn discover_walks_up_directories() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("proj");
        let nested = root.join("src").join("app");
        fs::create_dir_all(&nested).unwrap();
        KalamProjectConfig {
            project: ProjectSection {
                name: "demo".into(),
                default_env: "dev".into(),
                kalam_dir: "kalam".into(),
            },
            connection: HashMap::from([(
                "dev".into(),
                ConnectionEnv {
                    url: "http://localhost:2900".into(),
                    namespace: "app".into(),
                },
            )]),
            schema: SchemaSection {
                mode: SchemaMode::Sql,
                path: Some("schema.sql".into()),
                watch: true,
                languages: vec!["typescript".into()],
                targets: HashMap::from([(
                    "typescript".into(),
                    SchemaTarget {
                        output: "src/generated/kalam.ts".into(),
                    },
                )]),
            },
            migrations: MigrationsSection::default(),
            dev: DevSection::default(),
            logging: LoggingSection::default(),
        }
        .save_to_path(&root.join(KALAM_TOML))
        .unwrap();

        let (found_root, _) =
            KalamProjectConfig::discover(&nested, None).expect("discover from nested dir");
        assert_eq!(found_root, root.canonicalize().unwrap());
    }
}

impl Default for MigrationsSection {
    fn default() -> Self {
        Self {
            dir: default_migrations_dir(),
            auto_create: true,
        }
    }
}

impl Default for DevSection {
    fn default() -> Self {
        Self {
            auto_start_db: true,
            apply_schema: true,
            generate_types: true,
            watch: true,
            processes: HashMap::new(),
        }
    }
}

impl Default for LoggingSection {
    fn default() -> Self {
        Self {
            file: true,
            path: default_log_path(),
            capture_process_output: true,
        }
    }
}
