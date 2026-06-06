// Aggregator for CLI tests to ensure Cargo picks them up
//
// Run these tests with:
//   cargo test --test cli
//
// Run individual test files:
//   cargo test --test cli test_cli
//   cargo test --test cli test_cli_auth
//   cargo test --test cli test_cli_auth_admin

mod common;

#[path = "cli/test_cli.rs"]
mod test_cli;

#[path = "cli/test_cli_auth.rs"]
mod test_cli_auth;

#[path = "cli/test_cli_auth_admin.rs"]
mod test_cli_auth_admin;

#[path = "cli/test_cli_doc_matrix.rs"]
mod test_cli_doc_matrix;

#[path = "cli/test_cli_update.rs"]
mod test_cli_update;

#[path = "cli/test_cli_command_surface.rs"]
mod test_cli_command_surface;

#[path = "cli/test_project_workflow_init.rs"]
mod test_project_workflow_init;

#[path = "cli/test_project_workflow_schema.rs"]
mod test_project_workflow_schema;

#[path = "cli/test_project_workflow_dev.rs"]
mod test_project_workflow_dev;

#[path = "cli/test_project_workflow_dev_watch.rs"]
mod test_project_workflow_dev_watch;

#[path = "cli/test_project_workflow_status.rs"]
mod test_project_workflow_status;

#[path = "cli/test_project_workflow_resolution.rs"]
mod test_project_workflow_resolution;

#[path = "cli/test_project_workflow_deploy.rs"]
mod test_project_workflow_deploy;

#[path = "cli/test_project_workflow_deploy_guardrails.rs"]
mod test_project_workflow_deploy_guardrails;
