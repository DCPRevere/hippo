use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Hippo API request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct RememberRequest {
    pub statement: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RememberResponse {
    pub entities_created: usize,
    pub entities_resolved: usize,
    pub facts_written: usize,
    pub contradictions_invalidated: usize,
}

#[derive(Debug, Serialize)]
pub struct ContextRequest {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ContextResponse {
    pub facts: Vec<ContextFact>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContextFact {
    pub fact: String,
    pub subject: String,
    pub relation_type: String,
    pub object: String,
    pub confidence: f32,
    pub salience: i64,
    pub valid_at: String,
    pub edge_id: i64,
    pub hops: usize,
    pub source_agents: Vec<String>,
    pub memory_tier: String,
}

#[derive(Debug, Serialize)]
pub struct AskApiRequest {
    pub question: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AskApiResponse {
    pub answer: String,
    pub facts: Option<Vec<ContextFact>>,
}

#[derive(Debug, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub graph: String,
}

#[derive(Debug, Deserialize)]
pub struct GraphStats {
    pub entity_count: usize,
    pub edge_count: usize,
}
