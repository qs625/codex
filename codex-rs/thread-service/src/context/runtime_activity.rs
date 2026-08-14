use codex_context_manager::ContextualUserFragment;
use command_service_api::CommandNotificationFilter;
use command_service_api::RunningCommandSnapshot;
use protocol::subscriptions::PersistedSubscription;

pub(crate) struct RuntimeActivityContext {
    pub(crate) running_commands: Vec<RunningCommandSnapshot>,
    pub(crate) active_subscriptions: Vec<PersistedSubscription>,
    pub(crate) pending_poll_events: Vec<RuntimePollEventSnapshot>,
}

pub(crate) struct RuntimePollEventSnapshot {
    pub(crate) source_hint: String,
    pub(crate) item_count: usize,
}

impl RuntimeActivityContext {
    pub(crate) fn is_empty(&self) -> bool {
        self.running_commands.is_empty()
            && self.active_subscriptions.is_empty()
            && self.pending_poll_events.is_empty()
    }
}

impl ContextualUserFragment for RuntimeActivityContext {
    const ROLE: &'static str = "user";
    const START_MARKER: &'static str = "<runtime_activity>";
    const END_MARKER: &'static str = "</runtime_activity>";

    fn body(&self) -> String {
        let commands = self
            .running_commands
            .iter()
            .map(render_running_command)
            .collect::<String>();
        let subscriptions = self
            .active_subscriptions
            .iter()
            .map(render_subscription)
            .collect::<String>();
        let pending_poll_events = self
            .pending_poll_events
            .iter()
            .map(render_pending_poll_event)
            .collect::<String>();
        let command_hint = if self.running_commands.is_empty() {
            String::new()
        } else {
            "\n  <running_commands_hint>These commands are still running. Use poll_event to wait for command_output or command_exit notifications; use command_write_stdin with command_id for interactive input when needed.</running_commands_hint>".to_string()
        };
        format!(
            "\n  <running_commands count=\"{}\">{commands}\n  </running_commands>{command_hint}\n  <pending_poll_events count=\"{}\">{pending_poll_events}\n  </pending_poll_events>\n  <active_subscriptions count=\"{}\">{subscriptions}\n  </active_subscriptions>\n",
            self.running_commands.len(),
            self.pending_poll_events.len(),
            self.active_subscriptions.len(),
        )
    }
}

fn render_running_command(command: &RunningCommandSnapshot) -> String {
    let notify_on = match command.notify_on {
        CommandNotificationFilter::Output => "output",
        CommandNotificationFilter::Exit => "exit",
    };
    let label = command_label(&command.command);
    let latest_output_tail = command.latest_output_tail.as_deref().map_or_else(
        String::new,
        |output| {
            format!(
                "\n      <latest_output_tail>{}</latest_output_tail>",
                xml_escape(output)
            )
        },
    );
    format!(
        "\n    <command>\n      <command_id>{}</command_id>\n      <call_id>{}</call_id>\n      <label>{}</label>\n      <tty>{}</tty>\n      <notify_on>{notify_on}</notify_on>\n      <cwd>{}</cwd>\n      <command_text>{}</command_text>{latest_output_tail}\n    </command>",
        command.process_id,
        xml_escape(&command.call_id),
        xml_escape(&label),
        command.tty,
        xml_escape(&command.cwd.to_string_lossy()),
        xml_escape(&command.command),
    )
}

fn render_subscription(subscription: &PersistedSubscription) -> String {
    match subscription {
        PersistedSubscription::EventCommand {
            subscription_id,
            command,
            cwd,
            label,
        } => format!(
            "\n    <subscription>\n      <subscription_id>{}</subscription_id>\n      <type>event_command</type>\n      <label>{}</label>\n      <cwd>{}</cwd>\n      <command_text>{}</command_text>\n    </subscription>",
            xml_escape(subscription_id),
            xml_escape(label.as_deref().unwrap_or("")),
            xml_escape(cwd.as_deref().unwrap_or("")),
            xml_escape(command),
        ),
        PersistedSubscription::Schedule {
            subscription_id,
            schedule,
            label,
            message,
        } => format!(
            "\n    <subscription>\n      <subscription_id>{}</subscription_id>\n      <type>schedule</type>\n      <label>{}</label>\n      <schedule>{}</schedule>\n      <message>{}</message>\n    </subscription>",
            xml_escape(subscription_id),
            xml_escape(label.as_deref().unwrap_or("")),
            xml_escape(&format!("{schedule:?}")),
            xml_escape(message.as_deref().unwrap_or("")),
        ),
        PersistedSubscription::Fs {
            subscription_id,
            path,
            recursive,
            label,
        } => format!(
            "\n    <subscription>\n      <subscription_id>{}</subscription_id>\n      <type>fs</type>\n      <label>{}</label>\n      <path>{}</path>\n      <recursive>{recursive}</recursive>\n    </subscription>",
            xml_escape(subscription_id),
            xml_escape(label.as_deref().unwrap_or("")),
            xml_escape(path),
        ),
        PersistedSubscription::ProcessExit {
            subscription_id,
            session_id,
            label,
        } => format!(
            "\n    <subscription>\n      <subscription_id>{}</subscription_id>\n      <type>process_exit</type>\n      <label>{}</label>\n      <session_id>{session_id}</session_id>\n    </subscription>",
            xml_escape(subscription_id),
            xml_escape(label.as_deref().unwrap_or("")),
        ),
    }
}

fn render_pending_poll_event(snapshot: &RuntimePollEventSnapshot) -> String {
    format!(
        "\n    <pending_event>\n      <source_hint>{}</source_hint>\n      <item_count>{}</item_count>\n    </pending_event>",
        xml_escape(&snapshot.source_hint),
        snapshot.item_count,
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn command_label(command: &str) -> String {
    const MAX_LEN: usize = 80;
    let trimmed = command.trim();
    if trimmed.chars().count() <= MAX_LEN {
        return trimmed.to_string();
    }

    let mut out = trimmed.chars().take(MAX_LEN).collect::<String>();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use std::path::PathBuf;

    #[test]
    fn running_command_context_includes_wait_hint_and_latest_output_tail() {
        let context = RuntimeActivityContext {
            running_commands: vec![RunningCommandSnapshot {
                process_id: 7,
                call_id: "call_abc".to_string(),
                command: "rtk long-running".to_string(),
                cwd: AbsolutePathBuf::try_from(PathBuf::from("/repo")).expect("absolute path"),
                tty: false,
                notify_on: CommandNotificationFilter::Output,
                latest_output_tail: Some("tail <&>".to_string()),
            }],
            active_subscriptions: Vec::new(),
            pending_poll_events: Vec::new(),
        };

        let rendered = context.render();

        assert!(rendered.contains("<runtime_activity>"));
        assert!(rendered.contains("<running_commands count=\"1\">"));
        assert!(rendered.contains("<command_id>7</command_id>"));
        assert!(rendered.contains("<call_id>call_abc</call_id>"));
        assert!(rendered.contains("<notify_on>output</notify_on>"));
        assert!(rendered.contains("<latest_output_tail>tail &lt;&amp;&gt;</latest_output_tail>"));
        assert!(rendered.contains("Use poll_event to wait for command_output or command_exit"));
        assert!(rendered.contains("command_write_stdin with command_id"));
    }

    #[test]
    fn running_command_context_omits_empty_output_tail() {
        let context = RuntimeActivityContext {
            running_commands: vec![RunningCommandSnapshot {
                process_id: 8,
                call_id: "call_no_output".to_string(),
                command: "rtk sleep 30".to_string(),
                cwd: AbsolutePathBuf::try_from(PathBuf::from("/repo")).expect("absolute path"),
                tty: false,
                notify_on: CommandNotificationFilter::Exit,
                latest_output_tail: None,
            }],
            active_subscriptions: Vec::new(),
            pending_poll_events: Vec::new(),
        };

        let rendered = context.render();

        assert!(rendered.contains("<notify_on>exit</notify_on>"));
        assert!(!rendered.contains("<latest_output_tail>"));
    }

    #[test]
    fn pending_poll_event_context_keeps_source_hint_visible() {
        let context = RuntimeActivityContext {
            running_commands: Vec::new(),
            active_subscriptions: Vec::new(),
            pending_poll_events: vec![RuntimePollEventSnapshot {
                source_hint: "command_output".to_string(),
                item_count: 2,
            }],
        };

        let rendered = context.render();

        assert!(rendered.contains("<pending_poll_events count=\"1\">"));
        assert!(rendered.contains("<source_hint>command_output</source_hint>"));
        assert!(rendered.contains("<item_count>2</item_count>"));
    }
}
