//! Repository-backed example projects for `kalam init`.

use std::{
    env, fs, io,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use crate::{
    release_download::{download_bytes, GITHUB_REPO},
    workflow::{display_project_path, project::scaffold},
    CLIError, Result,
};

pub const EXAMPLES_ARCHIVE_URL_ENV: &str = "KALAM_EXAMPLES_ARCHIVE_URL";
pub const EXAMPLES_REF_ENV: &str = "KALAM_EXAMPLES_REF";
pub const EXAMPLES_DIR_ENV: &str = "KALAM_EXAMPLES_DIR";
pub const WORKSPACE_ENV: &str = "KALAM_WORKSPACE";
const DEFAULT_EXAMPLES_REF: &str = "main";
const TYPESCRIPT_SDK_PACKAGES_DIR: &str = "link/sdks/typescript";
const DART_SDK_PACKAGES_DIR: &str = "link/sdks/dart";
const DART_PACKAGES: &[(&str, &str)] = &[
    ("kalam_sync", "sync"),
    ("kalam_link", "link"),
    ("kalam_sync_generator", "generator"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepositoryExample {
    pub id:          &'static str,
    pub description: &'static str,
    pub source_path: &'static str,
}

pub const REPOSITORY_EXAMPLES: &[RepositoryExample] = &[
    RepositoryExample {
        id:          "live-okf-context-sync",
        description: "OKF folder sync with live FILE columns",
        source_path: "live-okf-context-sync",
    },
    RepositoryExample {
        id:          "realtime-ops-feed",
        description: "Small browser app with live SQL subscriptions",
        source_path: "simple-typescript",
    },
    RepositoryExample {
        id:          "chat-with-ai",
        description: "Realtime React chat with SHARED rooms, a USER inbox, and a topic-trigger \
                      procedure",
        source_path: "chat-with-ai",
    },
    RepositoryExample {
        id:          "react-ai-chat",
        description: "Personal AI assistant chat with USER tables, STREAM tokens, and approvals",
        source_path: "react-ai-chat",
    },
    RepositoryExample {
        id:          "summarizer-agent",
        description: "Worker-only topic consumer that enriches rows",
        source_path: "summarizer-agent",
    },
];

pub fn available() -> &'static [RepositoryExample] {
    REPOSITORY_EXAMPLES
}

pub fn find(id: &str) -> Option<&'static RepositoryExample> {
    REPOSITORY_EXAMPLES.iter().find(|example| {
        example.id == id || example.source_path == id || format!("example:{}", example.id) == id
    })
}

pub async fn download_repository_example(
    destination_root: &Path,
    example: &RepositoryExample,
    show_progress: bool,
) -> Result<()> {
    scaffold::io_with_guidance(
        "create project directory",
        destination_root,
        fs::create_dir_all(destination_root),
    )?;

    if let Some(source) = local_example_source(example) {
        copy_example_from_dir(destination_root, example, &source)?;
        return pin_sdk_dependencies(destination_root).await;
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(format!("kalam-cli/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| {
            CLIError::ConfigurationError(format!("failed to create HTTP client: {error}"))
        })?;
    let archive_url = examples_archive_url();
    let archive_bytes =
        download_bytes(&client, &archive_url, "KalamDB examples archive", show_progress).await?;
    copy_example_from_zip_bytes(destination_root, example, &archive_bytes)?;
    pin_sdk_dependencies(destination_root).await
}

fn examples_archive_url() -> String {
    if let Some(url) = env::var_os(EXAMPLES_ARCHIVE_URL_ENV)
        .map(|value| value.to_string_lossy().trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return url;
    }

    let repo_ref = env::var_os(EXAMPLES_REF_ENV)
        .map(|value| value.to_string_lossy().trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_EXAMPLES_REF.to_string());
    format!("https://codeload.github.com/{GITHUB_REPO}/zip/refs/heads/{repo_ref}")
}

pub(crate) fn copy_example_from_zip_bytes(
    destination_root: &Path,
    example: &RepositoryExample,
    archive_bytes: &[u8],
) -> Result<()> {
    let reader = io::Cursor::new(archive_bytes);
    let mut archive = zip::ZipArchive::new(reader).map_err(|error| {
        CLIError::FileError(format!("failed to read examples archive: {error}"))
    })?;
    let mut copied_files = 0usize;

    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|error| {
            CLIError::FileError(format!("failed to read examples archive entry: {error}"))
        })?;
        let Some(enclosed_name) = file.enclosed_name() else {
            continue;
        };
        let Some(project_path) = example_project_path(&enclosed_name, example.source_path) else {
            continue;
        };
        if project_path.as_os_str().is_empty() || should_skip_example_path(&project_path) {
            continue;
        }

        let destination = destination_root.join(&project_path);
        if destination.exists() {
            return Err(CLIError::ConfigurationError(format!(
                "cannot write example file '{}' because it already exists",
                display_project_path(destination_root, &destination)
            )));
        }

        if file.is_dir() {
            scaffold::io_with_guidance(
                "create example directory",
                &destination,
                fs::create_dir_all(&destination),
            )?;
            continue;
        }

        if let Some(parent) = destination.parent() {
            scaffold::io_with_guidance(
                "create example parent directory",
                parent,
                fs::create_dir_all(parent),
            )?;
        }

        let mut output_file = fs::File::create(&destination).map_err(|error| {
            CLIError::FileError(format!(
                "failed to create example file '{}': {error}",
                destination.display()
            ))
        })?;
        io::copy(&mut file, &mut output_file).map_err(|error| {
            CLIError::FileError(format!(
                "failed to write example file '{}': {error}",
                destination.display()
            ))
        })?;
        copied_files += 1;
    }

    if copied_files == 0 {
        return Err(CLIError::ConfigurationError(format!(
            "examples archive did not contain examples/{}",
            example.source_path
        )));
    }

    rewrite_sdk_dependencies(destination_root)?;
    ensure_env_file(destination_root)?;
    Ok(())
}

const LOCKFILE_NAMES: &[&str] = &[
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lock",
    "bun.lockb",
];

const SKIP_EXAMPLE_DIR_NAMES: &[&str] = &[
    "node_modules",
    "dist",
    "test-results",
    "playwright-report",
    "coverage",
    ".git",
    ".DS_Store",
];

/// Rewrite `@kalamdb/*` package.json specs so `kalam init` can install them.
///
/// A CLI running from this workspace points at `link/sdks/typescript/*` via
/// `file:` so unpublished local packages still install. A released CLI pins
/// each package to the `published` version baked into that CLI's
/// `versions.json`.
pub fn rewrite_sdk_dependencies(root: &Path) -> Result<()> {
    rewrite_sdk_dependencies_with(
        root,
        |name| crate::versions_manifest::published_typescript_package_version(name).to_string(),
        discover_typescript_sdk_root().as_deref(),
    )?;
    rewrite_dart_sdk_dependencies(root)
}

pub async fn pin_sdk_dependencies(root: &Path) -> Result<()> {
    rewrite_sdk_dependencies(root)
}

pub(crate) fn rewrite_sdk_dependencies_with(
    root: &Path,
    published_version: impl Fn(&str) -> String,
    local_sdk_root: Option<&Path>,
) -> Result<()> {
    let package_json_path = root.join("package.json");
    if !package_json_path.is_file() {
        return Ok(());
    }

    let raw = fs::read_to_string(&package_json_path).map_err(|error| {
        CLIError::FileError(format!("failed to read '{}': {error}", package_json_path.display()))
    })?;
    let mut value: serde_json::Value = serde_json::from_str(&raw).map_err(|error| {
        CLIError::FileError(format!("failed to parse '{}': {error}", package_json_path.display()))
    })?;

    let mut changed = false;
    let mut rewritten_file_spec = false;
    for key in [
        "dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
    ] {
        let Some(deps) = value.get_mut(key).and_then(serde_json::Value::as_object_mut) else {
            continue;
        };
        for (name, spec) in deps.iter_mut() {
            if !name.starts_with("@kalamdb/") {
                continue;
            }
            let Some(current) = spec.as_str() else {
                continue;
            };
            let next = sdk_dependency_spec(name, &published_version(name), local_sdk_root);
            if next != current {
                rewritten_file_spec |= current.starts_with("file:") || next.starts_with("file:");
                *spec = serde_json::Value::String(next);
                changed = true;
            }
        }
    }

    if !changed {
        return Ok(());
    }

    let rendered = serde_json::to_string_pretty(&value).map_err(|error| {
        CLIError::FileError(format!("failed to serialize package.json: {error}"))
    })?;
    fs::write(&package_json_path, format!("{rendered}\n")).map_err(|error| {
        CLIError::FileError(format!("failed to write '{}': {error}", package_json_path.display()))
    })?;

    if rewritten_file_spec {
        for lockfile in LOCKFILE_NAMES {
            let path = root.join(lockfile);
            if path.is_file() {
                fs::remove_file(&path).map_err(|error| {
                    CLIError::FileError(format!("failed to remove '{}': {error}", path.display()))
                })?;
            }
        }
    }
    Ok(())
}

fn sdk_dependency_spec(
    package_name: &str,
    published_version: &str,
    local_sdk_root: Option<&Path>,
) -> String {
    let short_name = package_name.strip_prefix("@kalamdb/").unwrap_or(package_name);
    if let Some(local_root) = local_sdk_root {
        let package_dir = local_root.join(short_name);
        if package_dir.join("package.json").is_file() {
            return format!("file:{}", package_dir.display());
        }
    }
    published_version.to_string()
}

pub(crate) fn rewrite_dart_sdk_dependencies(root: &Path) -> Result<()> {
    rewrite_dart_sdk_dependencies_with(root, discover_dart_sdk_root().as_deref(), |name| {
        crate::versions_manifest::published_dart_package_version(name).to_string()
    })
}

pub(crate) fn rewrite_dart_sdk_dependencies_with(
    root: &Path,
    local_sdk_root: Option<&Path>,
    published_version: impl Fn(&str) -> String,
) -> Result<()> {
    let pubspec_path = root.join("pubspec.yaml");
    if !pubspec_path.is_file() {
        return Ok(());
    }

    let raw = fs::read_to_string(&pubspec_path).map_err(|error| {
        CLIError::FileError(format!("failed to read '{}': {error}", pubspec_path.display()))
    })?;
    let rewritten = rewrite_pubspec_sdk_specs(&raw, local_sdk_root, published_version);
    if rewritten == raw {
        return Ok(());
    }

    fs::write(&pubspec_path, rewritten).map_err(|error| {
        CLIError::FileError(format!("failed to write '{}': {error}", pubspec_path.display()))
    })?;
    Ok(())
}

fn rewrite_pubspec_sdk_specs(
    raw: &str,
    local_sdk_root: Option<&Path>,
    published_version: impl Fn(&str) -> String,
) -> String {
    let mut lines = Vec::new();
    let mut skip_until_dedent: Option<usize> = None;
    for line in raw.lines() {
        if let Some(indent_len) = skip_until_dedent {
            let line_indent = line.len() - line.trim_start().len();
            if !line.trim().is_empty() && line_indent > indent_len {
                continue;
            }
            skip_until_dedent = None;
        }
        if let Some((indent, package_name)) = dart_sdk_dependency_line(line) {
            skip_until_dedent = Some(indent.len());
            if let Some(package_dir) = local_dart_package_dir(package_name, local_sdk_root) {
                lines.push(format!("{indent}{package_name}:"));
                lines.push(format!("{indent}  path: {}", package_dir.display()));
            } else {
                lines.push(format!(
                    "{indent}{package_name}: \"{}\"",
                    published_version(package_name)
                ));
            }
            continue;
        }
        lines.push(line.to_string());
    }

    let mut rendered = lines.join("\n");
    if raw.ends_with('\n') {
        rendered.push('\n');
    }
    rendered
}

fn dart_sdk_dependency_line(line: &str) -> Option<(&str, &str)> {
    let indent_len = line.len() - line.trim_start().len();
    let indent = &line[..indent_len];
    let trimmed = line.trim_start();
    for (package_name, _) in DART_PACKAGES {
        if trimmed.starts_with(*package_name)
            && trimmed.as_bytes().get(package_name.len()) == Some(&b':')
        {
            return Some((indent, *package_name));
        }
    }
    None
}

fn local_dart_package_dir(package_name: &str, local_sdk_root: Option<&Path>) -> Option<PathBuf> {
    let local_root = local_sdk_root?;
    let dir_name = DART_PACKAGES.iter().find(|(name, _)| *name == package_name)?.1;
    let package_dir = local_root.join(dir_name);
    package_dir.join("pubspec.yaml").is_file().then_some(package_dir)
}

pub fn discover_kalam_workspace() -> Option<PathBuf> {
    if let Some(explicit) = env_path(WORKSPACE_ENV) {
        return Some(explicit);
    }

    let mut starts = Vec::new();
    if let Ok(exe) = env::current_exe() {
        starts.push(exe);
    }
    if let Ok(cwd) = env::current_dir() {
        starts.push(cwd);
    }

    for start in starts {
        let mut dir = start;
        loop {
            if is_kalam_workspace(&dir) {
                return Some(dir);
            }
            if !dir.pop() {
                break;
            }
        }
    }
    None
}

pub fn discover_typescript_sdk_root() -> Option<PathBuf> {
    let root = discover_kalam_workspace()?.join(TYPESCRIPT_SDK_PACKAGES_DIR);
    root.join("client").join("package.json").is_file().then_some(root)
}

pub fn discover_dart_sdk_root() -> Option<PathBuf> {
    let root = discover_kalam_workspace()?.join(DART_SDK_PACKAGES_DIR);
    root.join("sync").join("pubspec.yaml").is_file().then_some(root)
}

fn local_examples_dir() -> Option<PathBuf> {
    if let Some(explicit) = env_path(EXAMPLES_DIR_ENV) {
        return explicit.is_dir().then_some(explicit);
    }
    let examples = discover_kalam_workspace()?.join("examples");
    examples.is_dir().then_some(examples)
}

fn local_example_source(example: &RepositoryExample) -> Option<PathBuf> {
    let source = local_examples_dir()?.join(example.source_path);
    source.is_dir().then_some(source)
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .map(|value| PathBuf::from(value.to_string_lossy().trim().to_string()))
        .filter(|value| !value.as_os_str().is_empty())
}

fn is_kalam_workspace(dir: &Path) -> bool {
    dir.join("cli").join("Cargo.toml").is_file()
        && dir.join("examples").is_dir()
        && dir
            .join(TYPESCRIPT_SDK_PACKAGES_DIR)
            .join("client")
            .join("package.json")
            .is_file()
}

pub(crate) fn copy_example_from_dir(
    destination_root: &Path,
    example: &RepositoryExample,
    source_root: &Path,
) -> Result<()> {
    let mut copied_files = 0usize;
    copy_example_tree(destination_root, source_root, Path::new(""), &mut copied_files)?;
    if copied_files == 0 {
        return Err(CLIError::ConfigurationError(format!(
            "local examples directory did not contain examples/{}",
            example.source_path
        )));
    }
    rewrite_sdk_dependencies(destination_root)?;
    ensure_env_file(destination_root)?;
    Ok(())
}

fn copy_example_tree(
    destination_root: &Path,
    source_root: &Path,
    relative: &Path,
    copied_files: &mut usize,
) -> Result<()> {
    let source = source_root.join(relative);
    let entries = fs::read_dir(&source).map_err(|error| {
        CLIError::FileError(format!(
            "failed to read example directory '{}': {error}",
            source.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            CLIError::FileError(format!(
                "failed to read example directory '{}': {error}",
                source.display()
            ))
        })?;
        let name = entry.file_name();
        let child_relative = relative.join(&name);
        if should_skip_example_path(&child_relative) {
            continue;
        }
        let file_type = entry.file_type().map_err(|error| {
            CLIError::FileError(format!(
                "failed to read example entry '{}': {error}",
                entry.path().display()
            ))
        })?;
        let destination = destination_root.join(&child_relative);
        if file_type.is_dir() {
            scaffold::io_with_guidance(
                "create example directory",
                &destination,
                fs::create_dir_all(&destination),
            )?;
            copy_example_tree(destination_root, source_root, &child_relative, copied_files)?;
            continue;
        }
        if destination.exists() {
            return Err(CLIError::ConfigurationError(format!(
                "cannot write example file '{}' because it already exists",
                display_project_path(destination_root, &destination)
            )));
        }
        if let Some(parent) = destination.parent() {
            scaffold::io_with_guidance(
                "create example parent directory",
                parent,
                fs::create_dir_all(parent),
            )?;
        }
        fs::copy(entry.path(), &destination).map_err(|error| {
            CLIError::FileError(format!(
                "failed to copy example file '{}' to '{}': {error}",
                entry.path().display(),
                destination.display()
            ))
        })?;
        *copied_files += 1;
    }
    Ok(())
}

fn should_skip_example_path(relative: &Path) -> bool {
    let posix = relative.to_string_lossy().replace('\\', "/");
    if posix.ends_with(".tsbuildinfo") || posix.ends_with(".env") {
        return true;
    }
    if posix == "kalam/.schema-baseline.sql"
        || posix.starts_with("kalam/cli/")
        || posix == "kalam/cli"
        || posix.starts_with("kalam/server/")
        || posix == "kalam/server"
        || posix.starts_with("functions/.kalam/")
        || posix == "functions/.kalam"
        || posix == "data"
        || posix.starts_with("data/")
    {
        return true;
    }
    relative.components().any(|component| match component {
        Component::Normal(name) => {
            let name = name.to_string_lossy();
            SKIP_EXAMPLE_DIR_NAMES.iter().any(|skip| *skip == name)
        },
        _ => false,
    })
}

fn ensure_env_file(root: &Path) -> Result<()> {
    let env_path = root.join(".env");
    let example_path = root.join(".env.example");
    if env_path.exists() || !example_path.is_file() {
        return Ok(());
    }
    fs::copy(&example_path, &env_path).map_err(|error| {
        CLIError::FileError(format!(
            "failed to copy '{}' to '{}': {error}",
            example_path.display(),
            env_path.display()
        ))
    })?;
    Ok(())
}

fn example_project_path(enclosed_name: &Path, example_source_path: &str) -> Option<PathBuf> {
    let components: Vec<&str> = enclosed_name.components().filter_map(component_as_str).collect();
    if components.len() < 3 || components.get(1) != Some(&"examples") {
        return None;
    }

    let source_parts: Vec<&str> = example_source_path.split('/').collect();
    let source_end = 2 + source_parts.len();
    if components.len() < source_end || components[2..source_end] != source_parts {
        return None;
    }

    Some(components[source_end..].iter().collect())
}

fn component_as_str(component: Component<'_>) -> Option<&str> {
    match component {
        Component::Normal(value) => value.to_str(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;
    use zip::{write::FileOptions, ZipWriter};

    use super::*;

    #[test]
    fn repository_examples_include_chat_with_ai() {
        let example = find("chat-with-ai").expect("chat-with-ai example");
        assert_eq!(example.source_path, "chat-with-ai");
    }

    #[test]
    fn copy_example_from_zip_extracts_only_selected_example() {
        let temp = TempDir::new().expect("tempdir");
        let mut archive = ZipWriter::new(io::Cursor::new(Vec::new()));
        let options: FileOptions<'_, ()> = FileOptions::default();
        archive
            .start_file("KalamDB-main/examples/chat-with-ai/kalam.toml", options)
            .expect("start selected file");
        io::Write::write_all(&mut archive, b"[project]\nname = \"chat-with-ai\"\n")
            .expect("write selected file");
        archive
            .start_file("KalamDB-main/examples/simple-typescript/package.json", options)
            .expect("start other file");
        io::Write::write_all(&mut archive, b"{}").expect("write other file");
        let bytes = archive.finish().expect("finish zip").into_inner();

        copy_example_from_zip_bytes(
            temp.path(),
            find("chat-with-ai").expect("chat-with-ai example"),
            &bytes,
        )
        .expect("copy example");

        assert!(temp.path().join("kalam.toml").is_file());
        assert!(!temp.path().join("package.json").exists());
    }

    #[test]
    fn copy_example_rewrites_file_sdk_deps_and_copies_env() {
        let temp = TempDir::new().expect("tempdir");
        let mut archive = ZipWriter::new(io::Cursor::new(Vec::new()));
        let options: FileOptions<'_, ()> = FileOptions::default();
        archive
            .start_file("KalamDB-main/examples/chat-with-ai/package.json", options)
            .expect("start package.json");
        io::Write::write_all(
            &mut archive,
            br#"{
  "name": "chat-with-ai",
  "dependencies": {
    "@kalamdb/client": "file:../../link/sdks/typescript/client",
    "react": "^19.0.0"
  }
}
"#,
        )
        .expect("write package.json");
        archive
            .start_file("KalamDB-main/examples/chat-with-ai/package-lock.json", options)
            .expect("start lockfile");
        io::Write::write_all(&mut archive, b"{\"lockfileVersion\": 3}\n").expect("write lockfile");
        archive
            .start_file("KalamDB-main/examples/chat-with-ai/.env.example", options)
            .expect("start env example");
        io::Write::write_all(&mut archive, b"KALAMDB_URL=http://127.0.0.1:2900\n")
            .expect("write env example");
        let bytes = archive.finish().expect("finish zip").into_inner();

        copy_example_from_zip_bytes(
            temp.path(),
            find("chat-with-ai").expect("chat-with-ai example"),
            &bytes,
        )
        .expect("copy example");

        let package_json =
            fs::read_to_string(temp.path().join("package.json")).expect("read package.json");
        if let Some(sdk_root) = discover_typescript_sdk_root() {
            let client = sdk_root.join("client");
            assert!(
                package_json
                    .contains(&format!("\"@kalamdb/client\": \"file:{}\"", client.display())),
                "expected local SDK rewrite\n{package_json}"
            );
        } else {
            assert!(package_json.contains(&format!(
                "\"@kalamdb/client\": \"{}\"",
                crate::versions_manifest::published_typescript_package_version("@kalamdb/client")
            )));
            assert!(!package_json.contains("file:"));
        }
        assert!(!temp.path().join("package-lock.json").exists());
        assert!(temp.path().join(".env").is_file());
    }

    #[test]
    fn rewrite_sdk_dependencies_pins_file_specs_without_local_sdk() {
        let temp = TempDir::new().expect("tempdir");
        fs::write(
            temp.path().join("package.json"),
            r#"{
  "dependencies": {
    "@kalamdb/client": "file:../../link/sdks/typescript/client",
    "react": "^19.0.0"
  }
}
"#,
        )
        .expect("write package.json");

        rewrite_sdk_dependencies_with(temp.path(), |_| "0.7.0-dev.0".to_string(), None)
            .expect("rewrite");

        let package_json = fs::read_to_string(temp.path().join("package.json")).expect("read");
        assert!(package_json.contains("\"@kalamdb/client\": \"0.7.0-dev.0\""));
        assert!(!package_json.contains("file:"));
        assert!(package_json.contains("\"react\": \"^19.0.0\""));
    }

    #[test]
    fn rewrite_sdk_dependencies_rewrites_published_specs_to_local_file_sdk() {
        let temp = TempDir::new().expect("tempdir");
        let sdk_root = temp.path().join("sdk");
        fs::create_dir_all(sdk_root.join("client")).expect("sdk client dir");
        fs::write(sdk_root.join("client").join("package.json"), "{}\n").expect("sdk package.json");
        fs::write(
            temp.path().join("package.json"),
            r#"{
  "dependencies": {
    "@kalamdb/client": "0.7.0-dev.0"
  }
}
"#,
        )
        .expect("write package.json");

        rewrite_sdk_dependencies_with(temp.path(), |_| "9.9.9".to_string(), Some(&sdk_root))
            .expect("rewrite");

        let package_json = fs::read_to_string(temp.path().join("package.json")).expect("read");
        let expected =
            format!("\"@kalamdb/client\": \"file:{}\"", sdk_root.join("client").display());
        assert!(package_json.contains(&expected), "{package_json}");
        assert!(!package_json.contains("0.7.0-dev.0"));
        assert!(!package_json.contains("9.9.9"));
    }

    #[test]
    fn rewrite_sdk_dependencies_pins_each_package_from_versions_manifest() {
        let temp = TempDir::new().expect("tempdir");
        fs::write(
            temp.path().join("package.json"),
            r#"{
  "dependencies": {
    "@kalamdb/client": "file:../../link/sdks/typescript/client",
    "@kalamdb/orm": "0.0.1"
  }
}
"#,
        )
        .expect("write package.json");

        rewrite_sdk_dependencies_with(
            temp.path(),
            |name| crate::versions_manifest::published_typescript_package_version(name).to_string(),
            None,
        )
        .expect("rewrite");

        let package_json = fs::read_to_string(temp.path().join("package.json")).expect("read");
        let client =
            crate::versions_manifest::published_typescript_package_version("@kalamdb/client");
        let orm = crate::versions_manifest::published_typescript_package_version("@kalamdb/orm");
        assert!(
            package_json.contains(&format!("\"@kalamdb/client\": \"{client}\"")),
            "{package_json}"
        );
        assert!(package_json.contains(&format!("\"@kalamdb/orm\": \"{orm}\"")), "{package_json}");
        assert!(!package_json.contains("file:"));
        assert!(!package_json.contains("0.0.1"));
    }

    #[test]
    fn rewrite_dart_sdk_dependencies_pins_published_version_without_local_sdk() {
        let temp = TempDir::new().expect("tempdir");
        fs::write(
            temp.path().join("pubspec.yaml"),
            "name: demo\ndependencies:\n  kalam_sync: \">=0.5.6-0 <0.6.0\"\n  flutter:\n    sdk: \
             flutter\n",
        )
        .expect("write pubspec");

        rewrite_dart_sdk_dependencies_with(temp.path(), None, |name| {
            crate::versions_manifest::published_dart_package_version(name).to_string()
        })
        .expect("rewrite");

        let pubspec = fs::read_to_string(temp.path().join("pubspec.yaml")).expect("read");
        let version = crate::versions_manifest::published_dart_package_version("kalam_sync");
        assert!(pubspec.contains(&format!("kalam_sync: \"{version}\"")), "{pubspec}");
        assert!(pubspec.contains("flutter:"), "{pubspec}");
        assert!(pubspec.contains("sdk: flutter"), "{pubspec}");
        assert!(!pubspec.contains("0.5.6"));
    }

    #[test]
    fn rewrite_dart_sdk_dependencies_rewrites_published_specs_to_local_path() {
        let temp = TempDir::new().expect("tempdir");
        let sdk_root = temp.path().join("dart");
        fs::create_dir_all(sdk_root.join("sync")).expect("sync dir");
        fs::write(sdk_root.join("sync").join("pubspec.yaml"), "name: kalam_sync\n")
            .expect("sync pubspec");
        fs::write(
            temp.path().join("pubspec.yaml"),
            "name: demo\ndependencies:\n  kalam_sync: \"0.7.0-beta.0\"\n",
        )
        .expect("write pubspec");

        rewrite_dart_sdk_dependencies_with(temp.path(), Some(&sdk_root), |name| {
            crate::versions_manifest::published_dart_package_version(name).to_string()
        })
        .expect("rewrite");

        let pubspec = fs::read_to_string(temp.path().join("pubspec.yaml")).expect("read");
        assert!(pubspec.contains("kalam_sync:"), "{pubspec}");
        assert!(
            pubspec.contains(&format!("path: {}", sdk_root.join("sync").display())),
            "{pubspec}"
        );
        assert!(!pubspec.contains("0.7.0-beta.0"));
    }

    #[test]
    fn pin_sdk_dependencies_uses_local_workspace_packages() {
        let Some(sdk_root) = discover_typescript_sdk_root() else {
            return;
        };
        let temp = TempDir::new().expect("tempdir");
        fs::write(
            temp.path().join("package.json"),
            r#"{
  "dependencies": {
    "@kalamdb/client": "0.0.1"
  }
}
"#,
        )
        .expect("write package.json");

        tokio::runtime::Runtime::new()
            .expect("tokio runtime")
            .block_on(pin_sdk_dependencies(temp.path()))
            .expect("pin");

        let package_json = fs::read_to_string(temp.path().join("package.json")).expect("read");
        assert!(
            package_json.contains(&format!(
                "\"@kalamdb/client\": \"file:{}\"",
                sdk_root.join("client").display()
            )),
            "{package_json}"
        );
        assert!(!package_json.contains("0.0.1"));
    }

    #[test]
    fn copy_example_from_dir_skips_build_artifacts() {
        let temp = TempDir::new().expect("tempdir");
        let source = temp.path().join("source");
        fs::create_dir_all(source.join("node_modules/left-pad")).expect("node_modules");
        fs::create_dir_all(source.join("kalam/server")).expect("server dir");
        fs::create_dir_all(source.join("src")).expect("src");
        fs::write(source.join("kalam.toml"), "[project]\nname = \"demo\"\n").expect("kalam.toml");
        fs::write(source.join("src/index.ts"), "export {}\n").expect("source file");
        fs::write(source.join("kalam/server/server.toml"), "port = 2900\n").expect("server.toml");
        fs::write(source.join("node_modules/left-pad/index.js"), "module.exports = {}\n")
            .expect("nested node_modules file");

        let destination = temp.path().join("destination");
        fs::create_dir_all(&destination).expect("destination");
        copy_example_from_dir(
            &destination,
            find("chat-with-ai").expect("chat-with-ai example"),
            &source,
        )
        .expect("copy local example");

        assert!(destination.join("kalam.toml").is_file());
        assert!(destination.join("src/index.ts").is_file());
        assert!(!destination.join("node_modules").exists());
        assert!(!destination.join("kalam/server/server.toml").exists());
    }
}
