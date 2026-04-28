//! Tests for the Dreamer worker pool: contract-level behaviour for the
//! query-driven, parallel-by-default architecture from docs/DREAMS.md.
//!
//! The Dreamer trait is exercised through fake test Dreamers rather than
//! the real link/infer/reconcile actions — those have their own tests in
//! unit_dreamer.rs and are integration-tested via run_dream.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::Mutex;

use hippo::backends::InMemoryGraph;
use hippo::graph_backend::GraphBackend;
use hippo::llm;
use hippo::models::Entity;
use hippo::pipeline::dreamer::{DreamReport, Dreamer, DreamerConfig, WorkUnit, WorkerPool};

async fn seed(graph: &InMemoryGraph, id: &str, name: &str) {
    graph
        .upsert_entity(&Entity {
            id: id.into(),
            name: name.into(),
            entity_type: "person".into(),
            resolved: true,
            hint: None,
            content: None,
            created_at: Utc::now(),
            embedding: llm::pseudo_embed(name),
        })
        .await
        .unwrap();
}

/// A Dreamer that just counts visits, tracking them internally.
struct CountingDreamer {
    visits: Arc<Mutex<Vec<String>>>,
    calls: Arc<AtomicUsize>,
}

impl CountingDreamer {
    fn new() -> Self {
        Self {
            visits: Arc::new(Mutex::new(Vec::new())),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl Dreamer for CountingDreamer {
    fn name(&self) -> &str {
        "counting"
    }

    async fn next_unit(&self, graph: &dyn GraphBackend) -> anyhow::Result<Option<WorkUnit>> {
        let visited = self.visits.lock().await;
        let entities = graph.dump_all_entities().await?;
        for e in entities {
            if !visited.contains(&e.id) {
                return Ok(Some(WorkUnit {
                    entity_id: e.id,
                    score: 0.0,
                }));
            }
        }
        Ok(None)
    }

    async fn process(
        &self,
        _graph: &dyn GraphBackend,
        unit: WorkUnit,
    ) -> anyhow::Result<DreamReport> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.visits.lock().await.push(unit.entity_id);
        Ok(DreamReport {
            facts_visited: 1,
            ..DreamReport::default()
        })
    }
}

/// A Dreamer that uses graph.last_visited / mark_visited as its idempotency
/// signal — i.e. relies on the graph itself, not internal state.
struct GraphMarkingDreamer;

#[async_trait]
impl Dreamer for GraphMarkingDreamer {
    fn name(&self) -> &str {
        "graph-marking"
    }

    async fn next_unit(&self, graph: &dyn GraphBackend) -> anyhow::Result<Option<WorkUnit>> {
        let entities = graph.dump_all_entities().await?;
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

    async fn process(
        &self,
        graph: &dyn GraphBackend,
        unit: WorkUnit,
    ) -> anyhow::Result<DreamReport> {
        tokio::task::yield_now().await;
        graph.mark_visited(&unit.entity_id, Utc::now()).await?;
        Ok(DreamReport {
            facts_visited: 1,
            ..DreamReport::default()
        })
    }
}

// ---- Pool drains all eligible units ----

#[tokio::test]
async fn pool_drains_all_eligible_units() {
    let im = Arc::new(InMemoryGraph::new("test"));
    seed(&im, "a", "Alice").await;
    seed(&im, "b", "Bob").await;
    seed(&im, "c", "Charlie").await;
    let graph: Arc<dyn GraphBackend> = im;

    let dreamer = Arc::new(CountingDreamer::new());
    let pool = WorkerPool::new(DreamerConfig::bounded(1, None, None));

    let report = pool.run_dream(dreamer.clone(), graph).await.unwrap();

    assert_eq!(report.facts_visited, 3);
    assert_eq!(dreamer.call_count(), 3);
}

#[tokio::test]
async fn pool_respects_max_units_budget() {
    let im = Arc::new(InMemoryGraph::new("test"));
    for c in ["a", "b", "c", "d", "e"] {
        seed(&im, c, &format!("Entity {c}")).await;
    }
    let graph: Arc<dyn GraphBackend> = im;

    let dreamer = Arc::new(CountingDreamer::new());
    let pool = WorkerPool::new(DreamerConfig::bounded(1, Some(2), None));

    let report = pool.run_dream(dreamer.clone(), graph).await.unwrap();

    assert_eq!(report.facts_visited, 2, "should stop after 2 units");
    assert_eq!(dreamer.call_count(), 2);
}

#[tokio::test]
async fn pool_aggregates_reports_across_units() {
    let im = Arc::new(InMemoryGraph::new("test"));
    seed(&im, "a", "Alice").await;
    seed(&im, "b", "Bob").await;
    let graph: Arc<dyn GraphBackend> = im;

    struct ReportingDreamer;
    #[async_trait]
    impl Dreamer for ReportingDreamer {
        fn name(&self) -> &str {
            "reporting"
        }
        async fn next_unit(&self, graph: &dyn GraphBackend) -> anyhow::Result<Option<WorkUnit>> {
            let entities = graph.dump_all_entities().await?;
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
        async fn process(
            &self,
            graph: &dyn GraphBackend,
            unit: WorkUnit,
        ) -> anyhow::Result<DreamReport> {
            graph.mark_visited(&unit.entity_id, Utc::now()).await?;
            Ok(DreamReport {
                facts_visited: 1,
                links_written: 2,
                supersessions_written: 1,
                contradictions_seen: 3,
                tokens_used: 100,
                ..DreamReport::default()
            })
        }
    }

    let pool = WorkerPool::new(DreamerConfig::bounded(1, None, None));
    let report = pool
        .run_dream(Arc::new(ReportingDreamer), graph)
        .await
        .unwrap();

    assert_eq!(report.facts_visited, 2);
    assert_eq!(report.links_written, 4);
    assert_eq!(report.supersessions_written, 2);
    assert_eq!(report.contradictions_seen, 6);
    assert_eq!(report.tokens_used, 200);
}

// ---- Token budget terminates early ----

#[tokio::test]
async fn pool_respects_token_budget() {
    let im = Arc::new(InMemoryGraph::new("test"));
    for c in ["a", "b", "c", "d"] {
        seed(&im, c, &format!("Entity {c}")).await;
    }
    let graph: Arc<dyn GraphBackend> = im;

    struct ExpensiveDreamer {
        visits: Arc<Mutex<Vec<String>>>,
    }
    #[async_trait]
    impl Dreamer for ExpensiveDreamer {
        fn name(&self) -> &str {
            "expensive"
        }
        async fn next_unit(&self, graph: &dyn GraphBackend) -> anyhow::Result<Option<WorkUnit>> {
            let visited = self.visits.lock().await;
            let entities = graph.dump_all_entities().await?;
            for e in entities {
                if !visited.contains(&e.id) {
                    return Ok(Some(WorkUnit {
                        entity_id: e.id,
                        score: 0.0,
                    }));
                }
            }
            Ok(None)
        }
        async fn process(
            &self,
            _graph: &dyn GraphBackend,
            unit: WorkUnit,
        ) -> anyhow::Result<DreamReport> {
            self.visits.lock().await.push(unit.entity_id);
            Ok(DreamReport {
                facts_visited: 1,
                tokens_used: 60,
                ..DreamReport::default()
            })
        }
    }

    let dreamer = Arc::new(ExpensiveDreamer {
        visits: Arc::new(Mutex::new(Vec::new())),
    });
    let pool = WorkerPool::new(DreamerConfig::bounded(1, None, Some(100)));
    let report = pool.run_dream(dreamer, graph).await.unwrap();

    assert!(
        report.facts_visited <= 2,
        "should stop near the token budget, got {}",
        report.facts_visited
    );
    assert!(report.tokens_used >= 60);
}

// ---- Multiple workers in parallel don't race ----

#[tokio::test]
async fn parallel_workers_visit_each_entity_at_most_once() {
    let im = Arc::new(InMemoryGraph::new("test"));
    for i in 0..20 {
        seed(&im, &format!("e{i}"), &format!("Entity {i}")).await;
    }
    let graph: Arc<dyn GraphBackend> = im;

    let pool = WorkerPool::new(DreamerConfig::bounded(4, None, None));
    let report = pool
        .run_dream(Arc::new(GraphMarkingDreamer), graph)
        .await
        .unwrap();

    // Visit count == entity count: the last-visited filter (consulted in the
    // Dreamer's next_unit) prevented races.
    assert_eq!(report.facts_visited, 20);
}

// ---- Idempotency: running again yields no new visits ----

#[tokio::test]
async fn second_dream_pass_with_recency_filter_is_a_noop() {
    let im = Arc::new(InMemoryGraph::new("test"));
    seed(&im, "a", "Alice").await;
    seed(&im, "b", "Bob").await;
    seed(&im, "c", "Charlie").await;
    let graph: Arc<dyn GraphBackend> = im;

    let pool = WorkerPool::new(DreamerConfig::bounded(1, None, None));
    let r1 = pool
        .run_dream(Arc::new(GraphMarkingDreamer), graph.clone())
        .await
        .unwrap();
    let r2 = pool
        .run_dream(Arc::new(GraphMarkingDreamer), graph)
        .await
        .unwrap();

    assert_eq!(r1.facts_visited, 3);
    assert_eq!(
        r2.facts_visited, 0,
        "second pass should be a no-op because all entities have last_visited set",
    );
}
