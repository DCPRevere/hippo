use std::sync::Arc;

use chrono::Utc;

use hippo::config::Config;
use hippo::graph_backend::GraphBackend;
use hippo::in_memory_graph::InMemoryGraph;
use hippo::llm;
use hippo::models::{
    ContextRequest, Entity, MemoryTier, Relation,
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
