use std::path::PathBuf;

pub fn shlex_join(tokens: &[String]) -> String {
    shlex::try_join(tokens.iter().map(String::as_str))
        .unwrap_or_else(|_| "<command included NUL byte>".to_string())
}

pub fn resolve_executable_in_path(binary_name: &str) -> Option<PathBuf> {
    which::which(binary_name).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shlex_join_handles_nul_byte() {
        let command = vec!["echo".to_string(), "bad\0arg".to_string()];

        assert_eq!(shlex_join(&command), "<command included NUL byte>");
    }

    #[test]
    fn returns_none_for_missing_executable() {
        let binary_name = "codex-shell-utils-test-definitely-missing-executable";

        assert_eq!(resolve_executable_in_path(binary_name), None);
    }
}
