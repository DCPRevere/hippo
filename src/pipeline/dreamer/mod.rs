//! The Dreamer — hippo's background processing primitive.
//!
//! Architecture from `docs/DREAMS.md`. A Dreamer is a process that runs
//! between conversations, querying the graph for its next unit of work and
//! taking append-only actions on it (linking, inferring, supersession,
//! consolidation). Multiple workers run the same Dreamer in parallel.
//!
//! There is one `Dreamer` trait — concrete implementations specialise the
//! action (link, infer, reconcile, consolidate). The actions themselves
//! decide what to do for an entity by querying the graph; the pool just
//! drives them.
//!
//! Idempotency comes from the graph: each Dreamer's `next_unit` consults
//! `last_visited` (or its own state) to skip recently-processed entities.
//! The pool does not maintain a separate work queue.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::graph_backend::GraphBackend;

pub mod actions;
pub use actions::{Consolidator, Inferrer, Linker, Reconciler};

/// One unit of work for a Dreamer — typically a single entity. The score is
/// available for selection logic (best-first vs weighted-random) but the
/// pool itself doesn't interpret it.
#[derive(Debug, Clone)]
pub struct WorkUnit {
    pub entity_id: String,
    pub score: f32,
}

/// Aggregated summary of a dream pass. Each `process` call returns a
/// per-unit report; the pool sums them.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DreamReport {
    /// Number of entities the Dreamer visited.
    pub facts_visited: usize,
    /// Links the Dreamer wrote between previously-unconnected entities.
    pub links_written: usize,
    /// Inferences the Dreamer wrote (implied facts from existing structure).
    pub inferences_written: usize,
    /// Supersession facts the Dreamer wrote (append-only contradiction
    /// resolution).
    pub supersessions_written: usize,
    /// Contradictions the Dreamer noticed but did not act on (low confidence).
    pub contradictions_seen: usize,
    /// Consolidations: episodic facts collapsed into a higher-order pattern.
    pub consolidations_written: usize,
    /// Total LLM tokens consumed during the pass.
    pub tokens_used: u64,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
}

impl DreamReport {
    pub fn merge(&mut self, other: &DreamReport) {
        self.facts_visited += other.facts_visited;
        self.links_written += other.links_written;
        self.inferences_written += other.inferences_written;
        self.supersessions_written += other.supersessions_written;
        self.contradictions_seen += other.contradictions_seen;
        self.consolidations_written += other.consolidations_written;
        self.tokens_used += other.tokens_used;
        self.duration_ms = self.duration_ms.max(other.duration_ms);
    }
}

/// The Dreamer trait. One implementation per action category.
///
/// Implementors return their next unit of work via `next_unit` (querying
/// the graph for whatever they care about) and execute that work in
/// `process`. The pool drives the loop and aggregates reports.
#[async_trait]
pub trait Dreamer: Send + Sync {
    /// Stable name used in logs and dream-reports.
    fn name(&self) -> &str;

    /// Return the next unit of work, or `None` when the Dreamer believes it
    /// has no useful work right now. The pool calls this in a loop until
    /// `None` is returned (or budget exhausts).
    async fn next_unit(&self, graph: &dyn GraphBackend) -> Result<Option<WorkUnit>>;

    /// Process one unit of work. Append-only by convention: the Dreamer
    /// writes new facts, never deletes or modifies existing ones. Returns
    /// a per-unit report that the pool aggregates.
    async fn process(&self, graph: &dyn GraphBackend, unit: WorkUnit) -> Result<DreamReport>;
}

/// Default `next_unit` strategy: scan the most recent entities and return
/// the first one without a `last_visited` marker. The pool's claim
/// handshake sets `last_visited` atomically so the next caller sees it as
/// visited. Three of the four built-in Dreamers (Linker, Reconciler,
/// Consolidator) use this as-is; Inferrer also uses it.
pub async fn next_unvisited_entity(
    graph: &dyn GraphBackend,
    scan_window: usize,
) -> Result<Option<WorkUnit>> {
    let entities = graph.list_entities_by_recency(0, scan_window).await?;
    for e in entities {
        if graph.last_visited(&e.id).await?.is_none() {
            return Ok(Some(WorkUnit {
                entity_id: e.id,
                score: 0.0,
            }));
        }
    }
    Ok(None)
}

/// Runtime configuration for a dream pass. Bounded by default so a manual
/// `dream()` invocation can't burn an unbounded LLM bill.
#[derive(Debug, Clone)]
pub struct DreamerConfig {
    /// Number of concurrent workers. Each pulls work via `next_unit`.
    pub worker_count: usize,
    /// Hard ceiling on units processed in this pass. `None` = unlimited (use
    /// for continuous mode with cadence-bounded passes).
    pub max_units: Option<usize>,
    /// Hard ceiling on total tokens consumed in this pass. `None` = no
    /// budget.
    pub max_tokens: Option<u64>,
}

impl DreamerConfig {
    pub fn bounded(worker_count: usize, max_units: Option<usize>, max_tokens: Option<u64>) -> Self {
        Self {
            worker_count: worker_count.max(1),
            max_units,
            max_tokens,
        }
    }

    pub fn unbounded(worker_count: usize) -> Self {
        Self::bounded(worker_count, None, None)
    }
}

impl Default for DreamerConfig {
    fn default() -> Self {
        Self::bounded(1, Some(100), Some(50_000))
    }
}

/// A pool of workers running one Dreamer in parallel.
pub struct WorkerPool {
    config: DreamerConfig,
}

impl WorkerPool {
    pub fn new(config: DreamerConfig) -> Self {
        Self { config }
    }

    /// Run the given Dreamer until it returns no more units, or the pass
    /// budget is exhausted. Returns the aggregated report.
    ///
    /// Concurrency model: the `next_unit` → claim handshake is serialised
    /// per-pool so two workers can't both observe the same entity as
    /// un-visited and process it twice. `process` itself runs concurrently
    /// across workers. Each unit is "claimed" by calling `mark_visited`
    /// before release; if the Dreamer relies on a different idempotency
    /// signal (e.g. its own internal state), `mark_visited` is harmless
    /// extra metadata.
    pub async fn run_dream(
        &self,
        dreamer: Arc<dyn Dreamer>,
        graph: Arc<dyn GraphBackend>,
    ) -> Result<DreamReport> {
        let start = std::time::Instant::now();
        let report = Arc::new(tokio::sync::Mutex::new(DreamReport::default()));
        let unit_count = Arc::new(AtomicUsize::new(0));
        let token_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        // Serialises the "find next unit + mark it claimed" handshake. Held
        // only across the call to `next_unit` and the immediate
        // `mark_visited`; `process` runs unlocked.
        let claim_lock = Arc::new(tokio::sync::Mutex::new(()));

        let mut handles = Vec::with_capacity(self.config.worker_count);
        for _ in 0..self.config.worker_count {
            let dreamer = dreamer.clone();
            let graph = graph.clone();
            let report = report.clone();
            let unit_count = unit_count.clone();
            let token_count = token_count.clone();
            let claim_lock = claim_lock.clone();
            let max_units = self.config.max_units;
            let max_tokens = self.config.max_tokens;

            let handle = tokio::spawn(async move {
                loop {
                    if let Some(limit) = max_units {
                        if unit_count.load(Ordering::Relaxed) >= limit {
                            return Ok::<(), anyhow::Error>(());
                        }
                    }
                    if let Some(limit) = max_tokens {
                        if token_count.load(Ordering::Relaxed) >= limit {
                            return Ok(());
                        }
                    }

                    // Serialised claim handshake.
                    let unit = {
                        let _guard = claim_lock.lock().await;
                        let next = dreamer.next_unit(&*graph).await?;
                        match next {
                            Some(u) => {
                                let claimed = unit_count.fetch_add(1, Ordering::Relaxed) + 1;
                                if let Some(limit) = max_units {
                                    if claimed > limit {
                                        return Ok(());
                                    }
                                }
                                graph.mark_visited(&u.entity_id, chrono::Utc::now()).await?;
                                u
                            }
                            None => return Ok(()),
                        }
                    };

                    let per_unit = dreamer.process(&*graph, unit).await?;
                    token_count.fetch_add(per_unit.tokens_used, Ordering::Relaxed);

                    let mut total = report.lock().await;
                    total.merge(&per_unit);
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            match handle.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    tracing::error!(dreamer = dreamer.name(), error = %e, "worker errored");
                    return Err(e);
                }
                Err(e) => {
                    tracing::error!(dreamer = dreamer.name(), error = %e, "worker panicked");
                    return Err(anyhow::anyhow!("worker panicked: {e}"));
                }
            }
        }

        let mut total = Arc::try_unwrap(report)
            .map_err(|_| anyhow::anyhow!("report Arc still held after join"))?
            .into_inner();
        total.duration_ms = start.elapsed().as_millis() as u64;
        Ok(total)
    }
}
