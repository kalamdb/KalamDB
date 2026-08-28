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
const DEFAULT_EXAMPLES_REF: &str = "main";

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
        description: "Topic-driven React chat with an agent worker",
        source_path: "chat-with-ai",
    },
    RepositoryExample {
        id:          "react-ai-chat",
        description: "Full React chat with approvals and attachments",
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
    copy_example_from_zip_bytes(destination_root, example, &archive_bytes)
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
        if project_path.as_os_str().is_empty() {
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
}
