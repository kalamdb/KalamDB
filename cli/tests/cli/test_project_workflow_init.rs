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
    assert!(project_dir.join("kalam/server/server.toml").is_file(), "server config missing");
    assert!(project_dir.join("kalam/cli/logs").is_dir(), "CLI logs dir missing");
    assert!(project_dir.join("src/generated").is_dir(), "typescript output dir missing");
    assert!(project_dir.join("lib/generated").is_dir(), "dart output dir missing");
    assert!(project_dir.join(".env.example").is_file(), ".env.example missing");

    let kalam_toml = fs::read_to_string(project_dir.join("kalam.toml")).expect("read kalam.toml");
    assert!(kalam_toml.contains("name = \"demo-app\""));
    assert!(kalam_toml.contains("mode = \"sql\""));
    assert!(
        kalam_toml.contains("languages = [")
            && kalam_toml.contains("\"typescript\"")
            && kalam_toml.contains("\"dart\""),
        "expected typescript and dart languages in kalam.toml\n{kalam_toml}"
    );
    assert!(kalam_toml.contains("[schema.targets.typescript]"));
    assert!(kalam_toml.contains("[schema.targets.dart]"));
    assert!(!kalam_toml.contains("&quot;"));
    assert!(!kalam_toml.contains("path = \".kalam/logs/kalam.log\""));
    assert!(!kalam_toml.contains("dir = \"kalam/migrations\""));

    let kalam_gitignore =
        fs::read_to_string(project_dir.join("kalam/.gitignore")).expect("read kalam .gitignore");
    assert!(kalam_gitignore.contains("/cli/logs/"));
    assert!(kalam_gitignore.contains("/server/"));
    assert!(!kalam_gitignore.contains(".kalam-state.json"));
}

#[test]
fn test_project_workflow_init_defaults_to_typescript_and_scaffolds_starter() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("demo-defaults");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut cmd = create_cli_command();
    cmd.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "demo-defaults",
        "--schema-mode",
        "sql",
    ]);

    let output = cmd.output().expect("run init");
    assert!(
        output.status.success(),
        "init should succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let kalam_toml =
        fs::read_to_string(project_dir.join("kalam.toml")).expect("read generated kalam.toml");
    assert!(kalam_toml.contains("languages = [\"typescript\"]"));
    assert!(kalam_toml.contains("[schema.targets.typescript]"));
    assert!(!kalam_toml.contains("[schema.targets.dart]"));
    assert!(!kalam_toml.contains("&quot;"));
    assert!(project_dir.join("src/generated").is_dir(), "typescript output dir missing");
    assert!(
        !project_dir.join("lib/generated").exists(),
        "dart output dir should not be scaffolded by default"
    );

    let gitignore =
        fs::read_to_string(project_dir.join(".gitignore")).expect("read generated .gitignore");
    assert!(gitignore.contains("kalam/cli/logs/"));
    assert!(gitignore.contains("kalam/server/"));
    assert!(gitignore.contains("kalam/.schema-baseline.sql"));
    assert!(gitignore.contains("node_modules/"));
    assert!(!gitignore.contains(".dart_tool/"));

    let server_toml = fs::read_to_string(project_dir.join("kalam/server/server.toml"))
        .expect("read generated server.toml");
    assert!(server_toml.contains("[rate_limit]"));
    assert!(server_toml.contains("max_queries_per_sec = 100000"));
    assert!(server_toml.contains("port = 2900"));

    let package_json =
        fs::read_to_string(project_dir.join("package.json")).expect("read generated package.json");
    assert!(package_json.contains("@kalamdb/client"));
    assert!(package_json.contains("@kalamdb/orm"));
    let tsconfig = fs::read_to_string(project_dir.join("tsconfig.json"))
        .expect("read generated tsconfig.json");
    assert!(tsconfig.contains(r#""types": ["node"]"#));
    let index_ts =
        fs::read_to_string(project_dir.join("src/index.ts")).expect("read generated src/index.ts");
    assert!(index_ts.contains("createClient"));
    assert!(index_ts.contains("liveTable"));
    assert!(project_dir.join("tsconfig.json").is_file(), "tsconfig.json missing");
}

#[test]
fn test_project_workflow_init_preserves_existing_gitignore() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("demo-gitignore");
    fs::create_dir_all(&project_dir).expect("create project dir");
    fs::write(project_dir.join(".gitignore"), "custom-ignore\n").expect("write existing gitignore");

    let mut cmd = create_cli_command();
    cmd.current_dir(&project_dir).args([
        "init",
        "--yes",
        "--name",
        "demo-gitignore",
        "--schema-mode",
        "sql",
        "--languages",
        "typescript",
    ]);

    let output = cmd.output().expect("run init");
    assert!(
        output.status.success(),
        "init should succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let gitignore =
        fs::read_to_string(project_dir.join(".gitignore")).expect("read generated .gitignore");
    assert_eq!(gitignore, "custom-ignore\n");
}

fn create_local_only_cli_command() -> assert_cmd::Command {
    let mut cmd = assert_cmd::Command::new(env!("CARGO_BIN_EXE_kalam"));
    cmd.env("KALAM_TEST_SKIP_PACKAGE_INSTALL", "1");
    cmd
}

#[test]
fn test_project_workflow_dev_requires_kalam_toml() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("uninitialized");
    fs::create_dir_all(&project_dir).expect("create project dir");

    let mut cmd = create_local_only_cli_command();
    cmd.current_dir(&project_dir).args(["dev"]);

    let output = cmd.output().expect("run dev");
    assert!(!output.status.success(), "dev should fail without kalam.toml");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("kalam.toml is missing"),
        "stderr should mention missing kalam.toml\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("kalam init"),
        "stderr should suggest kalam init\nstderr: {stderr}"
    );
}

#[test]
fn test_project_workflow_dev_rejects_invalid_kalam_toml() {
    let temp = TempDir::new().expect("temp dir");
    let project_dir = temp.path().join("broken-config");
    fs::create_dir_all(&project_dir).expect("create project dir");
    fs::write(
        project_dir.join("kalam.toml"),
        r#"
[project]
name = "test1"
default_env = "dev"

[connection.dev]
url = "http://localhost:2900"
namespace = "test1"

[schema]
path = "schema.sql"
languages = ["typescript"]

[schema.targets.typescript]
output = "src/generated/kalam.ts"
"#,
    )
    .expect("write kalam.toml");

    let mut cmd = create_local_only_cli_command();
    cmd.current_dir(&project_dir).args(["dev"]);

    let output = cmd.output().expect("run dev");
    assert!(!output.status.success(), "dev should fail for invalid kalam.toml");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("failed to parse kalam.toml"),
        "stderr should describe parse failure\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("kalam init"),
        "stderr should suggest kalam init\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("missing field `mode`"),
        "stderr should include underlying parse error\nstderr: {stderr}"
    );
}
