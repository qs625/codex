use crate::workflows::WorkflowRegistry;
use crate::workflows::WorkflowSummary;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkflowRunStatus {
    Running,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkflowRun {
    pub(crate) run_id: String,
    pub(crate) workflow: WorkflowSummary,
    pub(crate) status: WorkflowRunStatus,
    pub(crate) runner_status: String,
    pub(crate) inputs: Value,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) revision: u64,
    pub(crate) message: String,
    pub(crate) abort_reason: Option<String>,
}

#[derive(Default)]
pub(crate) struct WorkflowRunManager {
    next_id: AtomicU64,
    runs: Mutex<BTreeMap<String, WorkflowRun>>,
}

impl WorkflowRunManager {
    pub(crate) async fn start(
        &self,
        registry: &WorkflowRegistry,
        workflow_id: &str,
        inputs: Value,
    ) -> Result<WorkflowRun, String> {
        let workflow = registry
            .find(workflow_id)
            .ok_or_else(|| format!("unknown workflow `{workflow_id}`"))?
            .clone();
        let sequence = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let now = unix_timestamp_seconds();
        let run = WorkflowRun {
            run_id: format!("wf_{sequence}"),
            workflow,
            status: WorkflowRunStatus::Running,
            runner_status: "control_plane_started".to_string(),
            inputs,
            created_at: now,
            updated_at: now,
            revision: 1,
            message: "workflow control run started; TypeScript runner execution is pending"
                .to_string(),
            abort_reason: None,
        };
        self.runs
            .lock()
            .await
            .insert(run.run_id.clone(), run.clone());
        Ok(run)
    }

    pub(crate) async fn status(&self, run_id: &str) -> Result<WorkflowRun, String> {
        self.runs
            .lock()
            .await
            .get(run_id)
            .cloned()
            .ok_or_else(|| format!("unknown workflow run `{run_id}`"))
    }

    pub(crate) async fn resume(
        &self,
        run_id: &str,
        inputs: Option<Value>,
    ) -> Result<WorkflowRun, String> {
        let mut runs = self.runs.lock().await;
        let run = runs
            .get_mut(run_id)
            .ok_or_else(|| format!("unknown workflow run `{run_id}`"))?;
        if run.status == WorkflowRunStatus::Aborted {
            return Err(format!("workflow run `{run_id}` is aborted"));
        }
        if let Some(inputs) = inputs {
            run.inputs = inputs;
        }
        run.revision += 1;
        run.updated_at = unix_timestamp_seconds();
        run.runner_status = "control_plane_resumed".to_string();
        run.message =
            "workflow control run resumed; TypeScript runner execution is pending".to_string();
        Ok(run.clone())
    }

    pub(crate) async fn abort(
        &self,
        run_id: &str,
        reason: Option<String>,
    ) -> Result<WorkflowRun, String> {
        let mut runs = self.runs.lock().await;
        let run = runs
            .get_mut(run_id)
            .ok_or_else(|| format!("unknown workflow run `{run_id}`"))?;
        run.status = WorkflowRunStatus::Aborted;
        run.revision += 1;
        run.updated_at = unix_timestamp_seconds();
        run.runner_status = "aborted".to_string();
        run.message = "workflow control run aborted".to_string();
        run.abort_reason = reason;
        Ok(run.clone())
    }
}

fn unix_timestamp_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflows::WorkflowInputSpec;
    use crate::workflows::WorkflowSource;
    use pretty_assertions::assert_eq;

    #[tokio::test]
    async fn workflow_run_manager_controls_run_lifecycle() {
        let registry = WorkflowRegistry {
            workflows: vec![WorkflowSummary {
                id: "feature-dev".to_string(),
                name: "feature-dev".to_string(),
                description: "project description".to_string(),
                source: WorkflowSource::Project,
                path: "/repo/.codex/workflows/feature-dev".to_string(),
                entry: "workflow.ts".to_string(),
                version: Some("0.1.0".to_string()),
                when_to_use: Vec::new(),
                inputs: std::collections::BTreeMap::from([(
                    "objective".to_string(),
                    WorkflowInputSpec {
                        input_type: "string".to_string(),
                        description: Some("goal".to_string()),
                    },
                )]),
            }],
            diagnostics: Vec::new(),
        };
        let manager = WorkflowRunManager::default();

        let started = manager
            .start(
                &registry,
                "feature-dev",
                serde_json::json!({"objective": "ship"}),
            )
            .await
            .expect("start workflow run");
        assert_eq!(started.workflow.id, "feature-dev");
        assert_eq!(started.status, WorkflowRunStatus::Running);
        assert_eq!(started.revision, 1);

        let status = manager.status(&started.run_id).await.expect("run status");
        assert_eq!(status, started);

        let resumed = manager
            .resume(
                &started.run_id,
                Some(serde_json::json!({"objective": "resume"})),
            )
            .await
            .expect("resume workflow run");
        assert_eq!(resumed.status, WorkflowRunStatus::Running);
        assert_eq!(resumed.revision, 2);
        assert_eq!(resumed.inputs, serde_json::json!({"objective": "resume"}));

        let aborted = manager
            .abort(&started.run_id, Some("not needed".to_string()))
            .await
            .expect("abort workflow run");
        assert_eq!(aborted.status, WorkflowRunStatus::Aborted);
        assert_eq!(aborted.abort_reason.as_deref(), Some("not needed"));

        let err = manager
            .resume(&started.run_id, /*inputs*/ None)
            .await
            .expect_err("aborted run cannot resume");
        assert_eq!(err, format!("workflow run `{}` is aborted", started.run_id));
    }
}
