//! Binary-style command coverage tests for top-level CLI commands and subcommands.
//!
//! These tests intentionally execute the compiled `kalam` binary through assert_cmd,
//! verify expected output, and cover command/subcommand combinations.

use std::time::Duration;

use crate::common::*;

#[test]
fn test_cli_top_level_and_subcommand_help_matrix_binary_style() {
    let cases: Vec<(&[&str], &[&str])> = vec![
        (&["--help"], &["Interactive SQL terminal", "--watch-schema"]),
        (&["version", "--help"], &["Print version information"]),
        (&["doctor", "--help"], &["Run local, server, and authentication diagnostics"]),
        (&["login", "--help"], &["Login and save credentials", "--oidc", "--no-browser"]),
        (&["logout", "--help"], &["Delete saved credentials", "--all"]),
        (&["whoami", "--help"], &["Show the currently authenticated user"]),
        (
            &["invite", "--help"],
            &[
                "Create an OIDC email invite",
                "--email",
                "--role",
                "--expires-in-days",
            ],
        ),
        (&["token", "--help"], &["Manage service tokens", "create"]),
        (
            &["token", "create", "--help"],
            &["Token/service account name", "--name", "--save"],
        ),
        (&["update", "--help"], &["Update this kalam binary", "--version", "--dry-run"]),
    ];

    for (args, expected_snippets) in cases {
        let mut cmd = create_cli_command();
        cmd.args(args);

        let output = cmd.output().expect("run command help");
        assert!(
            output.status.success(),
            "help should succeed for {:?}\nstdout: {}\nstderr: {}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        for expected in expected_snippets {
            assert!(
                stdout.contains(expected),
                "help output for {:?} should contain '{}'\nstdout: {}",
                args,
                expected,
                stdout
            );
        }
    }
}

#[test]
fn test_cli_login_flag_combination_validation_binary_style() {
    let invalid_cases: Vec<(&[&str], &[&str])> = vec![
        (&["login", "--local", "--oidc"], &["cannot be used with", "--oidc"]),
        (&["login", "--no-browser"], &["required arguments were not provided", "--oidc"]),
        (
            &["login", "--brokered"],
            &["required arguments were not provided", "--no-browser"],
        ),
    ];

    for (args, expected_snippets) in invalid_cases {
        let mut cmd = create_cli_command();
        cmd.args(args);

        let output = cmd.output().expect("run invalid login combo");
        assert!(
            !output.status.success(),
            "invalid args should fail for {:?}\nstdout: {}\nstderr: {}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
        for expected in expected_snippets {
            assert!(
                stderr.contains(&expected.to_lowercase()),
                "stderr for {:?} should contain '{}'\nstderr: {}",
                args,
                expected,
                stderr
            );
        }
    }
}

#[test]
fn test_cli_runtime_top_level_commands_binary_style() {
    if !is_server_running() {
        eprintln!("Skipping runtime command coverage: server not running");
        return;
    }

    // version
    {
        let mut cmd = create_cli_command();
        cmd.arg("version");
        let output = cmd.output().expect("run version");
        assert!(
            output.status.success(),
            "version should succeed\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("Commit:"),
            "version output should include commit metadata"
        );
    }

    // doctor
    {
        let mut cmd = create_cli_command();
        cmd.arg("doctor").arg("--no-color");
        let output = cmd.output().expect("run doctor");
        assert!(
            output.status.success(),
            "doctor should succeed\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Doctor summary"),
            "doctor output should include summary\nstdout: {}",
            stdout
        );
    }

    // whoami
    {
        let mut cmd = create_cli_command_with_root_auth();
        cmd.arg("whoami").arg("--no-color");
        let output = cmd.output().expect("run whoami");
        assert!(
            output.status.success(),
            "whoami should succeed\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("User:"),
            "whoami output should include user\nstdout: {}",
            stdout
        );
    }

    // logout (after storing credentials for a unique instance)
    {
        let (_tmp_home, creds_path) = create_temp_credentials_path();
        let instance = format!("logout_case_{}", generate_unique_table("inst"));

        let mut save_cmd = create_cli_command_with_root_auth();
        with_credentials_path(&mut save_cmd, &creds_path)
            .arg("--instance")
            .arg(&instance)
            .arg("--save-credentials")
            .arg("--command")
            .arg("SELECT 1")
            .arg("--no-color");

        let save_output = save_cmd.output().expect("save credentials before logout");
        if !save_output.status.success() {
            eprintln!(
                "Skipping logout runtime assertion because pre-save failed. stderr: {}",
                String::from_utf8_lossy(&save_output.stderr)
            );
            return;
        }

        let mut logout_cmd = create_cli_command();
        with_credentials_path(&mut logout_cmd, &creds_path)
            .arg("--instance")
            .arg(&instance)
            .arg("logout")
            .arg("--no-color");

        let output = logout_cmd.output().expect("run logout");
        assert!(
            output.status.success(),
            "logout should succeed\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Deleted credentials") || stdout.contains("No stored credentials"),
            "logout output should report credential state\nstdout: {}",
            stdout
        );
    }

    // invite
    {
        let mut cmd = create_cli_command_with_root_auth();
        cmd.arg("invite")
            .arg("--email")
            .arg(format!("{}@example.com", generate_unique_table("invite")))
            .arg("--role")
            .arg("user")
            .arg("--expires-in-days")
            .arg("1")
            .arg("--no-color");

        let output = cmd.output().expect("run invite");
        assert!(
            output.status.success(),
            "invite should succeed\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Created OIDC invite for") && stdout.contains("Role: user"),
            "invite output should include created invite details\nstdout: {}",
            stdout
        );
    }

    // token create
    {
        let (_tmp_home, creds_path) = create_temp_credentials_path();
        let token_name = format!("svc_{}", generate_unique_table("token"));

        let mut cmd = create_cli_command_with_root_auth();
        with_credentials_path(&mut cmd, &creds_path)
            .arg("token")
            .arg("create")
            .arg("--name")
            .arg(&token_name)
            .arg("--role")
            .arg("service")
            .arg("--save")
            .arg("--no-color");

        let output = cmd.output().expect("run token create");
        assert!(
            output.status.success(),
            "token create should succeed\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Created service token")
                && stdout.contains("Saved credential instance"),
            "token create output should include creation + save details\nstdout: {}",
            stdout
        );
    }
}

#[test]
fn test_cli_export_import_shared_table_with_args_binary_style() {
    if !is_server_running() {
        eprintln!("Skipping shared export/import coverage: server not running");
        return;
    }

    let namespace = generate_unique_namespace("cli_xfer_shared");
    let source_table = generate_unique_table("source");
    let target_table = generate_unique_table("target");
    let source = format!("{}.{}", namespace, source_table);
    let target = format!("{}.{}", namespace, target_table);

    execute_sql_as_root_via_client(&format!("CREATE NAMESPACE {}", namespace))
        .expect("create namespace");
    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {} (id BIGINT PRIMARY KEY, note VARCHAR) WITH (TYPE='SHARED')",
        source
    ))
    .expect("create shared source table");
    execute_sql_as_root_via_client(&format!(
        "INSERT INTO {} (id, note) VALUES (1, 'a'), (2, 'b')",
        source
    ))
    .expect("insert shared source rows");
    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {} (id BIGINT PRIMARY KEY, note VARCHAR) WITH (TYPE='SHARED')",
        target
    ))
    .expect("create shared target table");

    let temp_dir = TempDir::new().expect("create temp dir for export zip");
    let zip_path = temp_dir.path().join("shared-export.zip");

    let export_cmd_text = format!("export {} --output {}", source, zip_path.display());
    let mut export_cmd = create_cli_command_with_root_auth();
    export_cmd
        .arg("--no-color")
        .arg("--command")
        .arg(&export_cmd_text)
        .timeout(Duration::from_secs(180));

    let export_output = export_cmd.output().expect("run shared export command");
    assert!(
        export_output.status.success(),
        "shared export should succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&export_output.stdout),
        String::from_utf8_lossy(&export_output.stderr)
    );

    let export_stdout = String::from_utf8_lossy(&export_output.stdout);
    assert!(
        export_stdout.contains("Export job started") && export_stdout.contains("Export saved to:"),
        "shared export output should include job + save details\nstdout: {}",
        export_stdout
    );
    assert!(zip_path.exists(), "shared export ZIP should exist at {}", zip_path.display());

    let import_cmd_text = format!("import {} {}", target, zip_path.display());
    let mut import_cmd = create_cli_command_with_root_auth();
    import_cmd
        .arg("--no-color")
        .arg("--command")
        .arg(&import_cmd_text)
        .timeout(Duration::from_secs(180));

    let import_output = import_cmd.output().expect("run shared import command");
    assert!(
        import_output.status.success(),
        "shared import should succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&import_output.stdout),
        String::from_utf8_lossy(&import_output.stderr)
    );

    let import_stdout = String::from_utf8_lossy(&import_output.stdout);
    assert!(
        import_stdout.contains("Import job started")
            && import_stdout.contains("completed successfully"),
        "shared import output should include job + completion details\nstdout: {}",
        import_stdout
    );

    let count_sql = format!("SELECT COUNT(*) AS cnt FROM {}", target);
    let count_output = wait_for_sql_output_contains(&count_sql, "2", Duration::from_secs(30))
        .expect("shared import should restore at least two rows");
    assert!(
        count_output.contains("2"),
        "shared import should restore rows, output: {}",
        count_output
    );

    let _ = execute_sql_as_root_via_client(&format!("DROP TABLE IF EXISTS {}", source));
    let _ = execute_sql_as_root_via_client(&format!("DROP TABLE IF EXISTS {}", target));
    let _ = execute_sql_as_root_via_client(&format!("DROP NAMESPACE IF EXISTS {}", namespace));
}

#[test]
fn test_cli_export_import_user_table_with_all_args_binary_style() {
    if !is_server_running() {
        eprintln!("Skipping user export/import coverage: server not running");
        return;
    }

    let namespace = generate_unique_namespace("cli_xfer_user");
    let source_table = generate_unique_table("source");
    let target_table = generate_unique_table("target");
    let source = format!("{}.{}", namespace, source_table);
    let target = format!("{}.{}", namespace, target_table);
    let user_id = generate_unique_namespace("xfer_actor");
    let password = "CliXferUser123!";

    execute_sql_as_root_via_client(&format!("CREATE NAMESPACE {}", namespace))
        .expect("create namespace");
    execute_sql_as_root_via_client(&format!(
        "CREATE USER {} WITH PASSWORD '{}' ROLE user",
        user_id, password
    ))
    .expect("create transfer user");

    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {} (id BIGINT PRIMARY KEY, note VARCHAR) WITH (TYPE='USER')",
        source
    ))
    .expect("create user source table");
    execute_sql_as_root_via_client(&format!(
        "CREATE TABLE {} (id BIGINT PRIMARY KEY, note VARCHAR) WITH (TYPE='USER')",
        target
    ))
    .expect("create user target table");

    execute_sql_via_cli_as(
        &user_id,
        password,
        &format!("INSERT INTO {} (id, note) VALUES (1, 'u1'), (2, 'u2')", source),
    )
    .expect("insert user rows into source");

    let temp_dir = TempDir::new().expect("create temp dir for user export zip");
    let zip_path = temp_dir.path().join("user-export.zip");

    let export_cmd_text =
        format!("export {} --user-id {} --output {}", source, user_id, zip_path.display());

    let mut export_cmd = create_cli_command_with_root_auth();
    export_cmd
        .arg("--no-color")
        .arg("--command")
        .arg(&export_cmd_text)
        .timeout(Duration::from_secs(180));

    let export_output = export_cmd.output().expect("run user export command");
    assert!(
        export_output.status.success(),
        "user export should succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&export_output.stdout),
        String::from_utf8_lossy(&export_output.stderr)
    );

    let export_stdout = String::from_utf8_lossy(&export_output.stdout);
    assert!(
        export_stdout.contains("Export job started") && export_stdout.contains("Export saved to:"),
        "user export output should include job + save details\nstdout: {}",
        export_stdout
    );
    assert!(zip_path.exists(), "user export ZIP should exist at {}", zip_path.display());

    let import_cmd_text = format!("import {} {} --user-id {}", target, zip_path.display(), user_id);
    let mut import_cmd = create_cli_command_with_root_auth();
    import_cmd
        .arg("--no-color")
        .arg("--command")
        .arg(&import_cmd_text)
        .timeout(Duration::from_secs(180));

    let import_output = import_cmd.output().expect("run user import command");
    assert!(
        import_output.status.success(),
        "user import should succeed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&import_output.stdout),
        String::from_utf8_lossy(&import_output.stderr)
    );

    let import_stdout = String::from_utf8_lossy(&import_output.stdout);
    assert!(
        import_stdout.contains("Import job started")
            && import_stdout.contains("completed successfully"),
        "user import output should include job + completion details\nstdout: {}",
        import_stdout
    );

    let user_count_output = execute_sql_via_cli_as(
        &user_id,
        password,
        &format!("SELECT COUNT(*) AS cnt FROM {}", target),
    )
    .expect("query user imported row count");
    assert!(
        user_count_output.contains('2'),
        "user import should restore rows for user scope\noutput: {}",
        user_count_output
    );

    let _ = execute_sql_as_root_via_client(&format!("DROP TABLE IF EXISTS {}", source));
    let _ = execute_sql_as_root_via_client(&format!("DROP TABLE IF EXISTS {}", target));
    let _ = execute_sql_as_root_via_client(&format!("DROP USER IF EXISTS {}", user_id));
    let _ = execute_sql_as_root_via_client(&format!("DROP NAMESPACE IF EXISTS {}", namespace));
}
