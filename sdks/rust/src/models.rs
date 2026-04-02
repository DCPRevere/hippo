pub use hippo_api::{
    // Request types
    AskRequest, BatchRememberRequest, ContextRequest, RememberRequest,
    // Response types
    AskResponse, BatchRememberResponse, BatchRememberResult, ContextFact, ContextResponse,
    ErrorResponse, HealthResponse, LlmUsage, RememberResponse, RememberTrace,
    OpExecutionTrace, GraphOp,
    // Scoring
    ScoringParams,
    // Enums
    MemoryTier,
    // Admin
    ApiKeyInfo, UserInfo,
};

use serde::{Deserialize, Serialize};

// SDK-only types (not in the server API crate)

#[derive(Debug, Clone, Serialize)]
pub struct CreateUserRequest {
    pub user_id: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graphs: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateKeyRequest {
    pub label: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateUserResponse {
    pub user_id: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListUsersResponse {
    pub users: Vec<UserInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateKeyResponse {
    pub user_id: String,
    pub label: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListKeysResponse {
    pub keys: Vec<ApiKeyInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GraphEvent {
    pub event: String,
    pub data: serde_json::Value,
}
