use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const EMBEDDING_DIM: usize = 768;

/// Weights for the context retrieval scoring formula and MMR diversity.
///
/// The final score for each fact is:
///   relevance × w_relevance + confidence × w_confidence + recency × w_recency + salience × w_salience
///
/// After scoring, MMR reranking uses `mmr_lambda` to balance relevance vs diversity.
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

/// Tracks token and call counts for LLM/embedding operations within a pipeline run.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LlmUsage {
    pub llm_calls: u32,
    pub embed_calls: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl LlmUsage {
    /// Accumulate counts from another usage record into this one.
    pub fn merge(&mut self, other: &LlmUsage) {
        self.llm_calls += other.llm_calls;
        self.embed_calls += other.embed_calls;
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MemoryTier {
    Working,   // recent, unverified, decays in ~24h if not promoted
    LongTerm,  // confirmed, stable
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub name: String,
    pub entity_type: String,
    pub resolved: bool,
    pub hint: Option<String>,
    pub content: Option<String>,
    pub created_at: DateTime<Utc>,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    pub fact: String,
    pub relation_type: String,
    pub embedding: Vec<f32>,
    pub source_agents: Vec<String>,
    pub valid_at: DateTime<Utc>,
    pub invalid_at: Option<DateTime<Utc>>,
    pub confidence: f32,
    pub salience: i64,
    pub created_at: DateTime<Utc>,
    pub memory_tier: MemoryTier,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedEntity {
    pub name: String,
    pub entity_type: String,
    pub resolved: bool,
    pub hint: Option<String>,
    pub content: Option<String>,
}

// Operations-based extraction types (LLM returns graph mutations)

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum GraphOp {
    CreateNode {
        /// Short reference for use in later operations (e.g. "n1").
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
        /// Edge id from the subgraph.
        #[serde(default)]
        edge_id: Option<i64>,
        /// Fact text for fallback matching if edge_id not provided.
        #[serde(default)]
        fact: Option<String>,
        reason: String,
    },
}

fn default_op_confidence() -> f32 { 0.9 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationsResult {
    pub operations: Vec<GraphOp>,
}

#[derive(Debug, Clone)]
pub struct ResolvedEntity {
    pub id: String,
    pub name: String,
    pub is_new: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EdgeClassification {
    Duplicate,
    Contradiction,
    Related,
    Unrelated,
}

// HTTP request/response types

#[derive(Debug, Deserialize)]
pub struct RememberRequest {
    pub statement: String,
    pub source_agent: Option<String>,
    pub source_credibility_hint: Option<f32>,
    pub graph: Option<String>,
    /// Optional TTL in seconds. Overrides the global DEFAULT_TTL_SECS if set.
    pub ttl_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RememberResponse {
    pub entities_created: usize,
    pub entities_resolved: usize,
    pub facts_written: usize,
    pub contradictions_invalidated: usize,
    pub usage: LlmUsage,
    pub trace: RememberTrace,
}

#[derive(Debug, Clone, Serialize)]
pub struct RememberTrace {
    pub operations: Vec<GraphOp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revised_operations: Option<Vec<GraphOp>>,
    pub execution: Vec<OpExecutionTrace>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpExecutionTrace {
    pub op: String,
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ContextRequest {
    pub query: String,
    pub limit: Option<usize>,
    #[serde(default)]
    pub max_hops: Option<usize>,
    #[serde(default)]
    pub memory_tier_filter: Option<String>,
    pub graph: Option<String>,
    #[serde(default)]
    pub at: Option<DateTime<Utc>>,
    /// Override scoring weights and MMR lambda for this request.
    #[serde(default)]
    pub scoring: Option<ScoringParams>,
}

#[derive(Debug, Deserialize)]
pub struct TemporalContextRequest {
    pub query: String,
    pub at: DateTime<Utc>,
    pub limit: Option<usize>,
    pub graph: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ContextResponse {
    pub facts: Vec<ContextFact>,
}

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub graph: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// Supersession / provenance types

#[derive(Debug, Clone, Serialize)]
pub struct SupersessionRecord {
    pub old_edge_id: i64,
    pub new_edge_id: i64,
    pub superseded_at: DateTime<Utc>,
    pub old_fact: String,
    pub new_fact: String,
}

#[derive(Debug, Serialize)]
pub struct ProvenanceResponse {
    pub edge_id: i64,
    pub superseded_by: Option<SupersessionRecord>,
    pub supersedes: Vec<SupersessionRecord>,
}

// Diagnostics

#[derive(Debug, Serialize)]
pub struct GraphEntity {
    pub name: String,
    pub entity_type: String,
    pub resolved: bool,
}

#[derive(Debug, Serialize)]
pub struct GraphEdge {
    pub subject: String,
    pub relation_type: String,
    pub object: String,
    pub fact: String,
    pub salience: i64,
    pub confidence: f32,
    pub valid_at: String,
    pub invalid_at: Option<String>,
}

// Reflect types

#[derive(Debug, Serialize)]
pub struct MemoryStats {
    pub total_entities: usize,
    pub total_facts: usize,
    pub oldest_fact: Option<DateTime<Utc>>,
    pub newest_fact: Option<DateTime<Utc>>,
    pub avg_confidence: f32,
    pub entities_by_type: HashMap<String, usize>,
}

// Memory tier stats

#[derive(Debug, Clone, Serialize)]
pub struct MemoryTierStats {
    pub working_count: usize,
    pub long_term_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphStats {
    pub entity_count: usize,
    pub edge_count: usize,
    pub oldest_valid_at: Option<String>,
    pub newest_valid_at: Option<String>,
    pub avg_confidence: f32,
}

// Streaming progress events for /remember/stream

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RememberProgress {
    ContextGathered {
        entities_found: usize,
        edges_found: usize,
    },
    Planning,
    Planned {
        operations: usize,
    },
    Revising {
        new_context_entities: usize,
    },
    Executing {
        op: String,
    },
    Complete(RememberResponse),
    Error(String),
}

// Batch ingest types

#[derive(Debug, Deserialize)]
pub struct BatchRememberRequest {
    pub statements: Vec<String>,
    pub source_agent: Option<String>,
    #[serde(default)]
    pub parallel: bool,
    pub graph: Option<String>,
    pub ttl_secs: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct BatchRememberResponse {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub results: Vec<BatchRememberResult>,
}

#[derive(Debug, Serialize)]
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

// Consolidation types

#[derive(Debug, Serialize)]
pub struct ConsolidateReport {
    pub new_links: Vec<NewLinkReport>,
    pub pruned_facts: Vec<PrunedFactReport>,
    pub clusters: Vec<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct NewLinkReport {
    pub entity_a: String,
    pub entity_b: String,
    pub relation: String,
    pub confidence: f32,
}

#[derive(Debug, Serialize)]
pub struct PrunedFactReport {
    pub fact: String,
    pub reason: String,
}

// Smart query types

// Ask types (NL question → NL answer)

#[derive(Debug, Deserialize)]
pub struct AskRequest {
    pub question: String,
    pub limit: Option<usize>,
    pub graph: Option<String>,
    #[serde(default)]
    pub verbose: bool,
}

#[derive(Debug, Serialize)]
pub struct AskResponse {
    pub answer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facts: Option<Vec<ContextFact>>,
}

// Admin seed types (for direct graph seeding in tests)

#[derive(Debug, Deserialize)]
pub struct AdminSeedRequest {
    pub entities: Vec<AdminSeedEntity>,
    pub edges: Vec<AdminSeedEdge>,
    pub graph: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AdminSeedEntity {
    pub id: String,
    pub name: String,
    pub entity_type: String,
    #[serde(default = "default_true")]
    pub resolved: bool,
    pub hint: Option<String>,
}

fn default_true() -> bool { true }

#[derive(Debug, Serialize, Deserialize)]
pub struct AdminSeedEdge {
    pub subject_id: String,
    pub object_id: String,
    pub fact: String,
    pub relation_type: String,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub salience: i64,
    pub valid_at: Option<String>,
    #[serde(default = "default_source")]
    pub source_agents: String,
    #[serde(default = "default_tier")]
    pub memory_tier: String,
}

fn default_confidence() -> f32 { 0.9 }
fn default_source() -> String { "seed".to_string() }
fn default_tier() -> String { "long_term".to_string() }

#[derive(Debug, Serialize)]
pub struct AdminSeedResponse {
    pub entities_created: usize,
    pub edges_created: usize,
}

// Backup / restore types

#[derive(Debug, Deserialize)]
pub struct BackupRequest {
    pub graph: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupPayload {
    pub graph: String,
    pub exported_at: String,
    pub entities: Vec<BackupEntity>,
    pub edges: Vec<AdminSeedEdge>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupEntity {
    pub id: String,
    pub name: String,
    pub entity_type: String,
    #[serde(default = "default_true")]
    pub resolved: bool,
    pub hint: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RestoreRequest {
    pub graph: String,
    pub exported_at: String,
    pub entities: Vec<BackupEntity>,
    pub edges: Vec<AdminSeedEdge>,
    pub target_graph: Option<String>,
}

// FalkorDB row types for parsing results

#[derive(Debug, Clone, Serialize)]
pub struct EntityRow {
    pub id: String,
    pub name: String,
    pub entity_type: String,
    pub resolved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    pub created_at: String,
    #[serde(skip)]
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EdgeRow {
    pub edge_id: i64,
    pub subject_id: String,
    pub subject_name: String,
    pub fact: String,
    pub relation_type: String,
    pub confidence: f32,
    pub salience: i64,
    pub valid_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid_at: Option<String>,
    pub object_id: String,
    pub object_name: String,
    #[serde(skip)]
    pub embedding: Vec<f32>,
    pub decayed_confidence: f32,
    pub source_agents: String,
    pub memory_tier: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

// ---- Subgraph context types (shared between LLM and pipeline) ----

/// A node in the subgraph sent to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgraphNode {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub node_type: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub properties: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

/// An edge in the subgraph sent to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubgraphEdge {
    pub id: i64,
    pub from: String,
    pub to: String,
    pub relation: String,
    pub fact: String,
    pub confidence: f32,
}

/// Rich subgraph context for the LLM, serialised as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphContext {
    pub nodes: Vec<SubgraphNode>,
    pub edges: Vec<SubgraphEdge>,
    /// The entity ID of the principal, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
}

impl GraphContext {
    pub fn empty() -> Self {
        Self { nodes: vec![], edges: vec![], principal_id: None }
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.edges.is_empty()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Serialise as JSON for embedding in the LLM prompt.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}
