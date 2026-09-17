//! Shared context for project-aware workflow commands.

use std::path::PathBuf;

use crate::{
    config::{CLIConfiguration, WorkflowLoggingPolicy},
    error::Result,
    output::{WorkflowDisplayMode, WorkflowOutput},
    workflow::{
        project::{config::KalamProjectConfig, resolve::ResolvedEnvironment},
        target::{resolve_loaded_target, ResolvedTarget, TargetSelector},
    },
};

/// Shared workflow context for command handlers.
#[derive(Debug, Clone)]
pub struct WorkflowContext {
    pub project_root:       PathBuf,
    pub config:             KalamProjectConfig,
    pub cli_config:         CLIConfiguration,
    pub use_color:          bool,
    pub animations:         bool,
    pub agent:              bool,
    pub json:               bool,
    pub project_dir:        Option<PathBuf>,
    pub env_override:       Option<String>,
    pub namespace_override: Option<String>,
    pub url_override:       Option<String>,
    pub global:             bool,
    pub host:               Option<String>,
    pub port:               Option<u16>,
    pub instance:           Option<String>,
}

impl WorkflowContext {
    pub fn discover(
        start: &std::path::Path,
        project_dir: Option<&std::path::Path>,
        cli_config: &CLIConfiguration,
        use_color: bool,
        env_override: Option<String>,
        namespace_override: Option<String>,
        url_override: Option<String>,
    ) -> Result<Self> {
        let (project_root, config) = KalamProjectConfig::discover(start, project_dir)?;
        Ok(Self {
            project_root,
            config,
            cli_config: cli_config.clone(),
            use_color,
            animations: true,
            agent: false,
            json: false,
            project_dir: project_dir.map(PathBuf::from),
            env_override,
            namespace_override,
            url_override,
            global: false,
            host: None,
            port: None,
            instance: None,
        })
    }

    pub fn output(&self) -> WorkflowOutput {
        let logging = WorkflowLoggingPolicy::merge_global(
            &self.project_root,
            self.config.workflow_log_path(&self.project_root),
            &self.config.logging,
            self.cli_config.workflow_logging.as_ref(),
        );
        let display_mode = if self.agent {
            WorkflowDisplayMode::Agent
        } else {
            WorkflowDisplayMode::Normal
        };
        WorkflowOutput::new(self.use_color && !self.agent, logging)
            .with_animations(self.animations && !self.agent)
            .with_display_mode(display_mode)
            .with_json(self.json)
    }

    pub fn resolved_environment(&self) -> Result<ResolvedEnvironment> {
        Ok(self.resolved_target()?.to_environment())
    }

    pub fn resolved_target(&self) -> Result<ResolvedTarget> {
        resolve_loaded_target(&self.target_selector(), self.project_root.clone(), &self.config)
    }

    pub fn target_selector(&self) -> TargetSelector {
        TargetSelector {
            start_dir:   self.project_root.clone(),
            project_dir: self.project_dir.clone(),
            global:      self.global,
            env:         self.env_override.clone(),
            url:         self.url_override.clone(),
            host:        self.host.clone(),
            port:        self.port,
            namespace:   self.namespace_override.clone(),
            instance:    self.instance.clone(),
        }
    }
}

pub fn standalone_output(
    use_color: bool,
    animations: bool,
    json: bool,
    agent: bool,
) -> WorkflowOutput {
    let display_mode = if agent {
        WorkflowDisplayMode::Agent
    } else {
        WorkflowDisplayMode::Normal
    };
    WorkflowOutput::new(use_color && !agent, WorkflowLoggingPolicy::disabled())
        .with_animations(animations && !agent)
        .with_display_mode(display_mode)
        .with_json(json)
}
