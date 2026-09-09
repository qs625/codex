use super::TurnError;
use crate::RequestId;
#[cfg(feature = "schema-export")]
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
#[cfg(feature = "schema-export")]
use ts_rs::TS;

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct DeprecationNoticeNotification {
    /// Concise summary of what is deprecated.
    pub summary: String,
    /// Optional extra guidance, such as migration steps or rationale.
    pub details: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct WarningNotification {
    /// Optional thread target when the warning applies to a specific thread.
    pub thread_id: Option<String>,
    /// Concise warning message for the user.
    pub message: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct GuardianWarningNotification {
    /// Thread target for the guardian warning.
    pub thread_id: String,
    /// Concise guardian warning message for the user.
    pub message: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum ClientRelaunchMode {
    Hot,
    Full,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ClientRelaunchRequestedNotification {
    /// Correlation identifier for this lifecycle operation.
    pub request_id: String,
    /// Required refresh mode chosen by the runtime tool caller.
    pub mode: ClientRelaunchMode,
    /// Optional human-readable reason supplied by the runtime tool caller.
    pub reason: Option<String>,
    /// Thread that requested the client relaunch, when the request originated from a thread turn.
    pub requested_by_thread_id: Option<String>,
    /// Reminder that post-relaunch continuation is handled by client bootstrap autoresume.
    pub resume_strategy: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ClientLifecycleRegisterParams {
    /// Stable identifier for the Electron Host instance owning this connection.
    pub host_id: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ClientLifecycleRegisterResponse {
    pub registered: bool,
    pub host_id: String,
    pub reason: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ClientLifecycleRecoveryRecordParams {
    pub target_thread_id: String,
    pub recovery_identity: String,
    pub launcher_claim_id: String,
    pub launcher_evidence_version: String,
    pub transaction_id: String,
    pub request_id: String,
    pub failed_build_id: String,
    pub failed_build_hash: String,
    pub source_commit: String,
    pub requested_by_thread_id: Option<String>,
    pub mode: String,
    pub failure_phase: String,
    pub exit_code: Option<i32>,
    pub signal: Option<String>,
    #[cfg_attr(
        feature = "schema-export",
        ts(optional = nullable, type = "number | null")
    )]
    pub ready_timeout_ms: Option<i64>,
    pub log_path: Option<String>,
    pub transaction_path: Option<String>,
    pub recovered_build_id: String,
    pub prompt: String,
    pub evidence_path: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ClientLifecycleRecoveryRecordResponse {
    pub accepted: bool,
    pub duplicate: bool,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ErrorNotification {
    pub error: TurnError,
    // Set to true if the error is transient and the app-server process will automatically retry.
    // If true, this will not interrupt a turn.
    pub will_retry: bool,
    pub thread_id: String,
    pub turn_id: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ServerRequestResolvedNotification {
    pub thread_id: String,
    pub request_id: RequestId,
}

#[cfg(test)]
mod client_lifecycle_recovery_record_tests {
    use super::ClientLifecycleRecoveryRecordParams;

    fn params_json() -> serde_json::Value {
        serde_json::json!({
            "targetThreadId": "00000000-0000-0000-0000-000000000001",
            "recoveryIdentity": "11111111-1111-4111-8111-111111111111",
            "launcherClaimId": "22222222-2222-4222-8222-222222222222",
            "launcherEvidenceVersion": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "transactionId": "tx-1",
            "requestId": "req-1",
            "failedBuildId": "failed",
            "failedBuildHash": "hash",
            "sourceCommit": "commit",
            "mode": "full",
            "failurePhase": "supervisor",
            "exitCode": null,
            "signal": null,
            "readyTimeoutMs": null,
            "logPath": null,
            "transactionPath": null,
            "recoveredBuildId": "recovered",
            "prompt": "inspect",
            "evidencePath": "/tmp/evidence.json"
        })
    }

    #[test]
    fn optional_requester_accepts_missing_null_and_string() {
        let missing: ClientLifecycleRecoveryRecordParams =
            serde_json::from_value(params_json()).expect("missing requester");
        assert_eq!(missing.requested_by_thread_id, None);

        let mut null = params_json();
        null["requestedByThreadId"] = serde_json::Value::Null;
        let null: ClientLifecycleRecoveryRecordParams =
            serde_json::from_value(null).expect("null requester");
        assert_eq!(null.requested_by_thread_id, None);

        let mut present = params_json();
        present["requestedByThreadId"] =
            serde_json::Value::String("00000000-0000-0000-0000-000000000002".into());
        let present: ClientLifecycleRecoveryRecordParams =
            serde_json::from_value(present).expect("present requester");
        assert_eq!(
            present.requested_by_thread_id.as_deref(),
            Some("00000000-0000-0000-0000-000000000002")
        );
        assert_eq!(
            serde_json::to_value(present).expect("serialize present requester")
                ["requestedByThreadId"],
            "00000000-0000-0000-0000-000000000002"
        );
    }

    #[test]
    fn recovery_identity_is_required() {
        let mut missing = params_json();
        missing
            .as_object_mut()
            .expect("params object")
            .remove("recoveryIdentity");
        assert!(
            serde_json::from_value::<ClientLifecycleRecoveryRecordParams>(missing)
                .is_err()
        );
    }

    #[test]
    fn launcher_claim_binding_is_required() {
        for field in ["launcherClaimId", "launcherEvidenceVersion"] {
            let mut missing = params_json();
            missing
                .as_object_mut()
                .expect("params object")
                .remove(field);
            assert!(
                serde_json::from_value::<ClientLifecycleRecoveryRecordParams>(missing)
                    .is_err()
            );
        }
    }
}
