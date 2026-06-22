use codex_core_skills_api::injection::SkillInjection;

use super::ContextualUserFragment;

#[derive(Debug, Clone, PartialEq)]
pub struct SkillInstructions {
    pub name: String,
    pub path: String,
    pub contents: String,
}

impl From<&SkillInjection> for SkillInstructions {
    fn from(skill: &SkillInjection) -> Self {
        Self {
            name: skill.name.clone(),
            path: skill.path.clone(),
            contents: skill.contents.clone(),
        }
    }
}

impl ContextualUserFragment for SkillInstructions {
    const ROLE: &'static str = "user";
    const START_MARKER: &'static str = "<skill>";
    const END_MARKER: &'static str = "</skill>";

    fn body(&self) -> String {
        format!(
            "\n<name>{}</name>\n<path>{}</path>\n{}\n",
            self.name, self.path, self.contents
        )
    }
}
