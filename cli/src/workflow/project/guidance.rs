//! Actionable setup guidance for `kalam init` and related workflow failures.

use std::path::Path;

pub fn bullet_list(items: &[impl AsRef<str>]) -> String {
    items
        .iter()
        .map(|item| format!("  - {}", item.as_ref()))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn init_project_already_exists(project_dir: &Path) -> String {
    format!(
        "a KalamDB project already exists at '{}'.\n\n\
         How to fix:\n{}\n\n\
         If you meant to start fresh, move or delete the existing project directory first.",
        project_dir.display(),
        bullet_list(&[
            "Open the existing project: cd into that directory and run `kalam dev`",
            "Scaffold somewhere else: `kalam init --project-dir ./new-app`",
            "Inspect the current config: open kalam.toml in that directory",
        ])
    )
}

pub fn init_requires_non_interactive_flags() -> String {
    format!(
        "interactive setup needs a terminal (TTY), but stdin/stdout is not interactive.\n\n\
         How to fix:\n{}\n\n\
         Example:\n\
         kalam init --yes --name my-app --schema-mode sql --languages typescript \
         --server-mode local",
        bullet_list(&[
            "Rerun in a regular terminal window instead of a piped/CI shell",
            "Or pass --yes plus the flags for every choice you want to make",
            "For TypeScript, add --package-manager npm|pnpm|yarn|bun when multiple are installed",
        ])
    )
}

pub fn init_empty_project_name() -> String {
    format!(
        "project name must not be empty.\n\n\
         How to fix:\n{}",
        bullet_list(&[
            "Rerun `kalam init` and enter a name when prompted",
            "Or pass one explicitly: `kalam init --yes --name my-app`",
        ])
    )
}

pub fn init_remote_schema_unavailable() -> String {
    format!(
        "remote schema mode is not available yet.\n\n\
         How to fix:\n{}",
        bullet_list(&[
            "Use SQL schema mode: `kalam init --schema-mode sql`",
            "Keep schema.sql in the project and let `kalam dev` apply it locally",
        ])
    )
}

pub fn init_repository_templates_unavailable() -> String {
    format!(
        "loading templates from a repository is not available yet.\n\n\
         How to fix:\n{}",
        bullet_list(&[
            "Choose a built-in template during interactive init",
            "Or pass one explicitly: `kalam init --template simple-live`",
        ])
    )
}

pub fn init_invalid_server_url(url: &str, parse_error: &str) -> String {
    format!(
        "server URL '{url}' is not valid ({parse_error}).\n\n\
         How to fix:\n{}\n\n\
         Examples:\n\
         http://localhost:2900\n\
         http://127.0.0.1:2900",
        bullet_list(&[
            "Use an absolute URL with scheme and host, for example http://localhost:2900",
            "For local dev, omit --server-url and use --server-mode local",
            "For remote dev, pass the reachable API URL: `kalam init --server-mode remote --server-url http://host:2900`",
        ])
    )
}

pub fn init_unsupported_language(language: &str) -> String {
    format!(
        "unsupported language target '{language}'.\n\n\
         How to fix:\n{}\n\n\
         Example:\n\
         kalam init --yes --languages typescript\n\
         kalam init --yes --languages typescript,dart",
        bullet_list(&[
            "Use typescript and/or dart",
            "Aliases: ts is accepted for typescript",
        ])
    )
}

pub fn init_missing_language_targets() -> String {
    format!(
        "at least one generated language target is required.\n\n\
         How to fix:\n{}",
        bullet_list(&[
            "Pass --languages typescript or --languages typescript,dart",
            "In interactive mode, select at least one language in the prompt",
        ])
    )
}

pub fn init_scaffold_io_error(operation: &str, path: &Path, error: &std::io::Error) -> String {
    let mut hints = vec![
        format!("Check permissions for '{}'", path.display()),
        "Make sure the destination drive has free space".to_string(),
    ];
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        hints.insert(0, "Close programs that may lock files in the target directory".to_string());
        hints.push("On Windows, try running the terminal as Administrator or choose a user-writable folder".to_string());
    }
    if error.kind() == std::io::ErrorKind::NotFound {
        hints.insert(
            0,
            "Ensure the parent directory exists or choose a different --project-dir".to_string(),
        );
    }

    format!(
        "failed to {operation} '{}' ({error}).\n\nHow to fix:\n{}",
        path.display(),
        bullet_list(&hints)
    )
}

pub fn init_missing_scaffold_template(project_path: &str, bundle: &str) -> String {
    format!(
        "missing scaffold template file '{project_path}' in '{bundle}'.\n\n\
         How to fix:\n{}",
        bullet_list(&[
            "Reinstall or upgrade the Kalam CLI — built-in templates ship with the binary",
            "If you built from source, run `cargo build --release` in the cli workspace",
            "Report an issue if the template bundle is missing after a clean install",
        ])
    )
}

pub fn init_stage_context(stage: &str, message: String) -> String {
    format!("kalam init failed while {stage}.\n\n{message}")
}

pub fn init_config_validation_failed(detail: &str) -> String {
    format!(
        "project configuration is invalid ({detail}).\n\n\
         How to fix:\n{}",
        bullet_list(&[
            "Open kalam.toml and correct the field mentioned above",
            "Compare with a fresh scaffold: kalam init --yes --project-dir /tmp/kalam-ref",
            "Run kalam doctor if available to inspect local toolchain setup",
        ])
    )
}

pub fn dev_kalamdb_server_bin_missing(path: &Path) -> String {
    format!(
        "KALAMDB_SERVER_BIN points to a file that does not exist ('{}').\n\n\
         How to fix:\n{}",
        path.display(),
        bullet_list(&[
            "Unset the variable if you want the CLI to auto-locate the server",
            "Or point it at the real binary, for example ~/.kalam/bin/kalamdb-server",
            "On Windows, use the .exe path and open a new terminal after installing",
        ])
    )
}

pub fn dev_kalamdb_server_not_found() -> String {
    format!(
        "kalamdb-server was not found (checked KALAMDB_SERVER_BIN, ~/.kalam/bin, and PATH).\n\n\
         How to fix:\n{}\n\n\
         After the server is available, rerun:\n\
         kalam dev",
        bullet_list(&[
            "Run `kalam dev` in an interactive terminal — the CLI can download the server on first use",
            "Or set KALAMDB_SERVER_BIN to the full path of kalamdb-server",
            "Or install kalamdb-server into ~/.kalam/bin (created automatically on download)",
            "On Windows, ensure kalamdb-server.exe is on PATH or use KALAMDB_SERVER_BIN",
            "If you just downloaded the server, open a new terminal so PATH updates apply",
        ])
    )
}

pub fn dev_kalamdb_server_non_interactive_download(detail: &str) -> String {
    format!(
        "{detail}\n\n\
         How to fix:\n{}",
        bullet_list(&[
            "Rerun `kalam dev` in a regular terminal (TTY) so the CLI can download the server",
            "Or download/install kalamdb-server manually and set KALAMDB_SERVER_BIN",
            "For CI, preinstall the server binary before running kalam dev",
        ])
    )
}

pub fn dev_process_spawn_failed(name: &str, shell: &str, command: &str, error: &str) -> String {
    let mut hints = vec![
        format!("Verify the command in kalam.toml under [dev.processes.{name}]"),
        format!("Try running it manually from the project root: {command}"),
    ];
    let lower = error.to_ascii_lowercase();
    if lower.contains("not found")
        || lower.contains("no such file")
        || lower.contains("program not found")
    {
        hints.insert(
            0,
            "A program in the command is missing from PATH — install it or use an absolute path"
                .to_string(),
        );
        hints.push(
            "On Windows, npm/pnpm/yarn are often *.cmd shims — the CLI uses cmd /C for dev processes"
                .to_string(),
        );
    }
    if lower.contains("permission") || lower.contains("eacces") {
        hints.push(
            "Permission denied: check execute bits (Unix) or run the terminal as Administrator (Windows)"
                .to_string(),
        );
    }

    format!(
        "failed to start dev process '{name}' via {shell} ({error}).\n\n\
         Command:\n  {command}\n\n\
         How to fix:\n{}",
        bullet_list(&hints)
    )
}

pub fn dev_empty_process_command(name: &str) -> String {
    format!(
        "dev.processes.{name} is empty in kalam.toml.\n\n\
         How to fix:\n{}",
        bullet_list(&[
            "Remove the entry if you do not need that process",
            "Or set a shell command, for example: npm run dev",
            "Custom processes run from the project root via the system shell",
        ])
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_kalamdb_server_not_found_lists_recovery_options() {
        let message = dev_kalamdb_server_not_found();
        assert!(message.contains("kalamdb-server was not found"));
        assert!(message.contains("KALAMDB_SERVER_BIN"));
        assert!(message.contains("How to fix:"));
    }

    #[test]
    fn dev_process_spawn_failed_detects_missing_program() {
        let message = dev_process_spawn_failed("web", "sh", "npm run dev", "program not found");
        assert!(message.contains("dev process 'web'"));
        assert!(message.contains("missing from PATH"));
    }
}
