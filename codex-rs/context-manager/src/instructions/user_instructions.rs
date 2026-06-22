use super::ContextualUserFragment;

#[derive(Debug, Clone, PartialEq)]
pub struct UserInstructions {
    pub directory: String,
    pub text: String,
}

impl ContextualUserFragment for UserInstructions {
    const ROLE: &'static str = "user";
    const START_MARKER: &'static str = "# AGENTS.md instructions for ";
    const END_MARKER: &'static str = "</INSTRUCTIONS>";

    fn body(&self) -> String {
        format!("{}\n\n<INSTRUCTIONS>\n{}\n", self.directory, self.text)
    }
}
