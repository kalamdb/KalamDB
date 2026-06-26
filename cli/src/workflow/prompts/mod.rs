//! Shared interactive prompt helpers for workflow commands.

mod modal;

pub use modal::{read_single_key_decision, run_workflow_modal_prompt, WorkflowModalPrompt};
