use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use crate::error_code::internal_error;
use crate::error_code::invalid_request;
use crate::fuzzy_file_search::FuzzyFileSearchSession;
use crate::fuzzy_file_search::run_fuzzy_file_search;
use crate::fuzzy_file_search::start_fuzzy_file_search_session;
use crate::outgoing_message::OutgoingMessageSender;
use app_server_protocol::FuzzyFileSearchParams;
use app_server_protocol::FuzzyFileSearchResponse;
use app_server_protocol::FuzzyFileSearchSessionStartParams;
use app_server_protocol::FuzzyFileSearchSessionStartResponse;
use app_server_protocol::FuzzyFileSearchSessionStopParams;
use app_server_protocol::FuzzyFileSearchSessionStopResponse;
use app_server_protocol::FuzzyFileSearchSessionUpdateParams;
use app_server_protocol::FuzzyFileSearchSessionUpdateResponse;
use app_server_protocol::JSONRPCErrorError;
use tokio::sync::Mutex;

struct OneShotFuzzyFileSearch {
    query: String,
    roots: Vec<String>,
    cancellation_token: Option<String>,
}

struct PreparedOneShotFuzzyFileSearch {
    query: String,
    roots: Vec<String>,
    cancellation_token: Option<String>,
    cancel_flag: Arc<AtomicBool>,
}

struct FuzzyFileSearchSessionStart {
    session_id: String,
    roots: Vec<String>,
}

struct FuzzyFileSearchSessionUpdate {
    session_id: String,
    query: String,
}

struct FuzzyFileSearchSessionStop {
    session_id: String,
}

#[derive(Clone)]
pub(crate) struct SearchRequestProcessor {
    outgoing: Arc<OutgoingMessageSender>,
    pending_fuzzy_searches: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    fuzzy_search_sessions: Arc<Mutex<HashMap<String, FuzzyFileSearchSession>>>,
}

impl SearchRequestProcessor {
    pub(crate) fn new(outgoing: Arc<OutgoingMessageSender>) -> Self {
        Self {
            outgoing,
            pending_fuzzy_searches: Arc::new(Mutex::new(HashMap::new())),
            fuzzy_search_sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) async fn fuzzy_file_search(
        &self,
        params: FuzzyFileSearchParams,
    ) -> Result<FuzzyFileSearchResponse, JSONRPCErrorError> {
        let search = OneShotFuzzyFileSearch::from(params);
        let prepared = self.prepare_one_shot_fuzzy_file_search(search).await;
        let results = self.run_one_shot_fuzzy_file_search(&prepared).await;
        self.cleanup_one_shot_fuzzy_file_search(prepared).await;

        Ok(FuzzyFileSearchResponse { files: results })
    }

    pub(crate) async fn fuzzy_file_search_session_start_response(
        &self,
        params: FuzzyFileSearchSessionStartParams,
    ) -> Result<FuzzyFileSearchSessionStartResponse, JSONRPCErrorError> {
        let start = FuzzyFileSearchSessionStart::try_from(params)?;
        let session = self.start_fuzzy_file_search_session(&start)?;
        self.fuzzy_search_sessions
            .lock()
            .await
            .insert(start.session_id, session);
        Ok(FuzzyFileSearchSessionStartResponse {})
    }

    pub(crate) async fn fuzzy_file_search_session_update_response(
        &self,
        params: FuzzyFileSearchSessionUpdateParams,
    ) -> Result<FuzzyFileSearchSessionUpdateResponse, JSONRPCErrorError> {
        let update = FuzzyFileSearchSessionUpdate::from(params);
        if !self.apply_fuzzy_file_search_session_update(&update).await {
            return Err(invalid_request(format!(
                "fuzzy file search session not found: {}",
                update.session_id
            )));
        }

        Ok(FuzzyFileSearchSessionUpdateResponse {})
    }

    pub(crate) async fn fuzzy_file_search_session_stop(
        &self,
        params: FuzzyFileSearchSessionStopParams,
    ) -> Result<FuzzyFileSearchSessionStopResponse, JSONRPCErrorError> {
        let stop = FuzzyFileSearchSessionStop::from(params);
        self.stop_fuzzy_file_search_session(stop).await;

        Ok(FuzzyFileSearchSessionStopResponse {})
    }

    async fn prepare_one_shot_fuzzy_file_search(
        &self,
        search: OneShotFuzzyFileSearch,
    ) -> PreparedOneShotFuzzyFileSearch {
        let cancel_flag = match search.cancellation_token.clone() {
            Some(token) => {
                let mut pending_fuzzy_searches = self.pending_fuzzy_searches.lock().await;
                if let Some(existing) = pending_fuzzy_searches.get(&token) {
                    existing.store(true, Ordering::Relaxed);
                }
                let flag = Arc::new(AtomicBool::new(false));
                pending_fuzzy_searches.insert(token, flag.clone());
                flag
            }
            None => Arc::new(AtomicBool::new(false)),
        };

        PreparedOneShotFuzzyFileSearch {
            query: search.query,
            roots: search.roots,
            cancellation_token: search.cancellation_token,
            cancel_flag,
        }
    }

    async fn run_one_shot_fuzzy_file_search(
        &self,
        search: &PreparedOneShotFuzzyFileSearch,
    ) -> Vec<app_server_protocol::FuzzyFileSearchResult> {
        match search.query.as_str() {
            "" => Vec::new(),
            _ => {
                run_fuzzy_file_search(
                    search.query.clone(),
                    search.roots.clone(),
                    search.cancel_flag.clone(),
                )
                .await
            }
        }
    }

    async fn cleanup_one_shot_fuzzy_file_search(&self, search: PreparedOneShotFuzzyFileSearch) {
        if let Some(token) = search.cancellation_token {
            let mut pending_fuzzy_searches = self.pending_fuzzy_searches.lock().await;
            if let Some(current_flag) = pending_fuzzy_searches.get(&token)
                && Arc::ptr_eq(current_flag, &search.cancel_flag)
            {
                pending_fuzzy_searches.remove(&token);
            }
        }
    }

    fn start_fuzzy_file_search_session(
        &self,
        start: &FuzzyFileSearchSessionStart,
    ) -> Result<FuzzyFileSearchSession, JSONRPCErrorError> {
        start_fuzzy_file_search_session(
            start.session_id.clone(),
            start.roots.clone(),
            self.outgoing.clone(),
        )
        .map_err(|err| internal_error(format!("failed to start fuzzy file search session: {err}")))
    }

    async fn apply_fuzzy_file_search_session_update(
        &self,
        update: &FuzzyFileSearchSessionUpdate,
    ) -> bool {
        let sessions = self.fuzzy_search_sessions.lock().await;
        if let Some(session) = sessions.get(&update.session_id) {
            session.update_query(update.query.clone());
            true
        } else {
            false
        }
    }

    async fn stop_fuzzy_file_search_session(&self, stop: FuzzyFileSearchSessionStop) {
        self.fuzzy_search_sessions
            .lock()
            .await
            .remove(&stop.session_id);
    }
}

impl From<FuzzyFileSearchParams> for OneShotFuzzyFileSearch {
    fn from(params: FuzzyFileSearchParams) -> Self {
        let FuzzyFileSearchParams {
            query,
            roots,
            cancellation_token,
        } = params;
        Self {
            query,
            roots,
            cancellation_token,
        }
    }
}

impl TryFrom<FuzzyFileSearchSessionStartParams> for FuzzyFileSearchSessionStart {
    type Error = JSONRPCErrorError;

    fn try_from(params: FuzzyFileSearchSessionStartParams) -> Result<Self, Self::Error> {
        let FuzzyFileSearchSessionStartParams { session_id, roots } = params;
        if session_id.is_empty() {
            return Err(invalid_request("sessionId must not be empty"));
        }
        Ok(Self { session_id, roots })
    }
}

impl From<FuzzyFileSearchSessionUpdateParams> for FuzzyFileSearchSessionUpdate {
    fn from(params: FuzzyFileSearchSessionUpdateParams) -> Self {
        let FuzzyFileSearchSessionUpdateParams { session_id, query } = params;
        Self { session_id, query }
    }
}

impl From<FuzzyFileSearchSessionStopParams> for FuzzyFileSearchSessionStop {
    fn from(params: FuzzyFileSearchSessionStopParams) -> Self {
        let FuzzyFileSearchSessionStopParams { session_id } = params;
        Self { session_id }
    }
}
