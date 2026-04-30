use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Re-export shared API types so existing `use crate::models::X` continues to work.
pub use hippo_api::{
    ApiKeyInfo, AskRequest, AskResponse, BatchRememberRequest, BatchRememberResponse,
    BatchRememberResult, ContextFact, ContextRequest, ContextResponse, CorrectRequest,
    CorrectResponse, ErrorResponse, GraphOp, HealthResponse, LlmUsage, MemoryTier,
    OpExecutionTrace, OperationsResult, PipelineTuning, RememberRequest, RememberResponse,
    RememberTrace, RetractRequest, RetractResponse, ScoringParams, UserInfo,
};

pub const EMBEDDING_DIM: usize = 768;

/// Pack an `&[f32]` embedding into little-endian bytes for storage in
/// blob columns (used by the SQLite and Postgres backends). The size
/// is always `4 * embedding.len()`.
pub fn serialize_embedding(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Inverse of [`serialize_embedding`]. Trailing bytes that don't form a
/// full f32 are silently dropped.
pub fn deserialize_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[cfg(test)]
mod embedding_tests {
    use super::*;

    #[test]
    fn round_trip_preserves_bits() {
        let v: Vec<f32> = vec![0.0, 1.0, -1.5, 1e-9, f32::MAX, f32::MIN_POSITIVE];
        let bytes = serialize_embedding(&v);
        assert_eq!(bytes.len(), v.len() * 4);
        let back = deserialize_embedding(&bytes);
        assert_eq!(back, v);
    }

    #[test]
    fn empty_embedding_round_trips() {
        let v: Vec<f32> = vec![];
        let back = deserialize_embedding(&serialize_embedding(&v));
        assert!(back.is_empty());
    }

    #[test]
    fn nan_survives_round_trip_via_bit_pattern() {
        let nan = f32::NAN;
        let v = vec![nan];
        let back = deserialize_embedding(&serialize_embedding(&v));
        assert!(back[0].is_nan());
    }

    #[test]
    fn truncated_blob_drops_trailing_bytes() {
        // 9 bytes — two f32s plus a stray byte.
        let bytes = vec![0u8; 9];
        let back = deserialize_embedding(&bytes);
        assert_eq!(back.len(), 2);
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

// HTTP request/response types (server-only)

#[derive(Debug, Deserialize)]
pub struct TemporalContextRequest {
    pub query: String,
    pub at: DateTime<Utc>,
    pub limit: Option<usize>,
    pub graph: Option<String>,
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

// Batch ingest helpers (default functions)

fn default_confidence() -> f32 {
    0.9
}
fn default_source() -> String {
    "seed".to_string()
}
fn default_tier() -> String {
    "long_term".to_string()
}
fn default_true() -> bool {
    true
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
        Self {
            nodes: vec![],
            edges: vec![],
            principal_id: None,
        }
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
