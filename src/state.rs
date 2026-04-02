use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

#[cfg(not(target_arch = "wasm32"))]
use crate::audit::AuditLog;
use crate::auth::UserStore;
use crate::config::Config;
use crate::credibility::CredibilityRegistry;
use crate::error::AppError;
use crate::events::GraphEvent;
use crate::graph::GraphRegistry;
use crate::graph_backend::GraphBackend;
use crate::llm_service::LlmService;

pub struct AppState {
    pub graphs: Option<GraphRegistry>,
    pub llm: Arc<dyn LlmService>,
    pub config: Config,
    pub recent_nodes_tx: tokio::sync::mpsc::Sender<String>,
    pub recent_nodes_rx: Arc<Mutex<tokio::sync::mpsc::Receiver<String>>>,
    /// Snapshot of recently drained node IDs, refreshed each maintenance cycle.
    pub recent_node_ids: Arc<RwLock<Vec<String>>>,
    pub checked_pairs: Arc<RwLock<HashSet<(String, String)>>>,
    pub metrics: Arc<MetricsState>,
    pub credibility: Arc<RwLock<CredibilityRegistry>>,
    /// Broadcast channel for real-time graph mutation events (SSE).
    pub event_tx: tokio::sync::broadcast::Sender<GraphEvent>,
    /// User store for API key authentication. None means auth is disabled.
    pub user_store: Option<Arc<dyn UserStore>>,
    /// Audit log for recording security and mutation events.
    #[cfg(not(target_arch = "wasm32"))]
    pub audit: Option<Arc<AuditLog>>,
    /// Per-user rate limiter. None when rate limiting is disabled.
    #[cfg(not(target_arch = "wasm32"))]
    pub rate_limiter: Option<crate::rate_limit::RateLimiter>,
}

impl AppState {
    /// Access the graph registry. Panics in test configurations where no registry is set.
    pub fn graph_registry(&self) -> &GraphRegistry {
        self.graphs.as_ref().expect("GraphRegistry not available (test mode)")
    }

    /// Construct an `AppState` for unit tests (no FalkorDB connection needed).
    pub fn for_test(llm: Arc<dyn LlmService>, config: Config) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(200);
        let (event_tx, _) = tokio::sync::broadcast::channel::<GraphEvent>(256);
        Self {
            graphs: None,
            llm,
            config,
            recent_nodes_tx: tx,
            recent_nodes_rx: Arc::new(Mutex::new(rx)),
            recent_node_ids: Arc::new(RwLock::new(Vec::new())),
            checked_pairs: Arc::new(RwLock::new(HashSet::new())),
            metrics: Arc::new(MetricsState::new()),
            credibility: Arc::new(RwLock::new(CredibilityRegistry::new())),
            event_tx,
            user_store: None,
            #[cfg(not(target_arch = "wasm32"))]
            audit: None,
            #[cfg(not(target_arch = "wasm32"))]
            rate_limiter: None,
        }
    }

    /// Record an audit event. No-op if audit is disabled.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn emit_audit(
        &self,
        user_id: impl Into<String>,
        action: impl Into<String>,
        details: impl Into<String>,
    ) {
        if let Some(ref audit) = self.audit {
            audit.log(crate::audit::AuditEntry {
                user_id: user_id.into(),
                action: action.into(),
                details: details.into(),
            });
        }
    }

    /// Resolve a graph by name and check that the user has access.
    pub async fn resolve_graph_for_user(
        &self,
        graph_name: Option<&str>,
        user: &crate::auth::AuthenticatedUser,
    ) -> Result<Arc<dyn GraphBackend>, AppError> {
        let graph = self.graph_registry().resolve(graph_name).await;
        let name = graph.graph_name();

        // Reserved system namespace — only admins can access hippo-* and admin-* graphs
        if crate::auth::is_system_graph(name) && !user.is_admin() {
            return Err(AppError::forbidden("system graphs are not accessible"));
        }

        if !user.can_access_graph(name) {
            return Err(AppError::forbidden(format!(
                "user '{}' does not have access to graph '{}'",
                user.user_id,
                name
            )));
        }
        Ok(graph)
    }
}

pub struct MetricsState {
    pub remember_calls_total: AtomicU64,
    pub remember_facts_written: AtomicU64,
    pub remember_contradictions: AtomicU64,
    pub context_calls_total: AtomicU64,
    pub reflect_calls_total: AtomicU64,
    pub entity_count: AtomicU64,
    pub fact_count: AtomicU64,
}

impl MetricsState {
    pub fn new() -> Self {
        Self {
            remember_calls_total: AtomicU64::new(0),
            remember_facts_written: AtomicU64::new(0),
            remember_contradictions: AtomicU64::new(0),
            context_calls_total: AtomicU64::new(0),
            reflect_calls_total: AtomicU64::new(0),
            entity_count: AtomicU64::new(0),
            fact_count: AtomicU64::new(0),
        }
    }

    pub fn reset(&self) {
        self.remember_calls_total.store(0, Ordering::Relaxed);
        self.remember_facts_written.store(0, Ordering::Relaxed);
        self.remember_contradictions.store(0, Ordering::Relaxed);
        self.context_calls_total.store(0, Ordering::Relaxed);
        self.reflect_calls_total.store(0, Ordering::Relaxed);
        self.entity_count.store(0, Ordering::Relaxed);
        self.fact_count.store(0, Ordering::Relaxed);
    }

    pub fn to_prometheus(&self) -> String {
        let mut buf = String::new();

        let counters = [
            ("hippo_remember_calls_total", "Total number of /remember calls", &self.remember_calls_total),
            ("hippo_facts_written_total", "Total facts written to graph", &self.remember_facts_written),
            ("hippo_contradictions_total", "Total contradictions detected", &self.remember_contradictions),
            ("hippo_context_calls_total", "Total /context queries", &self.context_calls_total),
            ("hippo_reflect_calls_total", "Total /reflect queries", &self.reflect_calls_total),
        ];

        for (name, help, counter) in &counters {
            buf.push_str(&format!("# HELP {name} {help}\n"));
            buf.push_str(&format!("# TYPE {name} counter\n"));
            buf.push_str(&format!("{name} {}\n\n", counter.load(Ordering::Relaxed)));
        }

        let gauges = [
            ("hippo_entity_count", "Current entity count (from last maintenance)", &self.entity_count),
            ("hippo_fact_count", "Current active fact count", &self.fact_count),
        ];

        for (name, help, gauge) in &gauges {
            buf.push_str(&format!("# HELP {name} {help}\n"));
            buf.push_str(&format!("# TYPE {name} gauge\n"));
            buf.push_str(&format!("{name} {}\n\n", gauge.load(Ordering::Relaxed)));
        }

        buf
    }
}
