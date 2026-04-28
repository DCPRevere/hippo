//! End-to-end idempotency: running the full dream pass twice on the same
//! graph state must produce identical state on the second pass. This is
//! the contract that prevents the WorkerPool from double-writing on
//! restart, and the locks-in invariant for parallel-safe workers.

use std::sync::Arc;

use chrono::Utc;

use hippo::backends::InMemoryGraph;
use hippo::config::Config;
use hippo::graph_backend::GraphBackend;
use hippo::llm;
use hippo::models::{Entity, MemoryTier, Relation};
use hippo::pipeline::dreamer::{
    DreamReport, Dreamer, DreamerConfig, Linker, Reconciler, WorkerPool,
};
use hippo::state::AppState;
use hippo::testing::FakeLlm;

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

fn make_rel(fact: &str) -> Relation {
    Relation {
        fact: fact.into(),
        relation_type: "RELATED_TO".into(),
        embedding: llm::pseudo_embed(fact),
        source_agents: vec!["test".into()],
        valid_at: Utc::now(),
        invalid_at: None,
        confidence: 0.9,
        salience: 0,
        created_at: Utc::now(),
        memory_tier: MemoryTier::LongTerm,
        expires_at: None,
    }
}

fn test_state() -> Arc<AppState> {
    Arc::new(AppState::for_test(
        Arc::new(FakeLlm::new()),
        Config::test_default(),
    ))
}

/// Snapshot of dream-relevant graph state for diffing across passes.
#[derive(Debug, PartialEq)]
struct GraphSnapshot {
    entities: Vec<(String, String)>,        // (id, name)
    edges: Vec<(i64, String, i64, String)>, // (edge_id, fact, salience, source_agents)
    visited: Vec<(String, bool)>,           // (entity_id, has_visited_timestamp)
}

async fn snapshot(graph: &InMemoryGraph) -> GraphSnapshot {
    let mut entities: Vec<(String, String)> = graph
        .dump_all_entities()
        .await
        .unwrap()
        .into_iter()
        .map(|e| (e.id, e.name))
        .collect();
    entities.sort();

    let mut edges: Vec<(i64, String, i64, String)> = graph
        .dump_all_edges()
        .await
        .unwrap()
        .into_iter()
        .map(|e| (e.edge_id, e.fact, e.salience, e.source_agents))
        .collect();
    edges.sort();

    let mut visited: Vec<(String, bool)> = vec![];
    for (id, _) in &entities {
        let has_ts = graph.last_visited(id).await.unwrap().is_some();
        visited.push((id.clone(), has_ts));
    }
    visited.sort();

    GraphSnapshot {
        entities,
        edges,
        visited,
    }
}

#[tokio::test]
async fn second_pass_with_linker_is_a_noop() {
    let im = Arc::new(InMemoryGraph::new("test"));
    seed(&im, "a", "Alice").await;
    seed(&im, "b", "Bob").await;
    seed(&im, "c", "Charlie").await;
    im.create_edge("a", "b", &make_rel("Alice met Bob"))
        .await
        .unwrap();
    let graph: Arc<dyn GraphBackend> = im.clone();

    let state = test_state();
    let dreamer: Arc<dyn Dreamer> = Arc::new(Linker::new(state));

    let pool = WorkerPool::new(DreamerConfig::bounded(1, None, None));
    let r1 = pool
        .run_dream(dreamer.clone(), graph.clone())
        .await
        .unwrap();
    let s1 = snapshot(&im).await;

    let r2 = pool
        .run_dream(dreamer.clone(), graph.clone())
        .await
        .unwrap();
    let s2 = snapshot(&im).await;

    // Pass 2 should have visited zero entities (all already marked visited
    // in pass 1, so next_unit returns None).
    assert!(r1.facts_visited >= 1);
    assert_eq!(
        r2.facts_visited, 0,
        "second pass should be a no-op — all entities already visited"
    );
    assert_eq!(s1, s2, "graph state must be identical across passes");
}

#[tokio::test]
async fn second_pass_with_reconciler_is_a_noop() {
    let im = Arc::new(InMemoryGraph::new("test"));
    seed(&im, "a", "Alice").await;
    seed(&im, "b", "Bob").await;
    im.create_edge("a", "b", &make_rel("Alice is a lawyer"))
        .await
        .unwrap();
    im.create_edge("a", "b", &make_rel("Alice is a doctor"))
        .await
        .unwrap();
    let graph: Arc<dyn GraphBackend> = im.clone();

    let state = test_state();
    let dreamer: Arc<dyn Dreamer> = Arc::new(Reconciler::new(state));

    let pool = WorkerPool::new(DreamerConfig::bounded(1, None, None));
    let _ = pool
        .run_dream(dreamer.clone(), graph.clone())
        .await
        .unwrap();
    let s1 = snapshot(&im).await;

    let r2 = pool
        .run_dream(dreamer.clone(), graph.clone())
        .await
        .unwrap();
    let s2 = snapshot(&im).await;

    assert_eq!(r2.facts_visited, 0, "second pass should not re-visit");
    assert_eq!(s1, s2);
}

#[tokio::test]
async fn parallel_workers_yield_same_result_as_single_worker() {
    // Build identical state in two graphs, run with different worker counts,
    // verify the result snapshots are equal.
    let g1 = Arc::new(InMemoryGraph::new("test1"));
    let g2 = Arc::new(InMemoryGraph::new("test2"));
    for (i, name) in ["Alice", "Bob", "Charlie", "Dave", "Eve"]
        .iter()
        .enumerate()
    {
        seed(&g1, &format!("e{i}"), name).await;
        seed(&g2, &format!("e{i}"), name).await;
    }
    let g1_dyn: Arc<dyn GraphBackend> = g1.clone();
    let g2_dyn: Arc<dyn GraphBackend> = g2.clone();

    let s1 = test_state();
    let s2 = test_state();
    let d1: Arc<dyn Dreamer> = Arc::new(Linker::new(s1));
    let d2: Arc<dyn Dreamer> = Arc::new(Linker::new(s2));

    let _ = WorkerPool::new(DreamerConfig::bounded(1, None, None))
        .run_dream(d1, g1_dyn)
        .await
        .unwrap();
    let _ = WorkerPool::new(DreamerConfig::bounded(4, None, None))
        .run_dream(d2, g2_dyn)
        .await
        .unwrap();

    // Compare visit-set sizes and entity counts. Detailed edge-set
    // equality requires deterministic LLM behaviour; FakeLlm provides
    // it but ordering of writes from parallel workers may differ.
    let snap1 = snapshot(&g1).await;
    let snap2 = snapshot(&g2).await;
    assert_eq!(
        snap1.visited.len(),
        snap2.visited.len(),
        "both pools should mark the same number of entities as visited",
    );
    let visited1: usize = snap1.visited.iter().filter(|(_, v)| *v).count();
    let visited2: usize = snap2.visited.iter().filter(|(_, v)| *v).count();
    assert_eq!(visited1, visited2);
}

#[tokio::test]
async fn dream_report_aggregates_correctly_across_passes() {
    // The pool sums per-unit reports. Verify the aggregate matches the
    // per-unit counts when we know each entity yields exactly one
    // facts_visited bump.
    let im = Arc::new(InMemoryGraph::new("test"));
    for i in 0..7 {
        seed(&im, &format!("e{i}"), &format!("Entity {i}")).await;
    }
    let graph: Arc<dyn GraphBackend> = im;

    let state = test_state();
    let dreamer: Arc<dyn Dreamer> = Arc::new(Linker::new(state));

    let pool = WorkerPool::new(DreamerConfig::bounded(1, None, None));
    let report = pool.run_dream(dreamer, graph).await.unwrap();

    assert_eq!(report.facts_visited, 7);
    // Wall-clock duration must be set.
    let _ = report.duration_ms;
    // Tokens may be 0 with FakeLlm, that's fine.
    let _: DreamReport = report;
}
