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

// -- Pipeline tuning ----------------------------------------------------------

/// Thresholds and tunables for the ingest/maintenance pipelines.
///
/// These were previously scattered as magic numbers across remember.rs and
/// maintain.rs. Each field's default reproduces the prior hardcoded value so
/// callers see no behaviour change unless they explicitly tune it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineTuning {
    /// During remember: cosine threshold above which a new fact is treated as
    /// a duplicate of an existing edge.
    pub duplicate_cosine_threshold: f32,

    /// During remember: cosine threshold above which a new fact's score against
    /// an existing edge counts as the same fact (used for invalidation).
    pub same_fact_cosine_threshold: f32,

    /// During link discovery: cosine threshold above which two unlinked
    /// entities are considered close enough to send to the LLM for relation
    /// inference.
    pub link_discovery_cosine_threshold: f32,

    /// During contradiction scan: confidence threshold above which a
    /// classifier match is accepted as a real contradiction/duplicate.
    pub classification_confidence_threshold: f32,

    /// Confidence discount applied to inferred (LLM-derived) facts vs.
    /// directly stated ones.
    pub inferred_fact_discount: f32,

    /// Maximum size of the link-discovery pair cache before the oldest entries
    /// are pruned.
    pub link_pair_cache_max: usize,

    /// Number of pair cache entries to drop when [`link_pair_cache_max`] is hit.
    pub link_pair_cache_evict: usize,

    /// Consolidator: minimum number of episodic facts about an entity
    /// before consolidation produces a summary fact.
    #[serde(default = "default_consolidation_min_facts")]
    pub consolidation_min_facts: usize,
}

fn default_consolidation_min_facts() -> usize {
    5
}

impl Default for PipelineTuning {
    fn default() -> Self {
        Self {
            duplicate_cosine_threshold: 0.9,
            same_fact_cosine_threshold: 0.85,
            link_discovery_cosine_threshold: 0.85,
            classification_confidence_threshold: 0.85,
            inferred_fact_discount: 0.8,
            link_pair_cache_max: 10_000,
            link_pair_cache_evict: 5_000,
            consolidation_min_facts: 5,
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

/// Explicit user/agent retraction of a fact. Distinct from supersession,
/// which the Dreamer writes append-only. See docs/DREAMS.md.
#[derive(Debug, Deserialize, Serialize)]
pub struct RetractRequest {
    pub edge_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RetractResponse {
    pub edge_id: i64,
    pub reason: Option<String>,
}

/// Convenience: retract an old fact and observe a new one in a single
/// operation.
#[derive(Debug, Deserialize, Serialize)]
pub struct CorrectRequest {
    pub edge_id: i64,
    pub statement: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CorrectResponse {
    pub retracted_edge_id: i64,
    pub reason: Option<String>,
    pub remember: RememberResponse,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- ScoringParams ----

    #[test]
    fn pipeline_tuning_default_matches_prior_hardcoded_values() {
        // Regression: these defaults reproduce the magic numbers that used to
        // be scattered across pipeline/remember.rs and pipeline/maintain.rs.
        let p = PipelineTuning::default();
        assert!((p.duplicate_cosine_threshold - 0.9).abs() < 1e-6);
        assert!((p.same_fact_cosine_threshold - 0.85).abs() < 1e-6);
        assert!((p.link_discovery_cosine_threshold - 0.85).abs() < 1e-6);
        assert!((p.classification_confidence_threshold - 0.85).abs() < 1e-6);
        assert!((p.inferred_fact_discount - 0.8).abs() < 1e-6);
        assert_eq!(p.link_pair_cache_max, 10_000);
        assert_eq!(p.link_pair_cache_evict, 5_000);
    }

    #[test]
    fn pipeline_tuning_round_trip_via_json() {
        let original = PipelineTuning::default();
        let s = serde_json::to_string(&original).unwrap();
        let parsed: PipelineTuning = serde_json::from_str(&s).unwrap();
        assert!((parsed.duplicate_cosine_threshold - original.duplicate_cosine_threshold).abs() < 1e-6);
        assert_eq!(parsed.link_pair_cache_max, original.link_pair_cache_max);
    }

    #[test]
    fn scoring_params_default_weights_sum_to_one() {
        let p = ScoringParams::default();
        let total = p.w_relevance + p.w_confidence + p.w_recency + p.w_salience;
        assert!((total - 1.0).abs() < 1e-6, "weights sum to {}", total);
    }

    #[test]
    fn scoring_params_round_trip() {
        let p = ScoringParams::default();
        let s = serde_json::to_string(&p).unwrap();
        let q: ScoringParams = serde_json::from_str(&s).unwrap();
        assert_eq!(q.w_relevance, p.w_relevance);
        assert_eq!(q.mmr_lambda, p.mmr_lambda);
    }

    // ---- LlmUsage ----

    #[test]
    fn llm_usage_default_is_zero() {
        let u = LlmUsage::default();
        assert_eq!(u.llm_calls, 0);
        assert_eq!(u.embed_calls, 0);
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.output_tokens, 0);
    }

    #[test]
    fn llm_usage_merge_sums_fields() {
        let mut a = LlmUsage {
            llm_calls: 1,
            embed_calls: 2,
            input_tokens: 100,
            output_tokens: 50,
        };
        let b = LlmUsage {
            llm_calls: 3,
            embed_calls: 4,
            input_tokens: 200,
            output_tokens: 75,
        };
        a.merge(&b);
        assert_eq!(a.llm_calls, 4);
        assert_eq!(a.embed_calls, 6);
        assert_eq!(a.input_tokens, 300);
        assert_eq!(a.output_tokens, 125);
    }

    // ---- MemoryTier ----

    #[test]
    fn memory_tier_default_is_working() {
        assert_eq!(MemoryTier::default(), MemoryTier::Working);
    }

    #[test]
    fn memory_tier_display_uses_snake_case() {
        assert_eq!(MemoryTier::Working.to_string(), "working");
        assert_eq!(MemoryTier::LongTerm.to_string(), "long_term");
    }

    // ---- GraphOp ----

    #[test]
    fn graph_op_create_node_serialises_with_op_tag_and_type_alias() {
        let op = GraphOp::CreateNode {
            node_ref: Some("ref1".into()),
            name: "Alice".into(),
            node_type: "person".into(),
            properties: HashMap::new(),
        };
        let v: serde_json::Value = serde_json::to_value(&op).unwrap();
        assert_eq!(v["op"], "create_node");
        assert_eq!(v["ref"], "ref1");
        assert_eq!(v["type"], "person");
        assert_eq!(v["name"], "Alice");
    }

    #[test]
    fn graph_op_create_edge_default_confidence_when_missing() {
        let raw = json!({
            "op": "create_edge",
            "from": "a",
            "to": "b",
            "relation": "WORKS_AT",
            "fact": "A works at B"
        });
        let op: GraphOp = serde_json::from_value(raw).unwrap();
        match op {
            GraphOp::CreateEdge { confidence, .. } => assert!((confidence - 0.9).abs() < 1e-6),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn graph_op_invalidate_edge_accepts_either_id_or_fact() {
        let by_id: GraphOp = serde_json::from_value(json!({
            "op": "invalidate_edge",
            "edge_id": 7,
            "reason": "superseded"
        }))
        .unwrap();
        match by_id {
            GraphOp::InvalidateEdge {
                edge_id, fact, ..
            } => {
                assert_eq!(edge_id, Some(7));
                assert_eq!(fact, None);
            }
            _ => panic!(),
        }

        let by_fact: GraphOp = serde_json::from_value(json!({
            "op": "invalidate_edge",
            "fact": "A works at B",
            "reason": "superseded"
        }))
        .unwrap();
        match by_fact {
            GraphOp::InvalidateEdge {
                edge_id, fact, ..
            } => {
                assert_eq!(edge_id, None);
                assert_eq!(fact.as_deref(), Some("A works at B"));
            }
            _ => panic!(),
        }
    }

    // ---- RememberRequest ----

    #[test]
    fn remember_request_optional_fields_default_to_none() {
        let req: RememberRequest =
            serde_json::from_str(r#"{"statement":"hello"}"#).unwrap();
        assert_eq!(req.statement, "hello");
        assert!(req.source_agent.is_none());
        assert!(req.graph.is_none());
        assert!(req.ttl_secs.is_none());
    }

    #[test]
    fn remember_request_omits_none_fields_on_serialise() {
        let req = RememberRequest {
            statement: "x".into(),
            source_agent: None,
            source_credibility_hint: None,
            graph: None,
            ttl_secs: None,
        };
        let s = serde_json::to_string(&req).unwrap();
        // Optional None fields should not appear at all.
        assert!(!s.contains("source_agent"));
        assert!(!s.contains("graph"));
        assert!(!s.contains("ttl_secs"));
    }

    #[test]
    fn remember_request_round_trip_preserves_all_fields() {
        let req = RememberRequest {
            statement: "alice".into(),
            source_agent: Some("agent".into()),
            source_credibility_hint: Some(0.7),
            graph: Some("g".into()),
            ttl_secs: Some(300),
        };
        let s = serde_json::to_string(&req).unwrap();
        let parsed: RememberRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.statement, "alice");
        assert_eq!(parsed.source_agent.as_deref(), Some("agent"));
        assert_eq!(parsed.ttl_secs, Some(300));
    }

    // ---- AskRequest ----

    #[test]
    fn ask_request_max_iterations_defaults_to_one() {
        let req: AskRequest = serde_json::from_str(r#"{"question":"why?"}"#).unwrap();
        assert_eq!(req.max_iterations, 1);
        assert!(!req.verbose);
    }

    // ---- ContextRequest ----

    #[test]
    fn context_request_minimal_payload_parses() {
        let req: ContextRequest = serde_json::from_str(r#"{"query":"alice"}"#).unwrap();
        assert_eq!(req.query, "alice");
        assert!(req.limit.is_none());
        assert!(req.max_hops.is_none());
    }

    // ---- BatchRememberRequest ----

    #[test]
    fn batch_remember_parallel_defaults_to_false() {
        let req: BatchRememberRequest =
            serde_json::from_str(r#"{"statements":["a","b"]}"#).unwrap();
        assert_eq!(req.statements.len(), 2);
        assert!(!req.parallel);
    }

    // ---- ErrorResponse / HealthResponse ----

    #[test]
    fn error_response_round_trip() {
        let er = ErrorResponse {
            error: "boom".into(),
        };
        let s = serde_json::to_string(&er).unwrap();
        assert_eq!(s, r#"{"error":"boom"}"#);
        let back: ErrorResponse = serde_json::from_str(&s).unwrap();
        assert_eq!(back.error, "boom");
    }

    #[test]
    fn health_response_round_trip() {
        let h = HealthResponse {
            status: "ok".into(),
            graph: "hippo".into(),
        };
        let v: serde_json::Value = serde_json::to_value(&h).unwrap();
        assert_eq!(v["status"], "ok");
        assert_eq!(v["graph"], "hippo");
    }

    // ---- RetractRequest / CorrectRequest ----

    #[test]
    fn retract_request_round_trip() {
        let req = RetractRequest {
            edge_id: 42,
            reason: Some("extraction error".into()),
            graph: None,
        };
        let s = serde_json::to_string(&req).unwrap();
        let back: RetractRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(back.edge_id, 42);
        assert_eq!(back.reason.as_deref(), Some("extraction error"));
        assert!(back.graph.is_none());
    }

    #[test]
    fn retract_request_omits_optionals_when_none() {
        let req = RetractRequest {
            edge_id: 7,
            reason: None,
            graph: None,
        };
        let v: serde_json::Value = serde_json::to_value(&req).unwrap();
        assert_eq!(v["edge_id"], 7);
        assert!(v.get("reason").is_none(), "reason should be omitted");
        assert!(v.get("graph").is_none(), "graph should be omitted");
    }

    #[test]
    fn correct_request_round_trip() {
        let req = CorrectRequest {
            edge_id: 99,
            statement: "Alice is a dentist".into(),
            reason: Some("user correction".into()),
            source_agent: Some("user".into()),
            graph: None,
        };
        let s = serde_json::to_string(&req).unwrap();
        let back: CorrectRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(back.edge_id, 99);
        assert_eq!(back.statement, "Alice is a dentist");
        assert_eq!(back.reason.as_deref(), Some("user correction"));
    }
}
