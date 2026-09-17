//! Fresh `kalam init --template` + `kalam dev start` coverage for every starter.

use std::{
    fs,
    io::Read,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread::JoinHandle,
    time::Duration,
};

use kalam_cli::workflow::project::config::KalamProjectConfig;
use tempfile::TempDir;
use wait_timeout::ChildExt;

use crate::common::*;

const INIT_TIMEOUT: Duration = Duration::from_secs(600);
const DEV_START_TIMEOUT: Duration = Duration::from_secs(240);
const DEV_STOP_TIMEOUT: Duration = Duration::from_secs(30);
const SCHEMA_GEN_TIMEOUT: Duration = Duration::from_secs(120);
const TSC_TIMEOUT: Duration = Duration::from_secs(120);
const SDK_BUILD_TIMEOUT: Duration = Duration::from_secs(900);

#[derive(Debug, Clone)]
struct ListedTemplate {
    id:       String,
    kind:     String,
    language: String,
}

struct StopOnDrop {
    home:           PathBuf,
    credentials:    PathBuf,
    project_dir:    PathBuf,
    server_bin:     PathBuf,
    workspace_root: PathBuf,
}

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        let _ = run_kalam(
            &self.home,
            &self.credentials,
            &self.project_dir,
            &self.server_bin,
            &self.workspace_root,
            &["dev", "stop"],
            DEV_STOP_TIMEOUT,
        );
    }
}

#[test]
#[ntest::timeout(2_160_000)]
fn test_project_workflow_init_templates_work_out_of_the_box() {
    let server_bin = kalamdb_server_bin().expect("kalamdb-server binary");
    let workspace = workspace_root();
    ensure_local_typescript_sdks_built(&workspace);
    let templates = list_init_templates();
    assert!(
        templates.iter().any(|template| template.id == "simple-live"),
        "expected embedded simple-live template"
    );
    assert!(
        templates.iter().any(|template| template.id == "chat-with-ai"),
        "expected chat-with-ai repository template"
    );

    let filter = std::env::var("KALAM_E2E_TEMPLATE")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let language_filter = std::env::var("KALAM_E2E_LANGUAGE")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let selected: Vec<&ListedTemplate> = templates
        .iter()
        .filter(|template| filter.as_ref().is_none_or(|id| template.id == *id))
        .filter(|template| {
            language_filter.as_ref().is_none_or(|language| template.language == *language)
        })
        .collect();
    assert!(
        !selected.is_empty(),
        "no templates selected (KALAM_E2E_TEMPLATE={filter:?} \
         KALAM_E2E_LANGUAGE={language_filter:?})"
    );

    for template in selected {
        eprintln!(
            "=== kalam init template {} ({}/{}) ===",
            template.id, template.kind, template.language
        );
        verify_template(template, &server_bin, &workspace);
    }
}

fn list_init_templates() -> Vec<ListedTemplate> {
    let output = Command::new(kalam_bin())
        .args(["init", "--list-templates", "--json"])
        .output()
        .expect("kalam init --list-templates");
    assert!(
        output.status.success(),
        "list-templates failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse list-templates json");
    payload["templates"]
        .as_array()
        .expect("templates array")
        .iter()
        .map(|template| ListedTemplate {
            id:       template["id"].as_str().expect("template id").to_string(),
            kind:     template["kind"].as_str().expect("template kind").to_string(),
            language: template["language"].as_str().expect("template language").to_string(),
        })
        .collect()
}

fn verify_template(template: &ListedTemplate, server_bin: &Path, workspace: &Path) {
    let temp = TempDir::new().expect("temp dir");
    let isolated_home = temp.path().join("home");
    fs::create_dir_all(isolated_home.join(".kalam")).expect("create isolated home");
    let credentials_path = isolated_home.join(".kalam/credentials.toml");
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).expect("create project dir");
    let (server_url, port) = unique_server_url();

    init_template(
        template,
        &isolated_home,
        &credentials_path,
        &project_dir,
        server_bin,
        workspace,
        &server_url,
    );
    point_project_at_port(&project_dir, port, &server_url);
    let _ = fs::create_dir_all(project_dir.join("data"));

    let schema_gen = run_kalam(
        &isolated_home,
        &credentials_path,
        &project_dir,
        server_bin,
        workspace,
        &["schema", "gen"],
        SCHEMA_GEN_TIMEOUT,
    );
    assert!(
        schema_gen.status.success(),
        "kalam schema gen failed for {}/{}\nstdout: {}\nstderr: {}",
        template.id,
        template.language,
        String::from_utf8_lossy(&schema_gen.stdout),
        String::from_utf8_lossy(&schema_gen.stderr)
    );

    if template.language == "typescript" {
        assert_typescript_package_uses_local_or_installed_sdk(&project_dir);
        typecheck_typescript_project(&project_dir);
    } else {
        assert!(
            project_dir.join("lib/generated/kalam.dart").is_file(),
            "dart schema gen should write lib/generated/kalam.dart"
        );
        clear_dev_processes(&project_dir);
    }

    let _guard = StopOnDrop {
        home:           isolated_home.clone(),
        credentials:    credentials_path.clone(),
        project_dir:    project_dir.clone(),
        server_bin:     server_bin.to_path_buf(),
        workspace_root: workspace.to_path_buf(),
    };
    let start = run_kalam(
        &isolated_home,
        &credentials_path,
        &project_dir,
        server_bin,
        workspace,
        &["dev", "start", "--agent"],
        DEV_START_TIMEOUT,
    );
    assert!(
        start.status.success(),
        "kalam dev start --agent failed for {}/{}\nstdout: {}\nstderr: {}\nlog: {}",
        template.id,
        template.language,
        String::from_utf8_lossy(&start.stdout),
        String::from_utf8_lossy(&start.stderr),
        read_dev_log(&project_dir)
    );

    let combined = format!(
        "{}\n{}\n{}",
        String::from_utf8_lossy(&start.stdout),
        String::from_utf8_lossy(&start.stderr),
        read_dev_log(&project_dir)
    );
    assert!(
        combined.contains("KALAM_READY"),
        "expected KALAM_READY for {}/{}\n{combined}",
        template.id,
        template.language
    );
    if project_dir.join("functions/package.json").is_file() {
        assert!(
            !combined.contains("function build failed"),
            "function build failed for {}/{}\n{combined}",
            template.id,
            template.language
        );
        assert!(
            !combined.contains("function activation failed"),
            "function activation failed for {}/{}\n{combined}",
            template.id,
            template.language
        );
    }
    if template.language == "typescript" {
        let config = KalamProjectConfig::load_from_path(&project_dir.join("kalam.toml"))
            .expect("load kalam.toml after start");
        if !config.dev.processes.is_empty() {
            assert!(
                combined.contains("KALAM_APP_STARTED"),
                "expected managed app processes to start for {}/{}\n{combined}",
                template.id,
                template.language
            );
        }
    }

    let health = tokio::runtime::Runtime::new()
        .expect("tokio runtime")
        .block_on(async { shared_http_client().get(format!("{server_url}/health")).send().await })
        .unwrap_or_else(|error| panic!("health request for {server_url}: {error}"));
    assert!(
        health.status().is_success(),
        "fresh server health check failed for {}/{} at {server_url}: {}",
        template.id,
        template.language,
        health.status()
    );
}

fn init_template(
    template: &ListedTemplate,
    home: &Path,
    credentials: &Path,
    project_dir: &Path,
    server_bin: &Path,
    workspace: &Path,
    server_url: &str,
) {
    let mut args = vec![
        "init".to_string(),
        "--yes".to_string(),
        "--name".to_string(),
        template.id.replace('-', "_"),
        "--template".to_string(),
        template.id.clone(),
        "--languages".to_string(),
        template.language.clone(),
        "--server-mode".to_string(),
        "local".to_string(),
        "--server-url".to_string(),
        server_url.to_string(),
    ];
    if template.language == "typescript" {
        args.extend(["--package-manager".to_string(), "npm".to_string()]);
    }
    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
    let output =
        run_kalam(home, credentials, project_dir, server_bin, workspace, &args_ref, INIT_TIMEOUT);
    assert!(
        output.status.success(),
        "kalam init --template {} failed\nstdout: {}\nstderr: {}",
        template.id,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        project_dir.join("kalam.toml").is_file(),
        "kalam.toml missing after init of {}",
        template.id
    );
}

fn point_project_at_port(project_dir: &Path, port: u16, server_url: &str) {
    let config_path = project_dir.join("kalam.toml");
    let mut config = KalamProjectConfig::load_from_path(&config_path).expect("load kalam.toml");
    if let Some(connection) = config.connection.get_mut("dev") {
        connection.url = server_url.to_string();
    }
    config.save_to_path(&config_path).expect("save kalam.toml");

    let env_path = project_dir.join(".env");
    let env_contents = if env_path.is_file() {
        fs::read_to_string(&env_path).expect("read .env")
    } else {
        String::new()
    };
    let rewritten = rewrite_env_urls(&env_contents, server_url, port);
    fs::write(&env_path, rewritten).expect("write .env");
}

fn rewrite_env_urls(contents: &str, server_url: &str, port: u16) -> String {
    let mut lines: Vec<String> = contents
        .lines()
        .map(|line| {
            line.replace("http://127.0.0.1:2900", server_url)
                .replace("http://localhost:2900", server_url)
                .replace(":2900", &format!(":{port}"))
        })
        .collect();
    let mut ensure = |key: &str| {
        if !lines.iter().any(|line| line.starts_with(&format!("{key}="))) {
            lines.push(format!("{key}={server_url}"));
        }
    };
    ensure("KALAM_URL");
    ensure("KALAMDB_URL");
    ensure("VITE_KALAM_URL");
    ensure("VITE_KALAMDB_URL");
    if !contents.is_empty() && !contents.ends_with('\n') {
        lines.push(String::new());
    }
    let mut rendered = lines.join("\n");
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered
}

fn clear_dev_processes(project_dir: &Path) {
    let config_path = project_dir.join("kalam.toml");
    let mut config = KalamProjectConfig::load_from_path(&config_path).expect("load kalam.toml");
    config.dev.processes.clear();
    config.save_to_path(&config_path).expect("save kalam.toml");
}

fn assert_typescript_package_uses_local_or_installed_sdk(project_dir: &Path) {
    let package_json =
        fs::read_to_string(project_dir.join("package.json")).expect("read package.json");
    assert!(
        package_json.contains("@kalamdb/client"),
        "TypeScript template should depend on @kalamdb/client\n{package_json}"
    );
    assert!(
        !package_json.contains("file:../../link/sdks/typescript"),
        "init should rewrite repo-relative SDK file: specs\n{package_json}"
    );
    assert!(
        project_dir.join("node_modules/@kalamdb/client").exists(),
        "npm install should link @kalamdb/client"
    );
}

fn typecheck_typescript_project(project_dir: &Path) {
    if !project_dir.join("tsconfig.json").is_file() {
        return;
    }
    let mut cmd = Command::new("npx");
    cmd.current_dir(project_dir)
        .args(["tsc", "--noEmit", "-p", "tsconfig.json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd.spawn().expect("spawn tsc");
    let output = wait_command_output(child, TSC_TIMEOUT, "tsc --noEmit");
    assert!(
        output.status.success(),
        "tsc --noEmit failed in {}\nstdout: {}\nstderr: {}",
        project_dir.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn ensure_local_typescript_sdks_built(workspace: &Path) {
    let sdk_root = workspace.join("link/sdks/typescript");
    let packages = [
        ("client", "dist/src/index.js"),
        ("orm", "dist/index.js"),
        ("consumer", "dist/src/index.js"),
        ("react", "dist/index.js"),
    ];
    for (name, artifact) in packages {
        let dir = sdk_root.join(name);
        let artifact_path = dir.join(artifact);
        if artifact_path.is_file() {
            continue;
        }
        eprintln!(
            "building @kalamdb/{name} so kalam init can install unpublished local file: packages"
        );
        if matches!(name, "client" | "consumer") {
            ensure_wasm_pack_available();
        }
        run_npm(&dir, &["install", "--no-audit", "--no-fund"], SDK_BUILD_TIMEOUT);
        run_npm(&dir, &["run", "build"], SDK_BUILD_TIMEOUT);
        assert!(
            artifact_path.is_file(),
            "failed to build @kalamdb/{name}; expected {}",
            artifact_path.display()
        );
    }
}

fn ensure_wasm_pack_available() {
    let status = Command::new("wasm-pack")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match status {
        Ok(status) if status.success() => {},
        _ => panic!(
            "wasm-pack is required to build unpublished @kalamdb/client and @kalamdb/consumer for \
             kalam init file: installs. Install wasm-pack 0.15.0 and the wasm32-unknown-unknown \
             Rust target."
        ),
    }
}

fn run_npm(dir: &Path, args: &[&str], timeout: Duration) {
    let mut cmd = Command::new("npm");
    cmd.current_dir(dir).args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd
        .spawn()
        .unwrap_or_else(|error| panic!("failed to spawn npm {}: {error}", args.join(" ")));
    let output = wait_command_output(child, timeout, &format!("npm {}", args.join(" ")));
    assert!(
        output.status.success(),
        "npm {} failed in {}\nstdout: {}\nstderr: {}",
        args.join(" "),
        dir.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn unique_server_url() -> (String, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    (format!("http://127.0.0.1:{port}"), port)
}

fn read_dev_log(project_dir: &Path) -> String {
    let log_path = project_dir.join("kalam/cli/logs/kalam.log");
    fs::read_to_string(log_path).unwrap_or_default()
}

fn run_kalam(
    home: &Path,
    credentials: &Path,
    project_dir: &Path,
    server_bin: &Path,
    workspace: &Path,
    args: &[&str],
    timeout: Duration,
) -> std::process::Output {
    let mut cmd = Command::new(kalam_bin());
    cmd.current_dir(project_dir)
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("KALAMDB_CREDENTIALS_PATH", credentials)
        .env("KALAMDB_SERVER_BIN", server_bin)
        .env("KALAM_WORKSPACE", workspace)
        .env("KALAM_EXAMPLES_DIR", workspace.join("examples"))
        .env("NO_PROXY", "127.0.0.1,localhost,::1")
        .env("no_proxy", "127.0.0.1,localhost,::1")
        .env_remove("KALAM_TEST_SKIP_PACKAGE_INSTALL")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    clear_workflow_url_env_overrides(&mut cmd);
    cmd.env_remove("HTTP_PROXY")
        .env_remove("http_proxy")
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy");
    let child = cmd
        .spawn()
        .unwrap_or_else(|error| panic!("failed to spawn kalam {}: {error}", args.join(" ")));
    wait_command_output(child, timeout, &format!("kalam {}", args.join(" ")))
}

fn spawn_output_reader<R>(mut reader: R) -> JoinHandle<Vec<u8>>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
        buf
    })
}

fn join_output_reader(handle: Option<JoinHandle<Vec<u8>>>) -> Vec<u8> {
    handle.and_then(|reader| reader.join().ok()).unwrap_or_default()
}

fn wait_command_output(mut child: std::process::Child, timeout: Duration, label: &str) -> Output {
    let stdout_reader = child.stdout.take().map(spawn_output_reader);
    let stderr_reader = child.stderr.take().map(spawn_output_reader);
    match child.wait_timeout(timeout).expect("wait for process") {
        Some(status) => Output {
            status,
            stdout: join_output_reader(stdout_reader),
            stderr: join_output_reader(stderr_reader),
        },
        None => {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "{label} timed out after {timeout:?}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&join_output_reader(stdout_reader)),
                String::from_utf8_lossy(&join_output_reader(stderr_reader))
            );
        },
    }
}
