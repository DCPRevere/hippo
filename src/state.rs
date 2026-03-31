use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

use crate::config::Config;
use crate::credibility::CredibilityRegistry;
use crate::graph::GraphRegistry;
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
}

impl AppState {
    /// Access the graph registry. Panics in test configurations where no registry is set.
    pub fn graph_registry(&self) -> &GraphRegistry {
        self.graphs.as_ref().expect("GraphRegistry not available (test mode)")
    }

    /// Construct an `AppState` for unit tests (no FalkorDB connection needed).
    pub fn for_test(llm: Arc<dyn LlmService>, config: Config) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(200);
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
        }
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
