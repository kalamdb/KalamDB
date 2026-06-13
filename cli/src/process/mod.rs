//! Cross-platform process and shell execution for the Kalam CLI.
//!
//! All child-process spawning in production code should go through this module
//! rather than constructing `Command` values ad hoc in feature modules.

mod lifecycle;
mod path;
mod runner;
mod shell;

pub use lifecycle::{
    configure_supervised_child, kill_child_process_tree, kill_process_tree_by_pid,
    kill_supervised_child, kill_supervised_process_by_pid, SupervisedKillScope,
};
pub use path::{
    program_needs_shell_launch, resolve_node_binary, resolve_program_on_path,
    shell_working_directory,
};
pub use runner::{
    run_path_tool, run_program, run_program_configured, run_shell_script,
    run_shell_script_inherited, spawn_detached, spawn_program_piped, spawn_shell_piped,
};
pub use shell::{
    configure_shell_command, configure_tokio_shell_command, quote_for_shell, shell_command,
    shell_command_program, tokio_shell_command,
};
