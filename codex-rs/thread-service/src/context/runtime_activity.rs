use codex_context_manager::ContextualUserFragment;
use command_service_api::CommandNotificationFilter;
use command_service_api::RunningCommandSnapshot;
use protocol::subscriptions::PersistedSubscription;

pub(crate) struct RuntimeActivityContext {
    pub(crate) running_commands: Vec<RunningCommandSnapshot>,
    pub(crate) active_subscriptions: Vec<PersistedSubscription>,
}

impl RuntimeActivityContext {
    pub(crate) fn is_empty(&self) -> bool {
        self.running_commands.is_empty() && self.active_subscriptions.is_empty()
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
        format!(
            "\n  <running_commands count=\"{}\">{commands}\n  </running_commands>\n  <active_subscriptions count=\"{}\">{subscriptions}\n  </active_subscriptions>\n",
            self.running_commands.len(),
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
    format!(
        "\n    <command>\n      <command_id>{}</command_id>\n      <call_id>{}</call_id>\n      <label>{}</label>\n      <tty>{}</tty>\n      <notify_on>{notify_on}</notify_on>\n      <cwd>{}</cwd>\n      <command_text>{}</command_text>\n    </command>",
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
