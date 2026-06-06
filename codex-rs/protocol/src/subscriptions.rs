use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleWeekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(tag = "kind")]
pub enum ScheduleSpec {
    OnceAfter {
        /// Trigger once after this many milliseconds.
        delay_ms: u64,
    },
    OnceAt {
        /// Trigger once at this RFC 3339 timestamp.
        run_at: String,
    },
    EveryInterval {
        /// Trigger repeatedly at this fixed interval in milliseconds.
        interval_ms: u64,
    },
    EveryDayAt {
        /// Local wall-clock time in `HH:MM` or `HH:MM:SS` format.
        time: String,
        /// IANA timezone name such as `Asia/Shanghai` or `America/Los_Angeles`.
        timezone: String,
    },
    EveryWeekAt {
        /// One or more weekdays to trigger on.
        weekdays: Vec<ScheduleWeekday>,
        /// Local wall-clock time in `HH:MM` or `HH:MM:SS` format.
        time: String,
        /// IANA timezone name such as `Asia/Shanghai` or `America/Los_Angeles`.
        timezone: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(tag = "type")]
pub enum PersistedSubscription {
    #[serde(skip_serializing)]
    Fs {
        subscription_id: String,
        path: String,
        recursive: bool,
        label: Option<String>,
    },
    EventCommand {
        subscription_id: String,
        command: String,
        cwd: Option<String>,
        label: Option<String>,
    },
    Schedule {
        subscription_id: String,
        schedule: ScheduleSpec,
        label: Option<String>,
    },
    #[serde(skip_serializing)]
    ProcessExit {
        subscription_id: String,
        session_id: i32,
        label: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::PersistedSubscription;

    #[test]
    fn deserializes_legacy_persisted_subscription_variants() {
        let fs = serde_json::from_value::<PersistedSubscription>(serde_json::json!({
            "type": "fs",
            "subscription_id": "sub-fs",
            "path": "/tmp/out.log",
            "recursive": false,
            "label": null
        }))
        .unwrap();
        let process_exit = serde_json::from_value::<PersistedSubscription>(serde_json::json!({
            "type": "process_exit",
            "subscription_id": "sub-process",
            "session_id": 42,
            "label": "tests"
        }))
        .unwrap();

        assert_eq!(
            fs,
            PersistedSubscription::Fs {
                subscription_id: "sub-fs".to_string(),
                path: "/tmp/out.log".to_string(),
                recursive: false,
                label: None,
            }
        );
        assert_eq!(
            process_exit,
            PersistedSubscription::ProcessExit {
                subscription_id: "sub-process".to_string(),
                session_id: 42,
                label: Some("tests".to_string()),
            }
        );
    }
}
