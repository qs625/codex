use super::ContextualUserFragment;

#[derive(Debug, Clone, PartialEq)]
pub struct HookAdditionalContext {
    text: String,
}

impl HookAdditionalContext {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl ContextualUserFragment for HookAdditionalContext {
    const ROLE: &'static str = "developer";
    const START_MARKER: &'static str = "";
    const END_MARKER: &'static str = "";

    fn body(&self) -> String {
        self.text.clone()
    }
}
