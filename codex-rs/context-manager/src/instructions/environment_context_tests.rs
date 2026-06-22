use super::*;
use pretty_assertions::assert_eq;
use std::path::PathBuf;

fn test_abs_path(unix_path: &str) -> AbsolutePathBuf {
    AbsolutePathBuf::try_from(PathBuf::from(unix_path)).expect("test path should be absolute")
}

#[test]
fn serialize_workspace_write_environment_context() {
    let cwd = PathBuf::from("/repo");
    let context = EnvironmentContext::new(
        vec![EnvironmentContextEnvironment::new(
            "local",
            test_abs_path("/repo"),
            "bash",
        )],
        Some("2026-02-26".to_string()),
        Some("America/Los_Angeles".to_string()),
        /*network*/ None,
    );

    let expected = format!(
        r#"<environment_context>
  <cwd>{cwd}</cwd>
  <shell>bash</shell>
  <current_date>2026-02-26</current_date>
  <timezone>America/Los_Angeles</timezone>
</environment_context>"#,
        cwd = cwd.display(),
    );

    assert_eq!(context.render(), expected);
}

#[test]
fn serialize_environment_context_with_network() {
    let network = NetworkContext::new(
        vec!["api.example.com".to_string(), "*.openai.com".to_string()],
        vec!["blocked.example.com".to_string()],
    );
    let context = EnvironmentContext::new(
        vec![EnvironmentContextEnvironment::new(
            "local",
            test_abs_path("/repo"),
            "bash",
        )],
        Some("2026-02-26".to_string()),
        Some("America/Los_Angeles".to_string()),
        Some(network),
    );

    let expected = format!(
        r#"<environment_context>
  <cwd>{}</cwd>
  <shell>bash</shell>
  <current_date>2026-02-26</current_date>
  <timezone>America/Los_Angeles</timezone>
  <network enabled="true"><allowed>api.example.com,*.openai.com</allowed><denied>blocked.example.com</denied></network>
</environment_context>"#,
        PathBuf::from("/repo").display()
    );

    assert_eq!(context.render(), expected);
}

#[test]
fn serialize_read_only_environment_context() {
    let context = EnvironmentContext::new(
        Vec::new(),
        Some("2026-02-26".to_string()),
        Some("America/Los_Angeles".to_string()),
        /*network*/ None,
    );

    let expected = r#"<environment_context>
  <current_date>2026-02-26</current_date>
  <timezone>America/Los_Angeles</timezone>
</environment_context>"#;

    assert_eq!(context.render(), expected);
}

#[test]
fn equals_except_shell_compares_cwd() {
    let context1 = EnvironmentContext::new(
        vec![EnvironmentContextEnvironment::new(
            "local",
            test_abs_path("/repo"),
            "bash",
        )],
        /*current_date*/ None,
        /*timezone*/ None,
        /*network*/ None,
    );
    let context2 = EnvironmentContext::new(
        vec![EnvironmentContextEnvironment::new(
            "local",
            test_abs_path("/repo"),
            "bash",
        )],
        /*current_date*/ None,
        /*timezone*/ None,
        /*network*/ None,
    );
    assert!(context1.equals_except_shell(&context2));
}

#[test]
fn equals_except_shell_compares_cwd_differences() {
    let context1 = EnvironmentContext::new(
        vec![EnvironmentContextEnvironment::new(
            "local",
            test_abs_path("/repo1"),
            "bash",
        )],
        /*current_date*/ None,
        /*timezone*/ None,
        /*network*/ None,
    );
    let context2 = EnvironmentContext::new(
        vec![EnvironmentContextEnvironment::new(
            "local",
            test_abs_path("/repo2"),
            "bash",
        )],
        /*current_date*/ None,
        /*timezone*/ None,
        /*network*/ None,
    );

    assert!(!context1.equals_except_shell(&context2));
}

#[test]
fn equals_except_shell_ignores_shell() {
    let context1 = EnvironmentContext::new(
        vec![EnvironmentContextEnvironment::new(
            "local",
            test_abs_path("/repo"),
            "bash",
        )],
        /*current_date*/ None,
        /*timezone*/ None,
        /*network*/ None,
    );
    let context2 = EnvironmentContext::new(
        vec![EnvironmentContextEnvironment::new(
            "other",
            test_abs_path("/repo"),
            "zsh",
        )],
        /*current_date*/ None,
        /*timezone*/ None,
        /*network*/ None,
    );

    assert!(context1.equals_except_shell(&context2));
}

#[test]
fn serialize_environment_context_with_multiple_selected_environments() {
    let local_cwd = PathBuf::from("/repo/local");
    let remote_cwd = PathBuf::from("/repo/remote");
    let context = EnvironmentContext::new(
        vec![
            EnvironmentContextEnvironment::new("local", test_abs_path("/repo/local"), "bash"),
            EnvironmentContextEnvironment::new("remote", test_abs_path("/repo/remote"), "bash"),
        ],
        Some("2026-02-26".to_string()),
        Some("America/Los_Angeles".to_string()),
        /*network*/ None,
    );

    let expected = format!(
        r#"<environment_context>
  <environments>
    <environment id="local">
      <cwd>{}</cwd>
      <shell>bash</shell>
    </environment>
    <environment id="remote">
      <cwd>{}</cwd>
      <shell>bash</shell>
    </environment>
  </environments>
  <current_date>2026-02-26</current_date>
  <timezone>America/Los_Angeles</timezone>
</environment_context>"#,
        local_cwd.display(),
        remote_cwd.display()
    );

    assert_eq!(context.render(), expected);
}

#[test]
fn serialize_environment_context_prefers_environment_shell_when_present() {
    let local_cwd = PathBuf::from("/repo/local");
    let remote_cwd = PathBuf::from("/repo/remote");
    let context = EnvironmentContext::new(
        vec![
            EnvironmentContextEnvironment::new("local", test_abs_path("/repo/local"), "powershell"),
            EnvironmentContextEnvironment::new("remote", test_abs_path("/repo/remote"), "cmd"),
        ],
        /*current_date*/ None,
        /*timezone*/ None,
        /*network*/ None,
    );

    let expected = format!(
        r#"<environment_context>
  <environments>
    <environment id="local">
      <cwd>{}</cwd>
      <shell>powershell</shell>
    </environment>
    <environment id="remote">
      <cwd>{}</cwd>
      <shell>cmd</shell>
    </environment>
  </environments>
</environment_context>"#,
        local_cwd.display(),
        remote_cwd.display()
    );

    assert_eq!(context.render(), expected);
}
