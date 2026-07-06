use crate::protocol::common::AuthMode;
use codex_experimental_api_macros::ExperimentalApi;
use protocol::account::PlanType;
use protocol::account::ProviderAccount;
use protocol::protocol::CreditsSnapshot as CoreCreditsSnapshot;
use protocol::protocol::RateLimitReachedType as CoreRateLimitReachedType;
use protocol::protocol::RateLimitSnapshot as CoreRateLimitSnapshot;
use protocol::protocol::RateLimitWindow as CoreRateLimitWindow;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use ts_rs::TS;

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(tag = "type"))]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum Account {
    #[serde(rename = "apiKey", rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename = "apiKey", rename_all = "camelCase"))]
    ApiKey {},

    #[serde(rename = "chatgpt", rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename = "chatgpt", rename_all = "camelCase"))]
    Chatgpt { email: String, plan_type: PlanType },

    #[serde(rename = "amazonBedrock", rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename = "amazonBedrock", rename_all = "camelCase"))]
    AmazonBedrock {},
}

impl From<ProviderAccount> for Account {
    fn from(account: ProviderAccount) -> Self {
        match account {
            ProviderAccount::ApiKey => Self::ApiKey {},
            ProviderAccount::Chatgpt { email, plan_type } => Self::Chatgpt { email, plan_type },
            ProviderAccount::AmazonBedrock => Self::AmazonBedrock {},
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ExperimentalApi)]
#[serde(tag = "type")]
#[cfg_attr(feature = "schema-export", ts(tag = "type"))]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum LoginAccountParams {
    #[serde(rename = "apiKey", rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename = "apiKey", rename_all = "camelCase"))]
    ApiKey {
        #[serde(rename = "apiKey")]
        #[cfg_attr(feature = "schema-export", ts(rename = "apiKey"))]
        api_key: String,
    },
    #[serde(rename = "chatgpt", rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename = "chatgpt", rename_all = "camelCase"))]
    Chatgpt {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        codex_streamlined_login: bool,
    },
    #[serde(rename = "chatgptDeviceCode")]
    #[cfg_attr(feature = "schema-export", ts(rename = "chatgptDeviceCode"))]
    ChatgptDeviceCode,
    /// [UNSTABLE] FOR OPENAI INTERNAL USE ONLY - DO NOT USE.
    /// The access token must contain the same scopes that Codex-managed ChatGPT auth tokens have.
    #[experimental("account/login/start.chatgptAuthTokens")]
    #[serde(rename = "chatgptAuthTokens", rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename = "chatgptAuthTokens", rename_all = "camelCase"))]
    ChatgptAuthTokens {
        /// Access token (JWT) supplied by the client.
        /// This token is used for backend API requests and email extraction.
        access_token: String,
        /// Workspace/account identifier supplied by the client.
        chatgpt_account_id: String,
        /// Optional plan type supplied by the client.
        ///
        /// When `null`, Codex attempts to derive the plan type from access-token
        /// claims. If unavailable, the plan defaults to `unknown`.
        #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
        chatgpt_plan_type: Option<String>,
    },
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(tag = "type"))]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum LoginAccountResponse {
    #[serde(rename = "apiKey", rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename = "apiKey", rename_all = "camelCase"))]
    ApiKey {},
    #[serde(rename = "chatgpt", rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename = "chatgpt", rename_all = "camelCase"))]
    Chatgpt {
        // Use plain String for identifiers to avoid TS/JSON Schema quirks around uuid-specific types.
        // Convert to/from UUIDs at the application layer as needed.
        login_id: String,
        /// URL the client should open in a browser to initiate the OAuth flow.
        auth_url: String,
    },
    #[serde(rename = "chatgptDeviceCode", rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename = "chatgptDeviceCode", rename_all = "camelCase"))]
    ChatgptDeviceCode {
        // Use plain String for identifiers to avoid TS/JSON Schema quirks around uuid-specific types.
        // Convert to/from UUIDs at the application layer as needed.
        login_id: String,
        /// URL the client should open in a browser to complete device code authorization.
        verification_url: String,
        /// One-time code the user must enter after signing in.
        user_code: String,
    },
    #[serde(rename = "chatgptAuthTokens", rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename = "chatgptAuthTokens", rename_all = "camelCase"))]
    ChatgptAuthTokens {},
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CancelLoginAccountParams {
    pub login_id: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum CancelLoginAccountStatus {
    Canceled,
    NotFound,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CancelLoginAccountResponse {
    pub status: CancelLoginAccountStatus,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct LogoutAccountResponse {}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum ChatgptAuthTokensRefreshReason {
    /// Codex attempted a backend request and received `401 Unauthorized`.
    Unauthorized,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ChatgptAuthTokensRefreshParams {
    pub reason: ChatgptAuthTokensRefreshReason,
    /// Workspace/account identifier that Codex was previously using.
    ///
    /// Clients that manage multiple accounts/workspaces can use this as a hint
    /// to refresh the token for the correct workspace.
    ///
    /// This may be `null` when the prior auth state did not include a workspace
    /// identifier (`chatgpt_account_id`).
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub previous_account_id: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ChatgptAuthTokensRefreshResponse {
    pub access_token: String,
    pub chatgpt_account_id: String,
    pub chatgpt_plan_type: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct GetAccountRateLimitsResponse {
    /// Backward-compatible single-bucket view; mirrors the historical payload.
    pub rate_limits: RateLimitSnapshot,
    /// Multi-bucket view keyed by metered `limit_id` (for example, `codex`).
    pub rate_limits_by_limit_id: Option<HashMap<String, RateLimitSnapshot>>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct SendAddCreditsNudgeEmailParams {
    pub credit_type: AddCreditsNudgeCreditType,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "snake_case"))]
pub enum AddCreditsNudgeCreditType {
    Credits,
    UsageLimit,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct SendAddCreditsNudgeEmailResponse {
    pub status: AddCreditsNudgeEmailStatus,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "snake_case"))]
pub enum AddCreditsNudgeEmailStatus {
    Sent,
    CooldownActive,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct GetAccountParams {
    /// When `true`, requests a proactive token refresh before returning.
    ///
    /// In managed auth mode this triggers the normal refresh-token flow. In
    /// external auth mode this flag is ignored. Clients should refresh tokens
    /// themselves and call `account/login/start` with `chatgptAuthTokens`.
    #[serde(default)]
    pub refresh_token: bool,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct GetAccountResponse {
    pub account: Option<Account>,
    pub requires_openai_auth: bool,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct AccountUpdatedNotification {
    pub auth_mode: Option<AuthMode>,
    pub plan_type: Option<PlanType>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct AccountRateLimitsUpdatedNotification {
    pub rate_limits: RateLimitSnapshot,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct RateLimitSnapshot {
    pub limit_id: Option<String>,
    pub limit_name: Option<String>,
    pub primary: Option<RateLimitWindow>,
    pub secondary: Option<RateLimitWindow>,
    pub credits: Option<CreditsSnapshot>,
    pub plan_type: Option<PlanType>,
    pub rate_limit_reached_type: Option<RateLimitReachedType>,
}

impl From<CoreRateLimitSnapshot> for RateLimitSnapshot {
    fn from(value: CoreRateLimitSnapshot) -> Self {
        Self {
            limit_id: value.limit_id,
            limit_name: value.limit_name,
            primary: value.primary.map(RateLimitWindow::from),
            secondary: value.secondary.map(RateLimitWindow::from),
            credits: value.credits.map(CreditsSnapshot::from),
            plan_type: value.plan_type,
            rate_limit_reached_type: value
                .rate_limit_reached_type
                .map(RateLimitReachedType::from),
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "snake_case"))]
pub enum RateLimitReachedType {
    RateLimitReached,
    WorkspaceOwnerCreditsDepleted,
    WorkspaceMemberCreditsDepleted,
    WorkspaceOwnerUsageLimitReached,
    WorkspaceMemberUsageLimitReached,
}

impl From<CoreRateLimitReachedType> for RateLimitReachedType {
    fn from(value: CoreRateLimitReachedType) -> Self {
        match value {
            CoreRateLimitReachedType::RateLimitReached => Self::RateLimitReached,
            CoreRateLimitReachedType::WorkspaceOwnerCreditsDepleted => {
                Self::WorkspaceOwnerCreditsDepleted
            }
            CoreRateLimitReachedType::WorkspaceMemberCreditsDepleted => {
                Self::WorkspaceMemberCreditsDepleted
            }
            CoreRateLimitReachedType::WorkspaceOwnerUsageLimitReached => {
                Self::WorkspaceOwnerUsageLimitReached
            }
            CoreRateLimitReachedType::WorkspaceMemberUsageLimitReached => {
                Self::WorkspaceMemberUsageLimitReached
            }
        }
    }
}

impl From<RateLimitReachedType> for CoreRateLimitReachedType {
    fn from(value: RateLimitReachedType) -> Self {
        match value {
            RateLimitReachedType::RateLimitReached => Self::RateLimitReached,
            RateLimitReachedType::WorkspaceOwnerCreditsDepleted => {
                Self::WorkspaceOwnerCreditsDepleted
            }
            RateLimitReachedType::WorkspaceMemberCreditsDepleted => {
                Self::WorkspaceMemberCreditsDepleted
            }
            RateLimitReachedType::WorkspaceOwnerUsageLimitReached => {
                Self::WorkspaceOwnerUsageLimitReached
            }
            RateLimitReachedType::WorkspaceMemberUsageLimitReached => {
                Self::WorkspaceMemberUsageLimitReached
            }
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct RateLimitWindow {
    pub used_percent: i32,
    #[cfg_attr(feature = "schema-export", ts(type = "number | null"))]
    pub window_duration_mins: Option<i64>,
    #[cfg_attr(feature = "schema-export", ts(type = "number | null"))]
    pub resets_at: Option<i64>,
}

impl From<CoreRateLimitWindow> for RateLimitWindow {
    fn from(value: CoreRateLimitWindow) -> Self {
        Self {
            used_percent: value.used_percent.round() as i32,
            window_duration_mins: value.window_minutes,
            resets_at: value.resets_at,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CreditsSnapshot {
    pub has_credits: bool,
    pub unlimited: bool,
    pub balance: Option<String>,
}

impl From<CoreCreditsSnapshot> for CreditsSnapshot {
    fn from(value: CoreCreditsSnapshot) -> Self {
        Self {
            has_credits: value.has_credits,
            unlimited: value.unlimited,
            balance: value.balance,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct AccountLoginCompletedNotification {
    // Use plain String for identifiers to avoid TS/JSON Schema quirks around uuid-specific types.
    // Convert to/from UUIDs at the application layer as needed.
    pub login_id: Option<String>,
    pub success: bool,
    pub error: Option<String>,
}
