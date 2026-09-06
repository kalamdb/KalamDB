//! Shared modal prompt runner for workflow commands.
//!
//! Interactive prompts pause dev output, clear the terminal, and collect input
//! before restoring the session view. Use this for any workflow prompt that
//! must stay usable while `kalam dev` is streaming managed process logs.

use std::io::{self, Read};

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    terminal_ui::{self, SelectOption},
    workflow::project::prompts::{interactive_available, prompt_error},
};

pub trait WorkflowModalPrompt {
    type Decision;

    fn message(&self) -> &str;
    fn banner_title(&self) -> &str;
    fn banner_subtitle(&self) -> Option<&str>;
    fn context_lines(&self) -> &[String];
    fn select_options(&self) -> Vec<SelectOption<'static>>;
    fn default_index(&self) -> usize;
    fn decision_from_selected_index(&self, index: usize) -> Result<Self::Decision>;
    fn read_noninteractive_decision(&self) -> Result<Self::Decision>;
    fn read_agent_decision(&self) -> Result<Self::Decision> {
        self.read_noninteractive_decision()
    }
}

pub fn run_workflow_modal_prompt<P: WorkflowModalPrompt>(
    output: &WorkflowOutput,
    prompt: &P,
) -> Result<P::Decision> {
    output.workflow_event(prompt.message());
    for line in prompt.context_lines() {
        output.workflow_event(line);
    }

    output.run_terminal_modal(|| {
        terminal_ui::print_banner(
            prompt.banner_title(),
            prompt.banner_subtitle(),
            output.use_color,
        );
        println!();
        if !prompt.context_lines().is_empty() {
            for line in prompt.context_lines() {
                println!("{line}");
            }
            println!();
        }

        if output.is_agent() || output.json {
            return prompt.read_agent_decision();
        }

        if !interactive_available() {
            return prompt.read_noninteractive_decision();
        }

        let options = prompt.select_options();
        let selected = match terminal_ui::prompt_select(
            prompt.message(),
            &options,
            prompt.default_index(),
            output.use_color,
        ) {
            Ok(selected) => selected,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                return prompt.read_noninteractive_decision();
            },
            Err(error) => return Err(prompt_error(error)),
        };

        prompt.decision_from_selected_index(selected)
    })
}

pub fn read_single_key_decision<T, F>(hint: &str, decide: F) -> Result<T>
where
    F: FnOnce(char) -> Result<T>,
{
    let mut buf = [0_u8; 1];
    io::stdin()
        .read_exact(&mut buf)
        .map_err(|error| CLIError::FileError(format!("failed to read {hint}: {error}")))?;
    decide(buf[0] as char)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestPrompt;

    impl WorkflowModalPrompt for TestPrompt {
        type Decision = &'static str;

        fn message(&self) -> &str {
            "Choose one"
        }

        fn banner_title(&self) -> &str {
            "Test"
        }

        fn banner_subtitle(&self) -> Option<&str> {
            None
        }

        fn context_lines(&self) -> &[String] {
            &[]
        }

        fn select_options(&self) -> Vec<SelectOption<'static>> {
            vec![SelectOption::new("One"), SelectOption::new("Two")]
        }

        fn default_index(&self) -> usize {
            0
        }

        fn decision_from_selected_index(&self, index: usize) -> Result<Self::Decision> {
            Ok(match index {
                0 => "one",
                _ => "two",
            })
        }

        fn read_noninteractive_decision(&self) -> Result<Self::Decision> {
            Ok("noninteractive")
        }
    }

    #[test]
    fn modal_prompt_trait_maps_selected_index() {
        let prompt = TestPrompt;
        assert_eq!(prompt.decision_from_selected_index(1).unwrap(), "two");
    }
}
