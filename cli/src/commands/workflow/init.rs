use kalam_cli::{
    workflow::project::init::{init_project, InitOptions},
    CLIError, Result, CLI_VERSION,
};

use crate::args::{Cli, InitArgs};

pub(super) async fn handle_init(cli: &Cli, args: &InitArgs) -> Result<()> {
    if args.list_templates {
        return list_init_templates(cli.json);
    }

    let cwd = args
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));

    init_project(
        InitOptions {
            name: args.name.clone(),
            schema_mode: args.schema_mode.map(Into::into),
            languages: args.languages.clone(),
            template: args.template.clone(),
            package_manager: args.package_manager.map(Into::into),
            server_mode: args.server_mode.map(Into::into),
            server_url: args.server_url.clone(),
            yes: args.yes,
            cwd,
        },
        !cli.no_color,
        !cli.no_spinner,
        cli.json,
    )
    .await
}

fn list_init_templates(json: bool) -> Result<()> {
    let templates = kalam_cli::workflow::project::init::list_init_templates();
    let payload = serde_json::json!({
        "ok": true,
        "cli_version": CLI_VERSION,
        "default_template": "simple-live",
        "next": "kalam init --yes --template <id> --languages typescript --package-manager npm && kalam dev --agent",
        "templates": templates,
    });
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&payload)
                .map_err(|error| CLIError::FormatError(error.to_string()))?
        );
        return Ok(());
    }

    println!("id\tkind\tlanguage\tdescription");
    for template in &templates {
        println!(
            "{}\t{}\t{}\t{}",
            template.id, template.kind, template.language, template.description
        );
    }
    println!();
    println!("Next: kalam init --yes --template <id> --languages typescript --package-manager npm");
    println!("Then: kalam dev --agent");
    Ok(())
}
