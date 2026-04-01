use std::sync::Arc;

use chrono::{Duration, Utc};

use hippo::config::Config;
use hippo::graph_backend::GraphBackend;
use hippo::in_memory_graph::InMemoryGraph;
use hippo::llm;
use hippo::models::{Entity, MemoryTier, Relation};
use hippo::pipeline::maintain;
use hippo::state::AppState;
use hippo::testing::FakeLlm;

fn test_state() -> Arc<AppState> {
    Arc::new(AppState::for_test(
        Arc::new(FakeLlm::new()),
        Config::test_default(),
    ))
}

async fn seed_entity(graph: &InMemoryGraph, id: &str, name: &str) {
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

fn make_rel(fact: &str, tier: MemoryTier, salience: i64, age: Duration) -> Relation {
    Relation {
        fact: fact.into(),
        relation_type: "RELATED_TO".into(),
        embedding: llm::pseudo_embed(fact),
        source_agents: vec!["test".into()],
        valid_at: Utc::now() - age,
        invalid_at: None,
        confidence: 0.9,
        salience,
        created_at: Utc::now() - age,
        memory_tier: tier,
        expires_at: None,
    }
}

// ---- Promote working → long_term ----

#[tokio::test]
async fn maintain_promotes_working_to_long_term() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;

    // Working edge, high salience, created 2 hours ago → should be promoted
    let rel = make_rel(
        "Alice knows Bob",
        MemoryTier::Working,
        5,
        Duration::hours(2),
    );
    graph.create_edge("a", "b", &rel).await.unwrap();

    let working_before = graph.memory_tier_stats().await.unwrap().working_count;
    assert_eq!(working_before, 1);

    let promoted = graph.promote_working_memory().await.unwrap();
    assert_eq!(promoted, 1, "should promote 1 edge");

    let tier = graph.memory_tier_stats().await.unwrap();
    assert_eq!(tier.working_count, 0);
    assert_eq!(tier.long_term_count, 1);
}

#[tokio::test]
async fn maintain_does_not_promote_low_salience() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;

    // Working edge, low salience — should NOT be promoted
    let rel = make_rel(
        "Alice saw Bob",
        MemoryTier::Working,
        1,
        Duration::hours(2),
    );
    graph.create_edge("a", "b", &rel).await.unwrap();

    let promoted = graph.promote_working_memory().await.unwrap();
    assert_eq!(promoted, 0, "low salience should not be promoted");
}

// ---- TTL expiry ----

#[tokio::test]
async fn maintain_expires_ttl_edges() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;

    // Edge with expires_at in the past → should be expired
    let mut rel = make_rel(
        "Alice met Bob yesterday",
        MemoryTier::Working,
        0,
        Duration::hours(2),
    );
    rel.expires_at = Some(Utc::now() - Duration::hours(1));
    graph.create_edge("a", "b", &rel).await.unwrap();

    let expired = graph.expire_ttl_edges(Utc::now()).await.unwrap();
    assert_eq!(expired, 1, "should expire 1 edge past TTL");

    let working = graph.memory_tier_stats().await.unwrap().working_count;
    assert_eq!(working, 0, "expired edge should no longer be active");
}

#[tokio::test]
async fn maintain_does_not_expire_future_ttl() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;

    // Edge with expires_at in the future → should NOT be expired
    let mut rel = make_rel(
        "Alice and Bob are friends",
        MemoryTier::Working,
        0,
        Duration::hours(1),
    );
    rel.expires_at = Some(Utc::now() + Duration::hours(24));
    graph.create_edge("a", "b", &rel).await.unwrap();

    let expired = graph.expire_ttl_edges(Utc::now()).await.unwrap();
    assert_eq!(expired, 0, "future TTL should not be expired");

    let working = graph.memory_tier_stats().await.unwrap().working_count;
    assert_eq!(working, 1, "edge should still be active");
}

#[tokio::test]
async fn maintain_no_ttl_means_no_expiry() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;

    // Edge with no expires_at → should never be expired
    let rel = make_rel(
        "Alice knows Bob",
        MemoryTier::Working,
        0,
        Duration::hours(100),
    );
    graph.create_edge("a", "b", &rel).await.unwrap();

    let expired = graph.expire_ttl_edges(Utc::now()).await.unwrap();
    assert_eq!(expired, 0, "edge without TTL should not be expired");
}

// ---- Decay stale edges ----

#[tokio::test]
async fn maintain_decays_stale_edges() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;

    let rel = make_rel(
        "Alice knows Bob from university",
        MemoryTier::LongTerm,
        2,
        Duration::days(60),
    );
    graph.create_edge("a", "b", &rel).await.unwrap();

    let stale_before = Utc::now() - Duration::days(30);
    let decayed = graph.decay_stale_edges(stale_before, Utc::now()).await.unwrap();
    assert_eq!(decayed, 1, "should decay 1 edge");

    let edges = graph.dump_all_edges().await.unwrap();
    assert!(
        edges[0].decayed_confidence < 0.9,
        "decayed confidence {} should be less than original 0.9",
        edges[0].decayed_confidence
    );
}

// ---- Run full housekeeping cycle ----

#[tokio::test]
async fn maintain_run_once_completes_without_error() {
    let state = test_state();
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;

    let rel = make_rel(
        "Alice knows Bob",
        MemoryTier::LongTerm,
        2,
        Duration::days(5),
    );
    graph.create_edge("a", "b", &rel).await.unwrap();

    // run_once should complete without panicking
    maintain::run_once(&state, &graph).await.unwrap();
}
