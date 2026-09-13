use super::*;

#[test]
fn pty_spawn_terminal_size_uses_request_size_when_present() {
    assert_eq!(
        pty_spawn_terminal_size(Some(command_service_api::ExecCommandTerminalSize {
            rows: 41,
            cols: 132,
        })),
        codex_utils_pty::TerminalSize {
            rows: 41,
            cols: 132,
        }
    );
    assert_eq!(
        pty_spawn_terminal_size(None),
        codex_utils_pty::TerminalSize::default()
    );
}
