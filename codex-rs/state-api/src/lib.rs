use chrono::DateTime;
use chrono::Utc;
use codex_protocol::ThreadId;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::ThreadGoalUpdateEventAction;
use codex_protocol::models::ThreadGoalUpdateEventSource;
use codex_protocol::models::ThreadGoalUpdateGoal;
use codex_protocol::models::ThreadGoalUpdateGoalStatus;
use codex_protocol::protocol::ThreadGoal as ProtocolThreadGoal;
use codex_protocol::protocol::ThreadGoalStatus as ProtocolThreadGoalStatus;
use codex_protocol::protocol::TokenUsage;
use std::time::Duration;
use std::time::Instant;
use strum::AsRefStr;
use strum::Display;
use strum::EnumString;

mod agent_job;
mod runtime;
mod thread_metadata;

pub use agent_job::AgentJob;
pub use agent_job::AgentJobCreateParams;
pub use agent_job::AgentJobItem;
pub use agent_job::AgentJobItemCreateParams;
pub use agent_job::AgentJobItemStatus;
pub use agent_job::AgentJobProgress;
pub use agent_job::AgentJobStatus;
pub use agent_job::build_agent_job_worker_prompt;
pub use agent_job::default_agent_job_output_csv_path;
pub use agent_job::ensure_unique_agent_job_headers;
pub use agent_job::parse_agent_job_csv;
pub use agent_job::render_agent_job_csv;
pub use agent_job::render_agent_job_instruction_template;
pub use runtime::AgentJobStateRuntime;
pub use runtime::GoalStateRuntime;
pub use runtime::MemoryStateRuntime;
pub use runtime::SharedStateDbRuntime;
pub use runtime::StateApiFuture;
pub use runtime::StateDbRuntime;
pub use runtime::ThreadStateRuntime;
pub use thread_metadata::Anchor;
pub use thread_metadata::BackfillStats;
pub use thread_metadata::ExtractionOutcome;
pub use thread_metadata::SortDirection;
pub use thread_metadata::SortKey;
pub use thread_metadata::ThreadMetadata;
pub use thread_metadata::ThreadMetadataBuilder;
pub use thread_metadata::ThreadsPage;

/// Environment variable for overriding the SQLite state database home directory.
pub const SQLITE_HOME_ENV: &str = "CODEX_SQLITE_HOME";

/// Status attached to a directional thread-spawn edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, Display, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum DirectionalThreadSpawnEdgeStatus {
    Open,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadGoalStatus {
    Active,
    Paused,
    BudgetLimited,
    Complete,
}

impl ThreadGoalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::BudgetLimited => "budget_limited",
            Self::Complete => "complete",
        }
    }

    pub fn is_active(self) -> bool {
        self == Self::Active
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::BudgetLimited | Self::Complete)
    }
}

impl TryFrom<&str> for ThreadGoalStatus {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "active" => Ok(Self::Active),
            "paused" => Ok(Self::Paused),
            "budget_limited" => Ok(Self::BudgetLimited),
            "complete" => Ok(Self::Complete),
            other => Err(anyhow::anyhow!("unknown thread goal status `{other}`")),
        }
    }
}

impl From<ProtocolThreadGoalStatus> for ThreadGoalStatus {
    fn from(value: ProtocolThreadGoalStatus) -> Self {
        match value {
            ProtocolThreadGoalStatus::Active => Self::Active,
            ProtocolThreadGoalStatus::Paused => Self::Paused,
            ProtocolThreadGoalStatus::BudgetLimited => Self::BudgetLimited,
            ProtocolThreadGoalStatus::Complete => Self::Complete,
        }
    }
}

pub fn protocol_goal_status_from_state(status: ThreadGoalStatus) -> ProtocolThreadGoalStatus {
    match status {
        ThreadGoalStatus::Active => ProtocolThreadGoalStatus::Active,
        ThreadGoalStatus::Paused => ProtocolThreadGoalStatus::Paused,
        ThreadGoalStatus::BudgetLimited => ProtocolThreadGoalStatus::BudgetLimited,
        ThreadGoalStatus::Complete => ProtocolThreadGoalStatus::Complete,
    }
}

pub fn state_goal_status_from_protocol(status: ProtocolThreadGoalStatus) -> ThreadGoalStatus {
    status.into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadGoal {
    pub thread_id: ThreadId,
    pub goal_id: String,
    pub objective: String,
    pub status: ThreadGoalStatus,
    pub token_budget: Option<i64>,
    pub tokens_used: i64,
    pub time_used_seconds: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Describes whether an external goal mutation created a new logical goal or
/// updated an existing one.
#[derive(Clone)]
pub enum ExternalGoalPreviousStatus {
    NewGoal,
    Existing(ExternalGoalPreviousGoal),
}

#[derive(Clone)]
pub struct ExternalGoalPreviousGoal {
    pub goal_id: String,
    pub status: ThreadGoalStatus,
    pub objective: String,
}

impl From<&ThreadGoal> for ExternalGoalPreviousStatus {
    fn from(goal: &ThreadGoal) -> Self {
        Self::Existing(ExternalGoalPreviousGoal::from(goal))
    }
}

impl From<&ThreadGoal> for ExternalGoalPreviousGoal {
    fn from(goal: &ThreadGoal) -> Self {
        Self {
            goal_id: goal.goal_id.clone(),
            status: goal.status,
            objective: goal.objective.clone(),
        }
    }
}

/// Runtime effects for an externally persisted goal mutation.
#[derive(Clone)]
pub struct ExternalGoalSet {
    pub goal: ThreadGoal,
    pub previous_status: ExternalGoalPreviousStatus,
}

pub fn protocol_goal_from_state(goal: ThreadGoal) -> ProtocolThreadGoal {
    ProtocolThreadGoal {
        thread_id: goal.thread_id,
        objective: goal.objective,
        status: protocol_goal_status_from_state(goal.status),
        token_budget: goal.token_budget,
        tokens_used: goal.tokens_used,
        time_used_seconds: goal.time_used_seconds,
        created_at: goal.created_at.timestamp(),
        updated_at: goal.updated_at.timestamp(),
    }
}

pub fn validate_thread_goal_budget(value: Option<i64>) -> anyhow::Result<()> {
    if let Some(value) = value
        && value <= 0
    {
        anyhow::bail!("goal budgets must be positive when provided");
    }
    Ok(())
}

pub fn goal_token_delta_for_usage(usage: &TokenUsage) -> i64 {
    usage
        .non_cached_input()
        .saturating_add(usage.output_tokens.max(0))
}

#[derive(Debug)]
pub struct ThreadGoalAccountingSnapshot {
    pub turn: Option<ThreadGoalTurnAccountingSnapshot>,
    pub wall_clock: ThreadGoalWallClockAccountingSnapshot,
}

impl ThreadGoalAccountingSnapshot {
    pub fn new() -> Self {
        Self {
            turn: None,
            wall_clock: ThreadGoalWallClockAccountingSnapshot::new(),
        }
    }
}

impl Default for ThreadGoalAccountingSnapshot {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct ThreadGoalTurnAccountingSnapshot {
    pub turn_id: String,
    last_accounted_token_usage: TokenUsage,
    active_goal_id: Option<String>,
}

impl ThreadGoalTurnAccountingSnapshot {
    pub fn new(turn_id: impl Into<String>, token_usage: TokenUsage) -> Self {
        Self {
            turn_id: turn_id.into(),
            last_accounted_token_usage: token_usage,
            active_goal_id: None,
        }
    }

    pub fn mark_active_goal(&mut self, goal_id: impl Into<String>) {
        self.active_goal_id = Some(goal_id.into());
    }

    pub fn active_this_turn(&self) -> bool {
        self.active_goal_id.is_some()
    }

    pub fn active_goal_id(&self) -> Option<String> {
        self.active_goal_id.clone()
    }

    pub fn clear_active_goal(&mut self) {
        self.active_goal_id = None;
    }

    pub fn reset_baseline(&mut self, token_usage: TokenUsage) {
        self.last_accounted_token_usage = token_usage;
    }

    pub fn token_delta_since_last_accounting(&self, current: &TokenUsage) -> i64 {
        let last = &self.last_accounted_token_usage;
        let delta = TokenUsage {
            input_tokens: current.input_tokens.saturating_sub(last.input_tokens),
            cached_input_tokens: current
                .cached_input_tokens
                .saturating_sub(last.cached_input_tokens),
            output_tokens: current.output_tokens.saturating_sub(last.output_tokens),
            reasoning_output_tokens: current
                .reasoning_output_tokens
                .saturating_sub(last.reasoning_output_tokens),
            total_tokens: current.total_tokens.saturating_sub(last.total_tokens),
        };
        goal_token_delta_for_usage(&delta)
    }

    pub fn mark_accounted(&mut self, current: TokenUsage) {
        self.last_accounted_token_usage = current;
    }
}

#[derive(Debug)]
pub struct ThreadGoalWallClockAccountingSnapshot {
    last_accounted_at: Instant,
    active_goal_id: Option<String>,
}

impl ThreadGoalWallClockAccountingSnapshot {
    pub fn new() -> Self {
        Self {
            last_accounted_at: Instant::now(),
            active_goal_id: None,
        }
    }

    pub fn time_delta_since_last_accounting(&self) -> i64 {
        let last = self.last_accounted_at;
        i64::try_from(last.elapsed().as_secs()).unwrap_or(i64::MAX)
    }

    pub fn mark_accounted(&mut self, accounted_seconds: i64) {
        if accounted_seconds <= 0 {
            return;
        }
        let advance = Duration::from_secs(u64::try_from(accounted_seconds).unwrap_or(u64::MAX));
        self.last_accounted_at = self
            .last_accounted_at
            .checked_add(advance)
            .unwrap_or_else(Instant::now);
    }

    pub fn reset_baseline(&mut self) {
        self.last_accounted_at = Instant::now();
    }

    pub fn mark_active_goal(&mut self, goal_id: impl Into<String>) {
        let goal_id = goal_id.into();
        if self.active_goal_id.as_deref() != Some(goal_id.as_str()) {
            self.reset_baseline();
            self.active_goal_id = Some(goal_id);
        }
    }

    pub fn clear_active_goal(&mut self) {
        self.active_goal_id = None;
        self.reset_baseline();
    }

    pub fn active_goal_id(&self) -> Option<String> {
        self.active_goal_id.clone()
    }
}

impl Default for ThreadGoalWallClockAccountingSnapshot {
    fn default() -> Self {
        Self::new()
    }
}

pub fn thread_goal_update_response_item(
    goal: ProtocolThreadGoal,
    previous_status: Option<ThreadGoalStatus>,
    source: ThreadGoalUpdateEventSource,
) -> ResponseItem {
    ResponseItem::ThreadGoalUpdate {
        id: None,
        action: thread_goal_update_action(&goal, previous_status),
        source,
        previous_status: previous_status.map(thread_goal_update_status_from_state),
        goal: thread_goal_update_goal_from_protocol(goal),
    }
}

fn thread_goal_update_goal_from_protocol(goal: ProtocolThreadGoal) -> ThreadGoalUpdateGoal {
    ThreadGoalUpdateGoal {
        thread_id: goal.thread_id,
        objective: goal.objective,
        status: thread_goal_update_status_from_protocol(goal.status),
        token_budget: goal.token_budget,
        tokens_used: goal.tokens_used,
        time_used_seconds: goal.time_used_seconds,
        created_at: goal.created_at,
        updated_at: goal.updated_at,
    }
}

fn thread_goal_update_status_from_protocol(
    status: ProtocolThreadGoalStatus,
) -> ThreadGoalUpdateGoalStatus {
    match status {
        ProtocolThreadGoalStatus::Active => ThreadGoalUpdateGoalStatus::Active,
        ProtocolThreadGoalStatus::Paused => ThreadGoalUpdateGoalStatus::Paused,
        ProtocolThreadGoalStatus::BudgetLimited => ThreadGoalUpdateGoalStatus::BudgetLimited,
        ProtocolThreadGoalStatus::Complete => ThreadGoalUpdateGoalStatus::Complete,
    }
}

fn thread_goal_update_status_from_state(status: ThreadGoalStatus) -> ThreadGoalUpdateGoalStatus {
    match status {
        ThreadGoalStatus::Active => ThreadGoalUpdateGoalStatus::Active,
        ThreadGoalStatus::Paused => ThreadGoalUpdateGoalStatus::Paused,
        ThreadGoalStatus::BudgetLimited => ThreadGoalUpdateGoalStatus::BudgetLimited,
        ThreadGoalStatus::Complete => ThreadGoalUpdateGoalStatus::Complete,
    }
}

fn thread_goal_update_action(
    goal: &ProtocolThreadGoal,
    previous_status: Option<ThreadGoalStatus>,
) -> ThreadGoalUpdateEventAction {
    match (previous_status, goal.status) {
        (None, _) => ThreadGoalUpdateEventAction::Created,
        (_, ProtocolThreadGoalStatus::Paused) => ThreadGoalUpdateEventAction::Paused,
        (_, ProtocolThreadGoalStatus::BudgetLimited) => ThreadGoalUpdateEventAction::BudgetLimited,
        (_, ProtocolThreadGoalStatus::Complete) => ThreadGoalUpdateEventAction::Completed,
        (Some(status), ProtocolThreadGoalStatus::Active) if status != ThreadGoalStatus::Active => {
            ThreadGoalUpdateEventAction::Resumed
        }
        (Some(_), ProtocolThreadGoalStatus::Active) => ThreadGoalUpdateEventAction::Updated,
    }
}

pub struct ThreadGoalUpdate {
    pub objective: Option<String>,
    pub status: Option<ThreadGoalStatus>,
    pub token_budget: Option<Option<i64>>,
    pub expected_goal_id: Option<String>,
}

pub enum ThreadGoalAccountingOutcome {
    Unchanged(Option<ThreadGoal>),
    Updated(ThreadGoal),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreadGoalAccountingMode {
    ActiveStatusOnly,
    ActiveOnly,
    ActiveOrComplete,
    ActiveOrStopped,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use codex_protocol::models::ThreadGoalUpdateEventSource;

    #[test]
    fn protocol_goal_conversion_preserves_state_fields() {
        let thread_id = ThreadId::new();
        let goal = ThreadGoal {
            thread_id,
            goal_id: "goal-1".to_string(),
            objective: "finish topology refactor".to_string(),
            status: ThreadGoalStatus::BudgetLimited,
            token_budget: Some(10_000),
            tokens_used: 10_001,
            time_used_seconds: 42,
            created_at: Utc.timestamp_opt(11, 0).unwrap(),
            updated_at: Utc.timestamp_opt(22, 0).unwrap(),
        };

        let protocol_goal = protocol_goal_from_state(goal);

        assert_eq!(protocol_goal.thread_id, thread_id);
        assert_eq!(protocol_goal.objective, "finish topology refactor");
        assert_eq!(
            protocol_goal.status,
            ProtocolThreadGoalStatus::BudgetLimited
        );
        assert_eq!(protocol_goal.token_budget, Some(10_000));
        assert_eq!(protocol_goal.tokens_used, 10_001);
        assert_eq!(protocol_goal.time_used_seconds, 42);
        assert_eq!(protocol_goal.created_at, 11);
        assert_eq!(protocol_goal.updated_at, 22);
    }

    #[test]
    fn goal_token_delta_excludes_cached_input_and_does_not_double_count_reasoning() {
        let usage = TokenUsage {
            input_tokens: 900,
            cached_input_tokens: 400,
            output_tokens: 80,
            reasoning_output_tokens: 20,
            total_tokens: 1_000,
        };

        assert_eq!(580, goal_token_delta_for_usage(&usage));
    }

    #[test]
    fn wall_clock_accounting_advances_by_persisted_seconds() {
        let mut snapshot = ThreadGoalWallClockAccountingSnapshot::new();
        let original = Instant::now() - Duration::from_millis(1500);
        snapshot.last_accounted_at = original;

        snapshot.mark_accounted(/*accounted_seconds*/ 1);
        assert_eq!(
            original + Duration::from_secs(1),
            snapshot.last_accounted_at
        );

        let token_only_original = snapshot.last_accounted_at;
        snapshot.mark_accounted(/*accounted_seconds*/ 0);
        assert_eq!(token_only_original, snapshot.last_accounted_at);
    }

    #[test]
    fn thread_goal_update_item_derives_lifecycle_action() {
        let item = thread_goal_update_response_item(
            ProtocolThreadGoal {
                thread_id: ThreadId::new(),
                objective: "resume work".to_string(),
                status: ProtocolThreadGoalStatus::Active,
                token_budget: None,
                tokens_used: 12,
                time_used_seconds: 3,
                created_at: 1,
                updated_at: 2,
            },
            Some(ThreadGoalStatus::Paused),
            ThreadGoalUpdateEventSource::ModelTool,
        );

        let ResponseItem::ThreadGoalUpdate {
            action,
            previous_status,
            source,
            goal,
            ..
        } = item
        else {
            panic!("expected thread goal update item");
        };

        assert_eq!(action, ThreadGoalUpdateEventAction::Resumed);
        assert_eq!(source, ThreadGoalUpdateEventSource::ModelTool);
        assert_eq!(previous_status, Some(ThreadGoalUpdateGoalStatus::Paused));
        assert_eq!(goal.objective, "resume work");
        assert_eq!(goal.status, ThreadGoalUpdateGoalStatus::Active);
    }

    #[test]
    fn goal_budget_must_be_positive_when_present() {
        assert!(validate_thread_goal_budget(None).is_ok());
        assert!(validate_thread_goal_budget(Some(1)).is_ok());
        assert!(validate_thread_goal_budget(Some(0)).is_err());
        assert!(validate_thread_goal_budget(Some(-1)).is_err());
    }
}
