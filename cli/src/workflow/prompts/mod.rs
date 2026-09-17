//! Shared interactive prompt helpers for workflow commands.

mod input;
mod modal;

pub(crate) use input::prompt_error;
pub use input::{
    echo_prompt_selection, interactive_available, print_workflow_banner, prompt_multi_select,
    prompt_select, prompt_text, prompt_text_with_default,
};
pub use modal::{read_single_key_decision, run_workflow_modal_prompt, WorkflowModalPrompt};
