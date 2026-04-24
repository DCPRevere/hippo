use std::collections::HashMap;
use std::sync::Arc;

use hippo::config::Config;
use hippo::in_memory_graph::InMemoryGraph;
use hippo::models::{Entity, GraphOp, MemoryTier, OperationsResult, Relation, RememberRequest};
use hippo::pipeline::remember::remember;
use hippo::state::AppState;
use hippo::testing::FakeLlm;

fn test_state(fake: FakeLlm) -> Arc<AppState> {
    Arc::new(AppState::for_test(Arc::new(fake), Config::test_default()))
}

fn make_remember_req(statement: &str) -> RememberRequest {
    RememberRequest {
        statement: statement.to_string(),
        source_agent: Some("test-agent".to_string()),
        source_credibility_hint: None,
        graph: None,
        ttl_secs: None,
    }
}

// ---- Basic entity and edge creation ----

#[tokio::test]
async fn remember_creates_entities_and_edges() {
    let fake = FakeLlm::new().with_operations(OperationsResult {
        operations: vec![
            GraphOp::CreateNode {
                node_ref: Some("n1".into()),
                name: "Alice".into(),
                node_type: "person".into(),
                properties: HashMap::new(),
            },
            GraphOp::CreateNode {
                node_ref: Some("n2".into()),
                name: "Acme".into(),
                node_type: "organization".into(),
                properties: HashMap::new(),
            },
            GraphOp::CreateEdge {
                from: "n1".into(),
                to: "n2".into(),
                relation: "WORKS_AT".into(),
                fact: "Alice works at Acme".into(),
                confidence: 0.9,
            },
        ],
    });

    let state = test_state(fake);
    let graph = InMemoryGraph::new("test");

    let resp = remember(
        &state,
        &graph,
        make_remember_req("Alice works at Acme"),
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(resp.entities_created, 2, "should create Alice and Acme");
    assert_eq!(resp.facts_written, 1, "should write one edge");

    // Verify usage tracking: 1 LLM call (extract_operations) + 3 embeds (2 nodes + 1 edge)
    assert_eq!(resp.usage.llm_calls, 1, "one extract_operations call");
    assert_eq!(resp.usage.embed_calls, 3, "2 entity embeds + 1 edge embed");

    // Verify graph state
    let entities = graph.dump_all_entities().await.unwrap();
    assert_eq!(entities.len(), 2);
    let names: Vec<&str> = entities.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"Alice"));
    assert!(names.contains(&"Acme"));

    let edges = graph.dump_all_edges().await.unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].fact, "Alice works at Acme");
    assert_eq!(edges[0].relation_type, "WORKS_AT");
    assert_eq!(
        edges[0].memory_tier, "working",
        "new edges start as Working"
    );
}

use hippo::graph_backend::GraphBackend;

// ---- Existing entity reuse ----

#[tokio::test]
async fn remember_reuses_existing_entities() {
    let fake = FakeLlm::new().with_operations(OperationsResult {
        operations: vec![
            GraphOp::CreateNode {
                node_ref: Some("n1".into()),
                name: "Alice".into(),
                node_type: "person".into(),
                properties: HashMap::new(),
            },
            GraphOp::CreateEdge {
                from: "n1".into(),
                to: "n1".into(),
                relation: "IS_A".into(),
                fact: "Alice is a person".into(),
                confidence: 0.9,
            },
        ],
    });

    let state = test_state(fake);
    let graph = InMemoryGraph::new("test");

    // Pre-seed Alice
    let alice = Entity {
        id: "alice-id".into(),
        name: "Alice".into(),
        entity_type: "person".into(),
        resolved: true,
        hint: None,
        content: None,
        created_at: chrono::Utc::now(),
        embedding: hippo::llm::pseudo_embed("Alice"),
    };
    graph.upsert_entity(&alice).await.unwrap();

    let resp = remember(
        &state,
        &graph,
        make_remember_req("Alice is a person"),
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(resp.entities_created, 0, "should reuse existing Alice");
    let entities = graph.dump_all_entities().await.unwrap();
    assert_eq!(entities.len(), 1, "still just one entity");
}

// ---- Contradiction via InvalidateEdge ----

#[tokio::test]
async fn remember_invalidates_edge_by_id() {
    let _state = test_state(FakeLlm::new());
    let graph = InMemoryGraph::new("test");

    // Seed Alice and London with an edge
    let alice = Entity {
        id: "alice".into(),
        name: "Alice".into(),
        entity_type: "person".into(),
        resolved: true,
        hint: None,
        content: None,
        created_at: chrono::Utc::now(),
        embedding: hippo::llm::pseudo_embed("Alice"),
    };
    let london = Entity {
        id: "london".into(),
        name: "London".into(),
        entity_type: "place".into(),
        resolved: true,
        hint: None,
        content: None,
        created_at: chrono::Utc::now(),
        embedding: hippo::llm::pseudo_embed("London"),
    };
    graph.upsert_entity(&alice).await.unwrap();
    graph.upsert_entity(&london).await.unwrap();

    let rel = Relation {
        fact: "Alice lives in London".into(),
        relation_type: "LIVES_IN".into(),
        embedding: hippo::llm::pseudo_embed("Alice lives in London"),
        source_agents: vec!["test".into()],
        valid_at: chrono::Utc::now(),
        invalid_at: None,
        confidence: 0.9,
        salience: 1,
        created_at: chrono::Utc::now(),
        memory_tier: MemoryTier::LongTerm,
        expires_at: None,
    };
    let edge_id = graph.create_edge("alice", "london", &rel).await.unwrap();

    // Now send a remember that invalidates the old edge and creates a new one.
    // The LLM must emit CreateNode for every entity referenced in edges so the
    // pipeline can resolve node references (it populates known_ids from ops).
    let fake = FakeLlm::new().with_operations(OperationsResult {
        operations: vec![
            GraphOp::InvalidateEdge {
                edge_id: Some(edge_id),
                fact: None,
                reason: "Alice moved to Edinburgh".into(),
            },
            GraphOp::CreateNode {
                node_ref: Some("n0".into()),
                name: "Alice".into(),
                node_type: "person".into(),
                properties: HashMap::new(),
            },
            GraphOp::CreateNode {
                node_ref: Some("n1".into()),
                name: "Edinburgh".into(),
                node_type: "place".into(),
                properties: HashMap::new(),
            },
            GraphOp::CreateEdge {
                from: "n0".into(),
                to: "n1".into(),
                relation: "LIVES_IN".into(),
                fact: "Alice lives in Edinburgh".into(),
                confidence: 0.9,
            },
        ],
    });
    // Replace state with new fake
    let state = test_state(fake);

    let resp = remember(
        &state,
        &graph,
        make_remember_req("Alice lives in Edinburgh"),
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(resp.contradictions_invalidated, 1);
    assert_eq!(resp.facts_written, 1);
    assert_eq!(resp.entities_created, 1);

    // The old edge should be invalidated
    let all_edges = graph.dump_all_edges().await.unwrap();
    let old_edge = all_edges.iter().find(|e| e.edge_id == edge_id).unwrap();
    assert!(
        old_edge.invalid_at.is_some(),
        "old edge should be invalidated"
    );

    // The new edge should be active
    let new_edge = all_edges
        .iter()
        .find(|e| e.fact == "Alice lives in Edinburgh")
        .unwrap();
    assert!(new_edge.invalid_at.is_none(), "new edge should be active");
}

// ---- Working memory tier ----

#[tokio::test]
async fn remember_new_edges_are_working_tier() {
    let fake = FakeLlm::new().with_operations(OperationsResult {
        operations: vec![
            GraphOp::CreateNode {
                node_ref: Some("n1".into()),
                name: "Bob".into(),
                node_type: "person".into(),
                properties: HashMap::new(),
            },
            GraphOp::CreateNode {
                node_ref: Some("n2".into()),
                name: "Beta".into(),
                node_type: "organization".into(),
                properties: HashMap::new(),
            },
            GraphOp::CreateEdge {
                from: "n1".into(),
                to: "n2".into(),
                relation: "WORKS_AT".into(),
                fact: "Bob works at Beta".into(),
                confidence: 0.8,
            },
        ],
    });

    let state = test_state(fake);
    let graph = InMemoryGraph::new("test");

    remember(
        &state,
        &graph,
        make_remember_req("Bob works at Beta"),
        None,
        None,
    )
    .await
    .unwrap();

    let tier = graph.memory_tier_stats().await.unwrap();
    assert_eq!(tier.working_count, 1, "new edge should be Working tier");
    assert_eq!(tier.long_term_count, 0);
}

// ---- Source tracking ----

#[tokio::test]
async fn remember_tracks_source_agent() {
    let fake = FakeLlm::new().with_operations(OperationsResult {
        operations: vec![
            GraphOp::CreateNode {
                node_ref: Some("n1".into()),
                name: "Carol".into(),
                node_type: "person".into(),
                properties: HashMap::new(),
            },
            GraphOp::CreateNode {
                node_ref: Some("n2".into()),
                name: "Gamma".into(),
                node_type: "organization".into(),
                properties: HashMap::new(),
            },
            GraphOp::CreateEdge {
                from: "n1".into(),
                to: "n2".into(),
                relation: "WORKS_AT".into(),
                fact: "Carol works at Gamma".into(),
                confidence: 0.85,
            },
        ],
    });

    let state = test_state(fake);
    let graph = InMemoryGraph::new("test");

    let mut req = make_remember_req("Carol works at Gamma");
    req.source_agent = Some("finance-agent".into());
    remember(&state, &graph, req, None, None).await.unwrap();

    let edges = graph.dump_all_edges().await.unwrap();
    assert_eq!(edges.len(), 1);
    assert!(
        edges[0].source_agents.contains("finance-agent"),
        "source_agents should contain the agent: {}",
        edges[0].source_agents
    );
}

// ---- Duplicate detection ----

#[tokio::test]
async fn remember_skips_duplicate_edges_by_embedding() {
    let graph = InMemoryGraph::new("test");

    // Seed entities + an existing edge with the same embedding
    let alice = Entity {
        id: "alice".into(),
        name: "Alice".into(),
        entity_type: "person".into(),
        resolved: true,
        hint: None,
        content: None,
        created_at: chrono::Utc::now(),
        embedding: hippo::llm::pseudo_embed("Alice"),
    };
    let bob = Entity {
        id: "bob".into(),
        name: "Bob".into(),
        entity_type: "person".into(),
        resolved: true,
        hint: None,
        content: None,
        created_at: chrono::Utc::now(),
        embedding: hippo::llm::pseudo_embed("Bob"),
    };
    graph.upsert_entity(&alice).await.unwrap();
    graph.upsert_entity(&bob).await.unwrap();

    // pseudo_embed is deterministic — same text → same vector
    let rel = Relation {
        fact: "Alice knows Bob".into(),
        relation_type: "KNOWS".into(),
        embedding: hippo::llm::pseudo_embed("Alice knows Bob"),
        source_agents: vec!["test".into()],
        valid_at: chrono::Utc::now(),
        invalid_at: None,
        confidence: 0.9,
        salience: 1,
        created_at: chrono::Utc::now(),
        memory_tier: MemoryTier::LongTerm,
        expires_at: None,
    };
    graph.create_edge("alice", "bob", &rel).await.unwrap();

    // Now try to remember the same fact — LLM returns CreateEdge with same text
    let fake = FakeLlm::new().with_operations(OperationsResult {
        operations: vec![GraphOp::CreateEdge {
            from: "Alice".into(),
            to: "Bob".into(),
            relation: "KNOWS".into(),
            fact: "Alice knows Bob".into(),
            confidence: 0.9,
        }],
    });
    let state = test_state(fake);

    let resp = remember(
        &state,
        &graph,
        make_remember_req("Alice knows Bob"),
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(resp.facts_written, 0, "duplicate should be skipped");
    let all = graph.dump_all_edges().await.unwrap();
    assert_eq!(all.len(), 1, "still just one edge");
}

// ---- Credibility weighting ----

#[tokio::test]
async fn remember_applies_credibility_to_confidence() {
    let fake = FakeLlm::new().with_operations(OperationsResult {
        operations: vec![
            GraphOp::CreateNode {
                node_ref: Some("n1".into()),
                name: "Dave".into(),
                node_type: "person".into(),
                properties: HashMap::new(),
            },
            GraphOp::CreateNode {
                node_ref: Some("n2".into()),
                name: "Delta".into(),
                node_type: "organization".into(),
                properties: HashMap::new(),
            },
            GraphOp::CreateEdge {
                from: "n1".into(),
                to: "n2".into(),
                relation: "WORKS_AT".into(),
                fact: "Dave works at Delta".into(),
                confidence: 1.0, // LLM says full confidence
            },
        ],
    });

    let state = test_state(fake);
    let graph = InMemoryGraph::new("test");

    // Set low credibility for this agent
    {
        let mut cred = state.credibility.write().await;
        cred.hydrate(vec![hippo::credibility::SourceCredibility {
            agent_id: "low-trust-agent".into(),
            credibility: 0.5,
            fact_count: 10,
            contradiction_rate: 0.3,
        }]);
    }

    let mut req = make_remember_req("Dave works at Delta");
    req.source_agent = Some("low-trust-agent".into());
    remember(&state, &graph, req, None, None).await.unwrap();

    let edges = graph.dump_all_edges().await.unwrap();
    assert_eq!(edges.len(), 1);
    // confidence should be 1.0 * 0.5 = 0.5
    assert!(
        (edges[0].confidence - 0.5).abs() < 0.1,
        "confidence should be weighted by credibility: got {}",
        edges[0].confidence
    );
}

// ---- UpdateNode ----

#[tokio::test]
async fn remember_update_node_sets_properties() {
    let graph = InMemoryGraph::new("test");

    // Seed Alice
    let alice = Entity {
        id: "alice".into(),
        name: "Alice".into(),
        entity_type: "person".into(),
        resolved: true,
        hint: None,
        content: None,
        created_at: chrono::Utc::now(),
        embedding: hippo::llm::pseudo_embed("Alice"),
    };
    graph.upsert_entity(&alice).await.unwrap();

    let fake = FakeLlm::new().with_operations(OperationsResult {
        operations: vec![GraphOp::UpdateNode {
            id: "alice".into(),
            set: [("nationality".into(), "British".into())]
                .into_iter()
                .collect(),
        }],
    });
    let state = test_state(fake);

    let resp = remember(
        &state,
        &graph,
        make_remember_req("Alice is British"),
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(
        resp.entities_resolved, 1,
        "UpdateNode should count as resolved"
    );
}

// ---- Unresolved edge references ----

#[tokio::test]
async fn remember_skips_edge_with_unresolved_refs() {
    let fake = FakeLlm::new().with_operations(OperationsResult {
        operations: vec![GraphOp::CreateEdge {
            from: "nonexistent-1".into(),
            to: "nonexistent-2".into(),
            relation: "KNOWS".into(),
            fact: "Ghost knows Phantom".into(),
            confidence: 0.9,
        }],
    });
    let state = test_state(fake);
    let graph = InMemoryGraph::new("test");

    let resp = remember(
        &state,
        &graph,
        make_remember_req("Ghost knows Phantom"),
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(
        resp.facts_written, 0,
        "unresolved refs should skip the edge"
    );
    assert_eq!(resp.trace.execution[0].outcome, "skipped");
}

// ---- Entity properties ----

#[tokio::test]
async fn remember_creates_entity_with_properties() {
    let fake = FakeLlm::new().with_operations(OperationsResult {
        operations: vec![GraphOp::CreateNode {
            node_ref: Some("n1".into()),
            name: "Eve".into(),
            node_type: "person".into(),
            properties: [("role".into(), "engineer".into())].into_iter().collect(),
        }],
    });
    let state = test_state(fake);
    let graph = InMemoryGraph::new("test");

    let resp = remember(
        &state,
        &graph,
        make_remember_req("Eve is an engineer"),
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(resp.entities_created, 1);
    let entities = graph.dump_all_entities().await.unwrap();
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].name, "Eve");
}

// ---- Trace includes original operations ----

#[tokio::test]
async fn remember_trace_includes_operations() {
    let fake = FakeLlm::new().with_operations(OperationsResult {
        operations: vec![
            GraphOp::CreateNode {
                node_ref: Some("n1".into()),
                name: "Frank".into(),
                node_type: "person".into(),
                properties: HashMap::new(),
            },
            GraphOp::CreateNode {
                node_ref: Some("n2".into()),
                name: "Omega".into(),
                node_type: "organization".into(),
                properties: HashMap::new(),
            },
            GraphOp::CreateEdge {
                from: "n1".into(),
                to: "n2".into(),
                relation: "WORKS_AT".into(),
                fact: "Frank works at Omega".into(),
                confidence: 0.9,
            },
        ],
    });
    let state = test_state(fake);
    let graph = InMemoryGraph::new("test");

    let resp = remember(
        &state,
        &graph,
        make_remember_req("Frank works at Omega"),
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(
        resp.trace.operations.len(),
        3,
        "trace should include all LLM operations"
    );
    assert!(
        resp.trace.revised_operations.is_none(),
        "no revision without enrichment"
    );
    assert_eq!(resp.trace.execution.len(), 3, "execution trace for each op");
}
