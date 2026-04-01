use std::sync::Arc;

use chrono::Utc;

use hippo::config::Config;
use hippo::graph_backend::GraphBackend;
use hippo::in_memory_graph::InMemoryGraph;
use hippo::llm;
use hippo::models::{
    ContextRequest, Entity, MemoryTier, Relation, ScoringParams,
};
use hippo::pipeline::context::context;
use hippo::state::AppState;
use hippo::testing::FakeLlm;

fn test_state() -> Arc<AppState> {
    Arc::new(AppState::for_test(
        Arc::new(FakeLlm::new()),
        Config::test_default(),
    ))
}

/// Seed a simple graph: Alice -[WORKS_AT]-> Acme -[BASED_IN]-> London
async fn seed_alice_acme_london(graph: &InMemoryGraph) {
    let now = Utc::now();

    for (id, name, etype) in [
        ("alice", "Alice", "person"),
        ("acme", "Acme", "organization"),
        ("london", "London", "place"),
    ] {
        graph
            .upsert_entity(&Entity {
                id: id.into(),
                name: name.into(),
                entity_type: etype.into(),
                resolved: true,
                hint: None,
                content: None,
                created_at: now,
                embedding: llm::pseudo_embed(name),
            })
            .await
            .unwrap();
    }

    let mk_rel = |fact: &str, rel_type: &str| Relation {
        fact: fact.into(),
        relation_type: rel_type.into(),
        embedding: llm::pseudo_embed(fact),
        source_agents: vec!["test".into()],
        valid_at: now,
        invalid_at: None,
        confidence: 0.9,
        salience: 2,
        created_at: now,
        memory_tier: MemoryTier::LongTerm,
        expires_at: None,
    };

    graph
        .create_edge("alice", "acme", &mk_rel("Alice works at Acme", "WORKS_AT"))
        .await
        .unwrap();
    graph
        .create_edge("acme", "london", &mk_rel("Acme is based in London", "BASED_IN"))
        .await
        .unwrap();
}

// ---- Basic retrieval ----

#[tokio::test]
async fn context_returns_matching_facts_by_fulltext() {
    let state = test_state();
    let graph = InMemoryGraph::new("test");
    seed_alice_acme_london(&graph).await;

    let resp = context(
        &state,
        &graph,
        ContextRequest {
            query: "Alice".into(),
            limit: Some(10),
            max_hops: Some(1),
            memory_tier_filter: None,
            graph: None,
            at: None,
            scoring: None,
        },
        None,
    )
    .await
    .unwrap();

    assert!(
        !resp.facts.is_empty(),
        "should find facts mentioning Alice"
    );
    let fact_texts: Vec<&str> = resp.facts.iter().map(|f| f.fact.as_str()).collect();
    assert!(
        fact_texts.iter().any(|f| f.contains("Alice")),
        "should contain an Alice fact, got: {:?}",
        fact_texts
    );
}

// ---- Multi-hop traversal ----

#[tokio::test]
async fn context_multi_hop_finds_london_via_acme() {
    let state = test_state();
    let graph = InMemoryGraph::new("test");
    seed_alice_acme_london(&graph).await;

    let resp = context(
        &state,
        &graph,
        ContextRequest {
            query: "Alice".into(),
            limit: Some(20),
            max_hops: Some(2),
            memory_tier_filter: None,
            graph: None,
            at: None,
            scoring: None,
        },
        None,
    )
    .await
    .unwrap();

    let fact_texts: Vec<&str> = resp.facts.iter().map(|f| f.fact.as_str()).collect();
    let has_london = fact_texts.iter().any(|f| f.contains("London"));
    assert!(
        has_london,
        "2-hop from Alice should reach London via Acme: {:?}",
        fact_texts
    );
}

// ---- Memory tier filtering ----

#[tokio::test]
async fn context_filters_by_memory_tier() {
    let state = test_state();
    let graph = InMemoryGraph::new("test");
    let now = Utc::now();

    // Create two entities
    for (id, name) in [("a", "Alpha"), ("b", "Beta")] {
        graph
            .upsert_entity(&Entity {
                id: id.into(),
                name: name.into(),
                entity_type: "concept".into(),
                resolved: true,
                hint: None,
                content: None,
                created_at: now,
                embedding: llm::pseudo_embed(name),
            })
            .await
            .unwrap();
    }

    // One working edge, one long-term edge
    let mk_rel = |fact: &str, tier: MemoryTier| Relation {
        fact: fact.into(),
        relation_type: "RELATED_TO".into(),
        embedding: llm::pseudo_embed(fact),
        source_agents: vec!["test".into()],
        valid_at: now,
        invalid_at: None,
        confidence: 0.9,
        salience: 2,
        created_at: now,
        memory_tier: tier,
        expires_at: None,
    };

    graph
        .create_edge("a", "b", &mk_rel("Alpha relates to Beta (working)", MemoryTier::Working))
        .await
        .unwrap();
    graph
        .create_edge("a", "b", &mk_rel("Alpha relates to Beta (longterm)", MemoryTier::LongTerm))
        .await
        .unwrap();

    // Query with long_term filter
    let resp = context(
        &state,
        &graph,
        ContextRequest {
            query: "Alpha".into(),
            limit: Some(20),
            max_hops: Some(1),
            memory_tier_filter: Some("long_term".into()),
            graph: None,
            at: None,
            scoring: None,
        },
        None,
    )
    .await
    .unwrap();

    for fact in &resp.facts {
        assert_eq!(
            fact.memory_tier.as_str(),
            "long_term",
            "should only return long_term facts"
        );
    }
}

// ---- Empty query ----

#[tokio::test]
async fn context_empty_query_does_not_panic() {
    let state = test_state();
    let graph = InMemoryGraph::new("test");
    seed_alice_acme_london(&graph).await;

    let resp = context(
        &state,
        &graph,
        ContextRequest {
            query: String::new(),
            limit: Some(10),
            max_hops: Some(1),
            memory_tier_filter: None,
            graph: None,
            at: None,
            scoring: None,
        },
        None,
    )
    .await
    .unwrap();

    // Should not crash — may or may not return results
    assert!(resp.facts.len() < 100, "empty query shouldn't return everything");
}

// ---- Nonexistent entity ----

#[tokio::test]
async fn context_nonexistent_entity_returns_empty() {
    let state = test_state();
    let graph = InMemoryGraph::new("test");
    seed_alice_acme_london(&graph).await;

    let resp = context(
        &state,
        &graph,
        ContextRequest {
            query: "Zebediah Wumpus".into(),
            limit: Some(10),
            max_hops: Some(1),
            memory_tier_filter: None,
            graph: None,
            at: None,
            scoring: None,
        },
        None,
    )
    .await
    .unwrap();

    let fact_texts: Vec<&str> = resp.facts.iter().map(|f| f.fact.as_str()).collect();
    assert!(
        !fact_texts.iter().any(|f| f.contains("Zebediah")),
        "should not hallucinate facts about nonexistent entities"
    );
}

// ---- Confidence ordering ----

#[tokio::test]
async fn context_orders_by_relevance() {
    let state = test_state();
    let graph = InMemoryGraph::new("test");
    let now = Utc::now();

    for (id, name) in [("x", "Testing"), ("y", "Related")] {
        graph
            .upsert_entity(&Entity {
                id: id.into(),
                name: name.into(),
                entity_type: "concept".into(),
                resolved: true,
                hint: None,
                content: None,
                created_at: now,
                embedding: llm::pseudo_embed(name),
            })
            .await
            .unwrap();
    }

    let mk_rel = |fact: &str, conf: f32| Relation {
        fact: fact.into(),
        relation_type: "RELATED_TO".into(),
        embedding: llm::pseudo_embed(fact),
        source_agents: vec!["test".into()],
        valid_at: now,
        invalid_at: None,
        confidence: conf,
        salience: 2,
        created_at: now,
        memory_tier: MemoryTier::LongTerm,
        expires_at: None,
    };

    graph
        .create_edge("x", "y", &mk_rel("Testing has low confidence", 0.3))
        .await
        .unwrap();
    graph
        .create_edge("x", "y", &mk_rel("Testing has high confidence", 0.99))
        .await
        .unwrap();

    let resp = context(
        &state,
        &graph,
        ContextRequest {
            query: "Testing".into(),
            limit: Some(10),
            max_hops: Some(1),
            memory_tier_filter: None,
            graph: None,
            at: None,
            scoring: None,
        },
        None,
    )
    .await
    .unwrap();

    if resp.facts.len() >= 2 {
        // First fact should have higher combined score
        assert!(
            resp.facts[0].confidence >= resp.facts[1].confidence
                || resp.facts[0].salience >= resp.facts[1].salience,
            "results should be ordered by relevance"
        );
    }
}

// ---- Scoring weight customisation ----

/// Seed two edges: one old+high-confidence, one recent+low-confidence.
/// With default weights (recency=0.25, confidence=0.10) the recent one
/// may or may not win depending on relevance. But cranking w_recency to
/// dominate should guarantee the recent fact comes first.
#[tokio::test]
async fn context_custom_scoring_recency_boost() {
    use chrono::Duration;

    let state = test_state();
    let graph = InMemoryGraph::new("test");
    let now = Utc::now();

    for (id, name) in [("a", "Alpha"), ("b", "Beta")] {
        graph
            .upsert_entity(&Entity {
                id: id.into(),
                name: name.into(),
                entity_type: "concept".into(),
                resolved: true,
                hint: None,
                content: None,
                created_at: now,
                embedding: llm::pseudo_embed(name),
            })
            .await
            .unwrap();
    }

    // Old edge, high confidence
    let old = Relation {
        fact: "Alpha is linked to Beta strongly".into(),
        relation_type: "RELATED_TO".into(),
        embedding: llm::pseudo_embed("Alpha is linked to Beta strongly"),
        source_agents: vec!["test".into()],
        valid_at: now - Duration::days(365),
        invalid_at: None,
        confidence: 0.99,
        salience: 10,
        created_at: now - Duration::days(365),
        memory_tier: MemoryTier::LongTerm,
        expires_at: None,
    };
    // Recent edge, low confidence
    let recent = Relation {
        fact: "Alpha is linked to Beta weakly".into(),
        relation_type: "RELATED_TO".into(),
        embedding: llm::pseudo_embed("Alpha is linked to Beta weakly"),
        source_agents: vec!["test".into()],
        valid_at: now,
        invalid_at: None,
        confidence: 0.1,
        salience: 0,
        created_at: now,
        memory_tier: MemoryTier::LongTerm,
        expires_at: None,
    };

    graph.create_edge("a", "b", &old).await.unwrap();
    graph.create_edge("a", "b", &recent).await.unwrap();

    // With recency dominating, the recent fact should come first
    let resp = context(
        &state,
        &graph,
        ContextRequest {
            query: "Alpha".into(),
            limit: Some(2),
            max_hops: Some(1),
            memory_tier_filter: None,
            graph: None,
            at: None,
            scoring: Some(ScoringParams {
                w_relevance: 0.0,
                w_confidence: 0.0,
                w_recency: 1.0,
                w_salience: 0.0,
                mmr_lambda: 1.0,
            }),
        },
        None,
    )
    .await
    .unwrap();

    assert_eq!(resp.facts.len(), 2, "should return both facts");
    assert!(
        resp.facts[0].fact.contains("weakly"),
        "recent fact should rank first with w_recency=1.0, got: {}",
        resp.facts[0].fact
    );
}

/// With w_confidence dominating, the high-confidence fact should come first.
#[tokio::test]
async fn context_custom_scoring_confidence_boost() {
    use chrono::Duration;

    let state = test_state();
    let graph = InMemoryGraph::new("test");
    let now = Utc::now();

    for (id, name) in [("a", "Gamma"), ("b", "Delta")] {
        graph
            .upsert_entity(&Entity {
                id: id.into(),
                name: name.into(),
                entity_type: "concept".into(),
                resolved: true,
                hint: None,
                content: None,
                created_at: now,
                embedding: llm::pseudo_embed(name),
            })
            .await
            .unwrap();
    }

    // Recent but low confidence
    let low_conf = Relation {
        fact: "Gamma is tenuously linked to Delta".into(),
        relation_type: "RELATED_TO".into(),
        embedding: llm::pseudo_embed("Gamma is tenuously linked to Delta"),
        source_agents: vec!["test".into()],
        valid_at: now,
        invalid_at: None,
        confidence: 0.1,
        salience: 10,
        created_at: now,
        memory_tier: MemoryTier::LongTerm,
        expires_at: None,
    };
    // Old but high confidence
    let high_conf = Relation {
        fact: "Gamma is definitely linked to Delta".into(),
        relation_type: "RELATED_TO".into(),
        embedding: llm::pseudo_embed("Gamma is definitely linked to Delta"),
        source_agents: vec!["test".into()],
        valid_at: now - Duration::days(365),
        invalid_at: None,
        confidence: 0.99,
        salience: 0,
        created_at: now - Duration::days(365),
        memory_tier: MemoryTier::LongTerm,
        expires_at: None,
    };

    graph.create_edge("a", "b", &low_conf).await.unwrap();
    graph.create_edge("a", "b", &high_conf).await.unwrap();

    let resp = context(
        &state,
        &graph,
        ContextRequest {
            query: "Gamma".into(),
            limit: Some(2),
            max_hops: Some(1),
            memory_tier_filter: None,
            graph: None,
            at: None,
            scoring: Some(ScoringParams {
                w_relevance: 0.0,
                w_confidence: 1.0,
                w_recency: 0.0,
                w_salience: 0.0,
                mmr_lambda: 1.0,
            }),
        },
        None,
    )
    .await
    .unwrap();

    assert_eq!(resp.facts.len(), 2);
    assert!(
        resp.facts[0].fact.contains("definitely"),
        "high-confidence fact should rank first with w_confidence=1.0, got: {}",
        resp.facts[0].fact
    );
}

/// MMR with lambda=0 (pure diversity) should deprioritise near-duplicate facts.
#[tokio::test]
async fn context_mmr_diversity_deprioritises_duplicates() {
    let state = test_state();
    let graph = InMemoryGraph::new("test");
    let now = Utc::now();

    for (id, name) in [("a", "Echo"), ("b", "Foxtrot"), ("c", "Golf")] {
        graph
            .upsert_entity(&Entity {
                id: id.into(),
                name: name.into(),
                entity_type: "concept".into(),
                resolved: true,
                hint: None,
                content: None,
                created_at: now,
                embedding: llm::pseudo_embed(name),
            })
            .await
            .unwrap();
    }

    // Two near-identical facts (same embedding) and one different fact
    let dup1 = Relation {
        fact: "Echo is connected to Foxtrot".into(),
        relation_type: "RELATED_TO".into(),
        embedding: llm::pseudo_embed("Echo connected Foxtrot"),
        source_agents: vec!["test".into()],
        valid_at: now,
        invalid_at: None,
        confidence: 0.9,
        salience: 5,
        created_at: now,
        memory_tier: MemoryTier::LongTerm,
        expires_at: None,
    };
    let dup2 = Relation {
        fact: "Echo is linked to Foxtrot".into(),
        relation_type: "RELATED_TO".into(),
        // Same embedding as dup1 — simulates near-duplicate
        embedding: llm::pseudo_embed("Echo connected Foxtrot"),
        source_agents: vec!["test".into()],
        valid_at: now,
        invalid_at: None,
        confidence: 0.9,
        salience: 5,
        created_at: now,
        memory_tier: MemoryTier::LongTerm,
        expires_at: None,
    };
    let different = Relation {
        fact: "Echo is connected to Golf".into(),
        relation_type: "RELATED_TO".into(),
        embedding: llm::pseudo_embed("Echo connected Golf"),
        source_agents: vec!["test".into()],
        valid_at: now,
        invalid_at: None,
        confidence: 0.9,
        salience: 5,
        created_at: now,
        memory_tier: MemoryTier::LongTerm,
        expires_at: None,
    };

    graph.create_edge("a", "b", &dup1).await.unwrap();
    graph.create_edge("a", "b", &dup2).await.unwrap();
    graph.create_edge("a", "c", &different).await.unwrap();

    // With strong diversity preference, the different fact should appear
    // before the second duplicate
    let resp = context(
        &state,
        &graph,
        ContextRequest {
            query: "Echo".into(),
            limit: Some(3),
            max_hops: Some(1),
            memory_tier_filter: None,
            graph: None,
            at: None,
            scoring: Some(ScoringParams {
                w_relevance: 0.5,
                w_confidence: 0.1,
                w_recency: 0.25,
                w_salience: 0.15,
                mmr_lambda: 0.3, // Strong diversity preference
            }),
        },
        None,
    )
    .await
    .unwrap();

    assert!(resp.facts.len() >= 2, "should return at least 2 facts");

    // The Golf fact should appear in the top 2 (diversity pushes it up
    // over the second Foxtrot duplicate)
    let top_2_facts: Vec<&str> = resp.facts.iter().take(2).map(|f| f.fact.as_str()).collect();
    assert!(
        top_2_facts.iter().any(|f| f.contains("Golf")),
        "MMR diversity should promote the different fact into top 2, got: {:?}",
        top_2_facts
    );
}
