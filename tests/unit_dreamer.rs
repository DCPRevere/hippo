//! Tests for the Dreamer architecture as specified in docs/DREAMS.md.
//!
//! These tests describe the contract: salience is wired up, dreaming is
//! append-only via `supersedes` (not `invalid_at`), and `last_visited` filters
//! prevent immediate revisit.

use chrono::{Duration, Utc};

use hippo::backends::InMemoryGraph;
use hippo::credibility::SourceCredibility;
use hippo::graph_backend::GraphBackend;
use hippo::llm;
use hippo::models::{Entity, MemoryTier, Relation};

fn cred(agent: &str, value: f32) -> SourceCredibility {
    SourceCredibility {
        agent_id: agent.into(),
        credibility: value,
        fact_count: 0,
        contradiction_rate: 0.0,
    }
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

fn make_rel(fact: &str, salience: i64) -> Relation {
    Relation {
        fact: fact.into(),
        relation_type: "RELATED_TO".into(),
        embedding: llm::pseudo_embed(fact),
        source_agents: vec!["test".into()],
        valid_at: Utc::now(),
        invalid_at: None,
        confidence: 0.9,
        salience,
        created_at: Utc::now(),
        memory_tier: MemoryTier::LongTerm,
        expires_at: None,
    }
}

// ---- Salience: increment-on-use ----

#[tokio::test]
async fn salience_is_incremented_when_edge_is_retrieved() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;

    let rel = make_rel("Alice knows Bob", 0);
    let edge_id = graph.create_edge("a", "b", &rel).await.unwrap();

    let before = graph
        .dump_all_edges()
        .await
        .unwrap()
        .iter()
        .find(|e| e.edge_id == edge_id)
        .unwrap()
        .salience;
    assert_eq!(before, 0);

    graph.bump_salience(&[edge_id]).await.unwrap();

    let after = graph
        .dump_all_edges()
        .await
        .unwrap()
        .iter()
        .find(|e| e.edge_id == edge_id)
        .unwrap()
        .salience;
    assert_eq!(after, 1, "salience should increment by 1 per bump");
}

#[tokio::test]
async fn bump_salience_handles_multiple_edges() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;
    seed_entity(&graph, "c", "Charlie").await;

    let e1 = graph
        .create_edge("a", "b", &make_rel("Alice knows Bob", 0))
        .await
        .unwrap();
    let e2 = graph
        .create_edge("a", "c", &make_rel("Alice knows Charlie", 5))
        .await
        .unwrap();

    graph.bump_salience(&[e1, e2]).await.unwrap();

    let edges = graph.dump_all_edges().await.unwrap();
    let s1 = edges.iter().find(|e| e.edge_id == e1).unwrap().salience;
    let s2 = edges.iter().find(|e| e.edge_id == e2).unwrap().salience;
    assert_eq!(s1, 1);
    assert_eq!(s2, 6);
}

// ---- Salience: affects ranking ----

#[tokio::test]
async fn higher_salience_ranks_higher_when_similarity_is_equal() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;
    seed_entity(&graph, "c", "Charlie").await;

    // Two edges with the *same* fact text → identical embeddings → identical
    // cosine similarity to any query. Salience is the only tiebreaker that
    // can produce a deterministic ordering. If salience isn't consulted, the
    // sort is unstable and the test will fail intermittently — which is
    // exactly what we want for a contract test on ranking.
    let low = graph
        .create_edge("a", "b", &make_rel("met at the cafe", 0))
        .await
        .unwrap();
    let high = graph
        .create_edge("a", "c", &make_rel("met at the cafe", 100))
        .await
        .unwrap();

    let query_embed = llm::pseudo_embed("met at the cafe");
    let results = graph
        .vector_search_edges_scored(&query_embed, 10, None)
        .await
        .unwrap();

    let rank_high = results.iter().position(|(e, _)| e.edge_id == high);
    let rank_low = results.iter().position(|(e, _)| e.edge_id == low);
    assert!(rank_high.is_some() && rank_low.is_some());
    assert!(
        rank_high < rank_low,
        "high-salience edge ({:?}) should rank before low-salience ({:?})",
        rank_high,
        rank_low,
    );
}

// ---- Credibility: affects ranking ----

#[tokio::test]
async fn high_credibility_source_outranks_low_credibility() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;
    seed_entity(&graph, "c", "Charlie").await;

    // Identical fact text so cosine similarity is equal between the two edges.
    // Credibility must be the tiebreaker.
    let mut trustworthy = make_rel("met at the cafe", 0);
    trustworthy.source_agents = vec!["trusted_crm".into()];
    let mut shaky = make_rel("met at the cafe", 0);
    shaky.source_agents = vec!["unreliable_chat".into()];

    let edge_trusted = graph.create_edge("a", "b", &trustworthy).await.unwrap();
    let edge_shaky = graph.create_edge("a", "c", &shaky).await.unwrap();

    graph
        .save_source_credibility(&cred("trusted_crm", 0.95))
        .await
        .unwrap();
    graph
        .save_source_credibility(&cred("unreliable_chat", 0.35))
        .await
        .unwrap();

    let query_embed = llm::pseudo_embed("met at the cafe");
    let results = graph
        .vector_search_edges_scored(&query_embed, 10, None)
        .await
        .unwrap();

    let rank_trusted = results.iter().position(|(e, _)| e.edge_id == edge_trusted);
    let rank_shaky = results.iter().position(|(e, _)| e.edge_id == edge_shaky);
    assert!(rank_trusted.is_some() && rank_shaky.is_some());
    assert!(
        rank_trusted < rank_shaky,
        "trusted-source edge should rank before unreliable-source",
    );
}

// ---- Append-only: supersedes instead of invalidate ----

#[tokio::test]
async fn dreaming_writes_supersedes_not_invalidations() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;

    let old = graph
        .create_edge("a", "b", &make_rel("Alice is a lawyer", 0))
        .await
        .unwrap();
    let new = graph
        .create_edge("a", "b", &make_rel("Alice is a doctor", 0))
        .await
        .unwrap();

    // Dreamer-style supersession: write a `supersedes` relationship.
    // The original facts MUST remain active (no invalid_at set).
    graph.supersede_edge(old, new).await.unwrap();

    let edges = graph.dump_all_edges().await.unwrap();
    let old_edge = edges.iter().find(|e| e.edge_id == old).unwrap();
    let new_edge = edges.iter().find(|e| e.edge_id == new).unwrap();

    assert!(
        old_edge.invalid_at.is_none(),
        "original fact must remain active under append-only dreaming",
    );
    assert!(new_edge.invalid_at.is_none());

    // Provenance should reflect the supersession.
    let prov = graph.get_provenance(old).await.unwrap();
    assert!(
        prov.superseded_by.is_some(),
        "old edge should report being superseded",
    );
}

#[tokio::test]
async fn superseded_edges_filtered_from_active_search() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;

    let old = graph
        .create_edge("a", "b", &make_rel("Alice is a lawyer at Acme", 0))
        .await
        .unwrap();
    let new = graph
        .create_edge("a", "b", &make_rel("Alice is a doctor at Acme", 0))
        .await
        .unwrap();
    graph.supersede_edge(old, new).await.unwrap();

    let query = llm::pseudo_embed("Alice's profession at Acme");
    let results = graph
        .vector_search_edges_scored(&query, 10, None)
        .await
        .unwrap();

    let has_old = results.iter().any(|(e, _)| e.edge_id == old);
    let has_new = results.iter().any(|(e, _)| e.edge_id == new);
    assert!(has_new, "current fact should still be retrievable");
    assert!(
        !has_old,
        "superseded fact should be filtered from active search",
    );
}

// ---- Last-visited: revisit window ----

#[tokio::test]
async fn last_visited_timestamp_is_recorded() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;

    let before = graph.last_visited("a").await.unwrap();
    assert!(before.is_none(), "fresh entity should have no last_visited");

    graph.mark_visited("a", Utc::now()).await.unwrap();

    let after = graph.last_visited("a").await.unwrap();
    assert!(after.is_some(), "marking visited should record a timestamp");
}

#[tokio::test]
async fn unvisited_entities_are_eligible_for_dreaming() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;
    seed_entity(&graph, "c", "Charlie").await;

    graph.mark_visited("a", Utc::now()).await.unwrap();
    graph
        .mark_visited("b", Utc::now() - Duration::hours(48))
        .await
        .unwrap();
    // Charlie never visited.

    let cutoff = Utc::now() - Duration::hours(24);
    let eligible = graph.entities_unvisited_since(cutoff).await.unwrap();
    let names: std::collections::HashSet<String> =
        eligible.iter().map(|e| e.name.clone()).collect();

    assert!(
        names.contains("Charlie"),
        "never-visited entity should be eligible",
    );
    assert!(
        names.contains("Bob"),
        "stale-visited entity should be eligible",
    );
    assert!(
        !names.contains("Alice"),
        "recently-visited entity should NOT be eligible",
    );
}

// ---- Retract: explicit destructive user/agent operation ----

#[tokio::test]
async fn retract_marks_edge_inactive_and_preserves_audit_trail() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;

    let edge_id = graph
        .create_edge("a", "b", &make_rel("Alice is a doctor", 0))
        .await
        .unwrap();

    graph
        .retract_edge(edge_id, Some("extraction error"))
        .await
        .unwrap();

    // Active retrieval should not see it.
    let query = llm::pseudo_embed("Alice is a doctor");
    let results = graph
        .vector_search_edges_scored(&query, 10, None)
        .await
        .unwrap();
    assert!(
        !results.iter().any(|(e, _)| e.edge_id == edge_id),
        "retracted edge should not appear in active search",
    );

    // The audit trail is preserved: the edge still exists with invalid_at set
    // and a retraction reason recorded.
    let edges = graph.dump_all_edges().await.unwrap();
    let retracted = edges
        .iter()
        .find(|e| e.edge_id == edge_id)
        .expect("retracted edge should still exist for audit");
    assert!(
        retracted.invalid_at.is_some(),
        "retracted edge should be marked inactive",
    );

    let reason = graph.retraction_reason(edge_id).await.unwrap();
    assert_eq!(reason.as_deref(), Some("extraction error"));
}

#[tokio::test]
async fn retract_is_distinct_from_supersession() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;

    let edge_id = graph
        .create_edge("a", "b", &make_rel("Alice is a doctor", 0))
        .await
        .unwrap();

    graph.retract_edge(edge_id, None).await.unwrap();

    // Provenance should show retraction, not supersession.
    let prov = graph.get_provenance(edge_id).await.unwrap();
    assert!(
        prov.superseded_by.is_none(),
        "retracted edge is not superseded — supersession is a Dreamer-written fact, retraction is a user/agent operation",
    );
}

#[tokio::test]
async fn correct_retracts_old_and_observes_new() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;

    let old_edge = graph
        .create_edge("a", "b", &make_rel("Alice is a doctor", 0))
        .await
        .unwrap();

    let new_rel = make_rel("Alice is a dentist", 0);
    let new_edge = graph
        .correct_edge(old_edge, "a", "b", &new_rel, Some("user correction"))
        .await
        .unwrap();

    assert_ne!(old_edge, new_edge);

    // Old is retracted, new is active.
    let edges = graph.dump_all_edges().await.unwrap();
    let old = edges.iter().find(|e| e.edge_id == old_edge).unwrap();
    let new = edges.iter().find(|e| e.edge_id == new_edge).unwrap();
    assert!(old.invalid_at.is_some(), "old edge should be retracted");
    assert!(new.invalid_at.is_none(), "new edge should be active");

    // Reason captured on the retracted edge.
    let reason = graph.retraction_reason(old_edge).await.unwrap();
    assert_eq!(reason.as_deref(), Some("user correction"));
}

// ---- Idempotency: dreaming twice yields the same state ----

#[tokio::test]
async fn supersede_is_idempotent() {
    let graph = InMemoryGraph::new("test");
    seed_entity(&graph, "a", "Alice").await;
    seed_entity(&graph, "b", "Bob").await;

    let old = graph
        .create_edge("a", "b", &make_rel("Alice is a lawyer", 0))
        .await
        .unwrap();
    let new = graph
        .create_edge("a", "b", &make_rel("Alice is a doctor", 0))
        .await
        .unwrap();

    graph.supersede_edge(old, new).await.unwrap();
    let edges_first = graph.dump_all_edges().await.unwrap().len();
    let chain_first = graph.get_supersession_chain(old).await.unwrap().len();

    // Calling supersede a second time with the same pair should be a no-op.
    graph.supersede_edge(old, new).await.unwrap();
    let edges_second = graph.dump_all_edges().await.unwrap().len();
    let chain_second = graph.get_supersession_chain(old).await.unwrap().len();

    assert_eq!(edges_first, edges_second, "edge count should not grow");
    assert_eq!(
        chain_first, chain_second,
        "supersession chain should not grow",
    );
}
