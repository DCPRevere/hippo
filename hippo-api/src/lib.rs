use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// -- Scoring ------------------------------------------------------------------

/// Weights for the context retrieval scoring formula and MMR diversity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringParams {
    pub w_relevance: f32,
    pub w_confidence: f32,
    pub w_recency: f32,
    pub w_salience: f32,
    pub mmr_lambda: f32,
}

impl Default for ScoringParams {
    fn default() -> Self {
        Self {
            w_relevance: 0.50,
            w_confidence: 0.10,
            w_recency: 0.25,
            w_salience: 0.15,
            mmr_lambda: 0.70,
        }
    }
}

// -- LLM usage ----------------------------------------------------------------

/// Tracks token and call counts for LLM/embedding operations within a pipeline run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmUsage {
    pub llm_calls: u32,
    pub embed_calls: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl LlmUsage {
    pub fn merge(&mut self, other: &LlmUsage) {
        self.llm_calls += other.llm_calls;
        self.embed_calls += other.embed_calls;
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
    }
}

// -- Memory tier --------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MemoryTier {
    Working,
    LongTerm,
}

impl Default for MemoryTier {
    fn default() -> Self {
        MemoryTier::Working
    }
}

impl std::fmt::Display for MemoryTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryTier::Working => write!(f, "working"),
            MemoryTier::LongTerm => write!(f, "long_term"),
        }
    }
}

// -- Graph operations (returned in traces) ------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum GraphOp {
    CreateNode {
        #[serde(rename = "ref", default)]
        node_ref: Option<String>,
        name: String,
        #[serde(rename = "type")]
        node_type: String,
        #[serde(default)]
        properties: HashMap<String, String>,
    },
    UpdateNode {
        id: String,
        #[serde(default)]
        set: HashMap<String, String>,
    },
    CreateEdge {
        from: String,
        to: String,
        relation: String,
        fact: String,
        #[serde(default = "default_op_confidence")]
        confidence: f32,
    },
    InvalidateEdge {
        #[serde(default)]
        edge_id: Option<i64>,
        #[serde(default)]
        fact: Option<String>,
        reason: String,
    },
}

fn default_op_confidence() -> f32 {
    0.9
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationsResult {
    pub operations: Vec<GraphOp>,
}

// -- Request types ------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
pub struct RememberRequest {
    pub statement: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_credibility_hint: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BatchRememberRequest {
    pub statements: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_agent: Option<String>,
    #[serde(default)]
    pub parallel: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ContextRequest {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hops: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_tier_filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scoring: Option<ScoringParams>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AskRequest {
    pub question: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
    #[serde(default)]
    pub verbose: bool,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
}

fn default_max_iterations() -> usize {
    1
}

// -- Response types -----------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RememberResponse {
    pub entities_created: usize,
    pub entities_resolved: usize,
    pub facts_written: usize,
    pub contradictions_invalidated: usize,
    pub usage: LlmUsage,
    pub trace: RememberTrace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RememberTrace {
    pub operations: Vec<GraphOp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revised_operations: Option<Vec<GraphOp>>,
    pub execution: Vec<OpExecutionTrace>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpExecutionTrace {
    pub op: String,
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextFact {
    pub fact: String,
    pub subject: String,
    pub relation_type: String,
    pub object: String,
    pub confidence: f32,
    pub salience: i64,
    pub valid_at: DateTime<Utc>,
    pub edge_id: i64,
    pub hops: usize,
    pub source_agents: Vec<String>,
    pub memory_tier: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContextResponse {
    pub facts: Vec<ContextFact>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AskResponse {
    pub answer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facts: Option<Vec<ContextFact>>,
    pub iterations: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub graph: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

// -- Batch types --------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct BatchRememberResponse {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub results: Vec<BatchRememberResult>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BatchRememberResult {
    pub statement: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facts_written: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entities_created: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// -- Admin types --------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub user_id: String,
    pub display_name: String,
    pub role: String,
    pub graphs: Vec<String>,
    pub key_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyInfo {
    pub label: String,
    pub created_at: String,
}
