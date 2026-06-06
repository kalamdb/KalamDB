use std::fs;

use crate::common::*;
use tempfile::TempDir;

#[test]
fn test_project_workflow_init_help_surface() {
    let mut cmd = create_cli_command();
    cmd.args(["init", "--help"]);

    let output = cmd.output().expect("run init help");
    assert!(
        output.status.success(),
        "init help should succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Scaffold") || stdout.contains("Initialize"),
        "init help should describe project scaffolding\nstdout: {}",
        stdout
    );
    assert!(stdout.contains("--name"));
    assert!(stdout.contains("--schema-mode"));
    assert!(stdout.contains("--languages"));
    assert!(stdout.contains("--server-mode"));
    assert!(stdout.contains("--server-url"));
    assert!(stdout.contains("--yes"));
}

#[test]
fn test_project_workflow_init_scaffolds_project() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("demo-app");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut cmd = create_cli_command();
    cmd.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "demo-app",
        "--schema-mode",
        "sql",
        "--languages",
        "typescript,dart",
    ]);

    let output = cmd.output().expect("run init");
    assert!(
        output.status.success(),
        "init should succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(project_dir.join("kalam.toml").is_file(), "kalam.toml missing");
    assert!(project_dir.join("schema.sql").is_file(), "schema.sql missing");
    assert!(
        project_dir.join("kalam/migrations/.gitkeep").is_file(),
        "migrations dir missing"
    );
    assert!(project_dir.join("src/generated").is_dir(), "typescript output dir missing");
    assert!(project_dir.join("lib/generated").is_dir(), "dart output dir missing");
    assert!(project_dir.join(".env.example").is_file(), ".env.example missing");

    let kalam_toml = fs::read_to_string(project_dir.join("kalam.toml")).expect("read kalam.toml");
    assert!(kalam_toml.contains("name = \"demo-app\""));
    assert!(kalam_toml.contains("mode = \"sql\""));
    assert!(kalam_toml.contains("typescript"));
    assert!(kalam_toml.contains("dart"));
}
