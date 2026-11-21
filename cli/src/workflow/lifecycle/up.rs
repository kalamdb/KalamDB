//! `kalam up` — start the managed local database in the background.

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        db::seed::{maybe_seed_once, SeedMode},
        instance::{self, StartedBy},
        io::display_path,
        target::{resolve_lifecycle_target, TargetSelector},
        WorkflowContext,
    },
};

pub async fn start_database(
    selector: &TargetSelector,
    explicit_port: Option<u16>,
    ctx: Option<&WorkflowContext>,
    output: &WorkflowOutput,
) -> Result<()> {
    let target = resolve_lifecycle_target(selector)?;
    if !target.is_managed_local() {
        return Err(CLIError::ConfigurationError(
            "`kalam up` starts a local database; use a local directory or `--global`, not a \
             remote `--env`"
                .into(),
        ));
    }

    let version = ctx.map(|item| item.config.resolved_server_version());
    let prepared = super::attach_or_start_managed_server(
        &target,
        explicit_port,
        StartedBy::Up,
        true,
        version,
        output,
    )
    .await?;

    if let Some(ctx) = ctx {
        let mut environment = target.to_environment();
        environment.url = prepared.record.url.clone();
        maybe_seed_once(ctx, &environment, Some(&prepared.layout), SeedMode::Once, output).await?;
    }

    let server_url = instance::browser_http_url(&prepared.record.url);
    let postgres = postgres_port(&prepared).map(instance::postgres_client_url);
    let admin_ui = instance::admin_ui_url(&server_url);
    let mut event = vec![
        ("environment", target.display_kind()),
        ("url", server_url.as_str()),
        ("admin_ui", admin_ui.as_str()),
        ("namespace", target.namespace.as_str()),
    ];
    if let Some(postgres) = postgres.as_deref() {
        event.insert(2, ("postgres", postgres));
    }
    output.agent_event("KALAM_UP", &event);
    if !output.is_agent() {
        let details = super::server_details::ServerDetails::read(&prepared.layout)?;
        let binary = server_binary_label(&prepared);
        let mut fields = Vec::with_capacity(8);
        fields.push(("Server", server_url.clone()));
        if let Some(port) = postgres_port(&prepared) {
            fields.push(("Postgres", instance::postgres_client_url(port)));
        }
        fields.push(("Admin UI", instance::admin_ui_url(&server_url)));
        if let Some(binary) = binary {
            fields.push(("Binary", binary));
        }
        fields.extend([
            ("Config", display_path(&prepared.layout.config_path)),
            ("Data", display_path(&prepared.record.data_dir)),
            ("Logs", display_path(&prepared.record.log_path)),
            ("User", "root".into()),
        ]);
        output.fields(&fields);
        output.bootstrap_password(
            details
                .bootstrap_password
                .as_deref()
                .unwrap_or("(not configured; use the server's initial login instructions)"),
        );
        output
            .detail("Initial login credentials; an existing database keeps its current password.");
        let scope = if selector.global { " -g" } else { "" };
        output.detail(format!("View logs: kalam logs{scope} --follow"));
        output.detail(format!("Stop server: kalam down{scope}"));
    }
    Ok(())
}

fn postgres_port(prepared: &super::prepare::PreparedServer) -> Option<u16> {
    prepared.record.postgres_port.or_else(|| {
        instance::parse_server_listen_ports(&prepared.layout.config_path)
            .ok()
            .and_then(|(_, port)| port)
    })
}

fn server_binary_label(prepared: &super::prepare::PreparedServer) -> Option<String> {
    let path = if prepared.program.as_os_str().is_empty() {
        prepared.record.exe.clone()?
    } else {
        prepared.program.clone()
    };
    if path.as_os_str().is_empty() || !path.is_file() {
        return None;
    }
    Some(crate::workflow::dev::server::format_server_binary(&path))
}
