use super::ThreadHistoryBuilder;
use crate::protocol::CollabAgentState;
use crate::protocol::CollabAgentTool;
use crate::protocol::CollabAgentToolCallStatus;
use crate::protocol::ThreadLifecycleFinalStatus;
use crate::protocol::ThreadLifecycleStatus;
use crate::protocol::ThreadItem;
use protocol::protocol::AgentStatus;
use std::collections::HashMap;

impl ThreadHistoryBuilder {
    pub(super) fn handle_collab_agent_spawn_begin(
        &mut self,
        payload: &protocol::protocol::CollabAgentSpawnBeginEvent,
    ) {
        let item = ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::SpawnAgent,
            status: CollabAgentToolCallStatus::InProgress,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: Vec::new(),
            receiver_paths: Vec::new(),
            timeout_ms: None,
            prompt: Some(payload.prompt.clone()),
            model: Some(payload.model.clone()),
            reasoning_effort: Some(payload.reasoning_effort),
            agents_states: HashMap::new(),
        };
        self.upsert_item_in_current_turn(item);
    }

    pub(super) fn handle_collab_agent_spawn_end(
        &mut self,
        payload: &protocol::protocol::CollabAgentSpawnEndEvent,
    ) {
        let has_receiver = payload.new_thread_id.is_some();
        let status = match &payload.status {
            AgentStatus::Errored(_) | AgentStatus::NotFound => CollabAgentToolCallStatus::Failed,
            _ if has_receiver => CollabAgentToolCallStatus::Completed,
            _ => CollabAgentToolCallStatus::Failed,
        };
        let (receiver_thread_ids, agents_states) = match &payload.new_thread_id {
            Some(id) => {
                let receiver_id = id.to_string();
                let mut received_status = CollabAgentState::from(payload.status.clone());
                received_status.path = payload.new_agent_path.clone();
                (
                    vec![receiver_id.clone()],
                    [(receiver_id, received_status)].into_iter().collect(),
                )
            }
            None => (Vec::new(), HashMap::new()),
        };
        self.upsert_item_in_current_turn(ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::SpawnAgent,
            status,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids,
            receiver_paths: payload.new_agent_path.clone().into_iter().collect(),
            timeout_ms: None,
            prompt: Some(payload.prompt.clone()),
            model: Some(payload.model.clone()),
            reasoning_effort: Some(payload.reasoning_effort),
            agents_states,
        });
    }

    pub(super) fn handle_collab_agent_interaction_begin(
        &mut self,
        payload: &protocol::protocol::CollabAgentInteractionBeginEvent,
    ) {
        let item = ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::SendInput,
            status: CollabAgentToolCallStatus::InProgress,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: vec![payload.receiver_thread_id.to_string()],
            receiver_paths: vec![payload.receiver_agent_path.clone()],
            timeout_ms: None,
            prompt: Some(payload.prompt.clone()),
            model: None,
            reasoning_effort: None,
            agents_states: HashMap::new(),
        };
        self.upsert_item_in_current_turn(item);
    }

    pub(super) fn handle_collab_agent_interaction_end(
        &mut self,
        payload: &protocol::protocol::CollabAgentInteractionEndEvent,
    ) {
        let status = match &payload.status {
            AgentStatus::Errored(_) | AgentStatus::NotFound => CollabAgentToolCallStatus::Failed,
            _ => CollabAgentToolCallStatus::Completed,
        };
        let receiver_id = payload.receiver_thread_id.to_string();
        let mut received_status = CollabAgentState::from(payload.status.clone());
        received_status.path = Some(payload.receiver_agent_path.clone());
        self.upsert_item_in_current_turn(ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::SendInput,
            status,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: vec![receiver_id.clone()],
            receiver_paths: vec![payload.receiver_agent_path.clone()],
            timeout_ms: None,
            prompt: Some(payload.prompt.clone()),
            model: None,
            reasoning_effort: None,
            agents_states: [(receiver_id, received_status)].into_iter().collect(),
        });
    }

    pub(super) fn handle_collab_list_agents_begin(
        &mut self,
        payload: &protocol::protocol::CollabListAgentsBeginEvent,
    ) {
        let item = ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::ListAgents,
            status: CollabAgentToolCallStatus::InProgress,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: Vec::new(),
            receiver_paths: Vec::new(),
            timeout_ms: None,
            prompt: payload.path_prefix.clone(),
            model: None,
            reasoning_effort: None,
            agents_states: HashMap::new(),
        };
        self.upsert_item_in_current_turn(item);
    }

    pub(super) fn handle_collab_list_agents_end(
        &mut self,
        payload: &protocol::protocol::CollabListAgentsEndEvent,
    ) {
        let receiver_paths: Vec<String> = payload
            .agents
            .iter()
            .map(|agent| agent.agent_path.clone())
            .collect();
        let agents_states = payload
            .agents
            .iter()
            .map(|agent| {
                let mut state = CollabAgentState::from(agent.lifecycle_status.clone());
                state.path = Some(agent.agent_path.clone());
                if state.message.is_none() {
                    state.message = agent.last_task_message.clone();
                }
                (agent.agent_path.clone(), state)
            })
            .collect();
        self.upsert_item_in_current_turn(ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::ListAgents,
            status: if payload.success {
                CollabAgentToolCallStatus::Completed
            } else {
                CollabAgentToolCallStatus::Failed
            },
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: Vec::new(),
            receiver_paths,
            timeout_ms: None,
            prompt: payload.path_prefix.clone(),
            model: None,
            reasoning_effort: None,
            agents_states,
        });
    }

    pub(super) fn handle_collab_waiting_begin(
        &mut self,
        payload: &protocol::protocol::CollabWaitingBeginEvent,
    ) {
        let item = ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::Wait,
            status: CollabAgentToolCallStatus::InProgress,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: payload
                .receiver_thread_ids
                .iter()
                .map(ToString::to_string)
                .collect(),
            receiver_paths: payload
                .receiver_agents
                .iter()
                .filter_map(|agent| agent.agent_path.clone())
                .collect(),
            timeout_ms: Some(payload.timeout_ms),
            prompt: None,
            model: None,
            reasoning_effort: None,
            agents_states: HashMap::new(),
        };
        self.upsert_item_in_current_turn(item);
    }

    pub(super) fn handle_collab_waiting_end(
        &mut self,
        payload: &protocol::protocol::CollabWaitingEndEvent,
    ) {
        let status = if payload
            .lifecycle_statuses
            .values()
            .any(|status| {
                matches!(
                    status,
                    ThreadLifecycleStatus::Final {
                        result: ThreadLifecycleFinalStatus::Errored { .. }
                    } | ThreadLifecycleStatus::NotLoaded
                        | ThreadLifecycleStatus::SystemError { .. }
                )
            })
        {
            CollabAgentToolCallStatus::Failed
        } else {
            CollabAgentToolCallStatus::Completed
        };
        let mut receiver_thread_ids: Vec<String> =
            payload.lifecycle_statuses.keys().map(ToString::to_string).collect();
        receiver_thread_ids.sort();
        let agents_states = payload
            .lifecycle_statuses
            .iter()
            .map(|(id, status)| {
                let mut state = CollabAgentState::from(status.clone());
                state.path = payload
                    .agent_lifecycles
                    .iter()
                    .find(|entry| entry.thread_id == *id)
                    .and_then(|entry| entry.agent_path.clone());
                (id.to_string(), state)
            })
            .collect();
        self.upsert_item_in_current_turn(ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::Wait,
            status,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids,
            receiver_paths: payload
                .agent_lifecycles
                .iter()
                .filter_map(|entry| entry.agent_path.clone())
                .collect(),
            timeout_ms: Some(payload.timeout_ms),
            prompt: None,
            model: None,
            reasoning_effort: None,
            agents_states,
        });
    }

    pub(super) fn handle_collab_close_begin(
        &mut self,
        payload: &protocol::protocol::CollabCloseBeginEvent,
    ) {
        let item = ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::CloseAgent,
            status: CollabAgentToolCallStatus::InProgress,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: vec![payload.receiver_thread_id.to_string()],
            receiver_paths: vec![payload.receiver_agent_path.clone()],
            timeout_ms: None,
            prompt: None,
            model: None,
            reasoning_effort: None,
            agents_states: HashMap::new(),
        };
        self.upsert_item_in_current_turn(item);
    }

    pub(super) fn handle_collab_close_end(
        &mut self,
        payload: &protocol::protocol::CollabCloseEndEvent,
    ) {
        let status = match &payload.status {
            AgentStatus::Errored(_) | AgentStatus::NotFound => CollabAgentToolCallStatus::Failed,
            _ => CollabAgentToolCallStatus::Completed,
        };
        let receiver_id = payload.receiver_thread_id.to_string();
        let mut state = CollabAgentState::from(payload.status.clone());
        state.path = Some(payload.receiver_agent_path.clone());
        let agents_states = [(receiver_id.clone(), state)].into_iter().collect();
        self.upsert_item_in_current_turn(ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::CloseAgent,
            status,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: vec![receiver_id],
            receiver_paths: vec![payload.receiver_agent_path.clone()],
            timeout_ms: None,
            prompt: None,
            model: None,
            reasoning_effort: None,
            agents_states,
        });
    }

    pub(super) fn handle_collab_resume_begin(
        &mut self,
        payload: &protocol::protocol::CollabResumeBeginEvent,
    ) {
        let item = ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::ResumeAgent,
            status: CollabAgentToolCallStatus::InProgress,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: vec![payload.receiver_thread_id.to_string()],
            receiver_paths: vec![payload.receiver_agent_path.clone()],
            timeout_ms: None,
            prompt: None,
            model: None,
            reasoning_effort: None,
            agents_states: HashMap::new(),
        };
        self.upsert_item_in_current_turn(item);
    }

    pub(super) fn handle_collab_resume_end(
        &mut self,
        payload: &protocol::protocol::CollabResumeEndEvent,
    ) {
        let status = match &payload.status {
            AgentStatus::Errored(_) | AgentStatus::NotFound => CollabAgentToolCallStatus::Failed,
            _ => CollabAgentToolCallStatus::Completed,
        };
        let receiver_id = payload.receiver_thread_id.to_string();
        let mut state = CollabAgentState::from(payload.status.clone());
        state.path = Some(payload.receiver_agent_path.clone());
        let agents_states = [(receiver_id.clone(), state)].into_iter().collect();
        self.upsert_item_in_current_turn(ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::ResumeAgent,
            status,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: vec![receiver_id],
            receiver_paths: vec![payload.receiver_agent_path.clone()],
            timeout_ms: None,
            prompt: None,
            model: None,
            reasoning_effort: None,
            agents_states,
        });
    }
}
