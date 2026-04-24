use chrono::Utc;
use hippo::graph_backend::GraphBackend;
use hippo::models::{Entity, MemoryTier, Relation};
use hippo::sqlite_graph::SqliteGraph;

async fn setup() -> SqliteGraph {
    let graph = SqliteGraph::in_memory("test").expect("open in-memory sqlite");
    graph.setup_schema().await.expect("setup schema");
    graph
}

fn make_entity(id: &str, name: &str) -> Entity {
    Entity {
        id: id.to_string(),
        name: name.to_string(),
        entity_type: "person".to_string(),
        resolved: true,
        hint: None,
        content: None,
        created_at: Utc::now(),
        embedding: vec![1.0, 0.0, 0.0],
    }
}

fn make_relation(fact: &str) -> Relation {
    Relation {
        fact: fact.to_string(),
        relation_type: "knows".to_string(),
        embedding: vec![0.0, 1.0, 0.0],
        source_agents: vec!["test".to_string()],
        valid_at: Utc::now(),
        invalid_at: None,
        confidence: 0.9,
        salience: 1,
        created_at: Utc::now(),
        memory_tier: MemoryTier::Working,
        expires_at: None,
    }
}

#[tokio::test]
async fn ping_works() {
    let g = setup().await;
    g.ping().await.expect("ping");
}

#[tokio::test]
async fn upsert_and_get_entity() {
    let g = setup().await;
    let entity = make_entity("e1", "Alice");
    g.upsert_entity(&entity).await.unwrap();

    let found = g.get_entity_by_id("e1").await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "Alice");
}

#[tokio::test]
async fn fulltext_search_entities() {
    let g = setup().await;
    g.upsert_entity(&make_entity("e1", "Alice")).await.unwrap();
    g.upsert_entity(&make_entity("e2", "Bob")).await.unwrap();

    let results = g.fulltext_search_entities("ali").await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Alice");
}

#[tokio::test]
async fn create_edge_and_search() {
    let g = setup().await;
    g.upsert_entity(&make_entity("e1", "Alice")).await.unwrap();
    g.upsert_entity(&make_entity("e2", "Bob")).await.unwrap();

    let edge_id = g.create_edge("e1", "e2", &make_relation("Alice knows Bob")).await.unwrap();
    assert!(edge_id > 0);

    let edges = g.fulltext_search_edges("knows bob", None).await.unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].subject_name, "Alice");
    assert_eq!(edges[0].object_name, "Bob");
}

#[tokio::test]
async fn invalidate_edge() {
    let g = setup().await;
    g.upsert_entity(&make_entity("e1", "Alice")).await.unwrap();
    g.upsert_entity(&make_entity("e2", "Bob")).await.unwrap();
    let edge_id = g.create_edge("e1", "e2", &make_relation("Alice knows Bob")).await.unwrap();

    g.invalidate_edge(edge_id, Utc::now()).await.unwrap();

    let edges = g.fulltext_search_edges("knows bob", None).await.unwrap();
    assert!(edges.is_empty());
}

#[tokio::test]
async fn compound_confidence() {
    let g = setup().await;
    g.upsert_entity(&make_entity("e1", "A")).await.unwrap();
    g.upsert_entity(&make_entity("e2", "B")).await.unwrap();
    let edge_id = g.create_edge("e1", "e2", &make_relation("fact")).await.unwrap();

    let combined = g.compound_edge_confidence(edge_id, "agent2", 0.8).await.unwrap();
    // Bayesian: 1 - (1-0.9)(1-0.8) = 1 - 0.02 = 0.98
    assert!((combined - 0.98).abs() < 0.01);
}

#[tokio::test]
async fn walk_one_hop() {
    let g = setup().await;
    g.upsert_entity(&make_entity("e1", "A")).await.unwrap();
    g.upsert_entity(&make_entity("e2", "B")).await.unwrap();
    g.upsert_entity(&make_entity("e3", "C")).await.unwrap();
    g.create_edge("e1", "e2", &make_relation("A-B")).await.unwrap();
    g.create_edge("e2", "e3", &make_relation("B-C")).await.unwrap();

    let results = g.walk_n_hops(&["e1".to_string()], 1, 10, None).await.unwrap();
    let hops: Vec<_> = results.into_iter().map(|(e, _)| e).collect();
    assert_eq!(hops.len(), 1);
    assert_eq!(hops[0].fact, "A-B");
}

#[tokio::test]
async fn walk_n_hops() {
    let g = setup().await;
    g.upsert_entity(&make_entity("e1", "A")).await.unwrap();
    g.upsert_entity(&make_entity("e2", "B")).await.unwrap();
    g.upsert_entity(&make_entity("e3", "C")).await.unwrap();
    g.create_edge("e1", "e2", &make_relation("A-B")).await.unwrap();
    g.create_edge("e2", "e3", &make_relation("B-C")).await.unwrap();

    let results = g.walk_n_hops(&["e1".to_string()], 2, 10, None).await.unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].1, 1); // first hop
    assert_eq!(results[1].1, 2); // second hop
}

#[tokio::test]
async fn graph_stats() {
    let g = setup().await;
    g.upsert_entity(&make_entity("e1", "A")).await.unwrap();
    g.upsert_entity(&make_entity("e2", "B")).await.unwrap();
    g.create_edge("e1", "e2", &make_relation("fact")).await.unwrap();

    let stats = g.graph_stats().await.unwrap();
    assert_eq!(stats.entity_count, 2);
    assert_eq!(stats.edge_count, 1);
    assert!((stats.avg_confidence - 0.9).abs() < 0.01);
}

#[tokio::test]
async fn memory_tier_stats() {
    let g = setup().await;
    g.upsert_entity(&make_entity("e1", "A")).await.unwrap();
    g.upsert_entity(&make_entity("e2", "B")).await.unwrap();
    g.create_edge("e1", "e2", &make_relation("fact")).await.unwrap();

    let tier = g.memory_tier_stats().await.unwrap();
    assert_eq!(tier.working_count, 1);
    assert_eq!(tier.long_term_count, 0);
}

#[tokio::test]
async fn rename_entity() {
    let g = setup().await;
    g.upsert_entity(&make_entity("e1", "Alice")).await.unwrap();
    g.rename_entity("e1", "Alicia").await.unwrap();

    let found = g.get_entity_by_id("e1").await.unwrap().unwrap();
    assert_eq!(found.name, "Alicia");
}

#[tokio::test]
async fn entity_properties() {
    let g = setup().await;
    g.upsert_entity(&make_entity("e1", "Alice")).await.unwrap();
    g.set_entity_property("e1", "role", "engineer").await.unwrap();

    let found = g.find_entity_by_property("role", "engineer").await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, "e1");
}

#[tokio::test]
async fn drop_and_reinitialise() {
    let g = setup().await;
    g.upsert_entity(&make_entity("e1", "Alice")).await.unwrap();
    g.drop_and_reinitialise().await.unwrap();

    let entities = g.dump_all_entities().await.unwrap();
    assert!(entities.is_empty());
}

#[tokio::test]
async fn supersession_and_provenance() {
    let g = setup().await;
    g.upsert_entity(&make_entity("e1", "A")).await.unwrap();
    g.upsert_entity(&make_entity("e2", "B")).await.unwrap();
    let e1 = g.create_edge("e1", "e2", &make_relation("old fact")).await.unwrap();
    let e2 = g.create_edge("e1", "e2", &make_relation("new fact")).await.unwrap();

    g.create_supersession(e1, e2, Utc::now(), "old fact", "new fact").await.unwrap();

    let chain = g.get_supersession_chain(e1).await.unwrap();
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].new_edge_id, e2);

    let prov = g.get_provenance(e2).await.unwrap();
    assert_eq!(prov.supersedes.len(), 1);
}

#[tokio::test]
async fn source_credibility_round_trip() {
    use hippo::credibility::SourceCredibility;

    let g = setup().await;
    let cred = SourceCredibility {
        agent_id: "agent1".to_string(),
        credibility: 0.95,
        fact_count: 10,
        contradiction_rate: 0.05,
    };
    g.save_source_credibility(&cred).await.unwrap();

    let all = g.load_all_source_credibility().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].agent_id, "agent1");
    assert!((all[0].credibility - 0.95).abs() < 0.001);
}

#[tokio::test]
async fn vector_search_entities() {
    let g = setup().await;
    let mut alice = make_entity("e1", "Alice");
    alice.embedding = vec![1.0, 0.0, 0.0];
    let mut bob = make_entity("e2", "Bob");
    bob.embedding = vec![0.0, 1.0, 0.0];

    g.upsert_entity(&alice).await.unwrap();
    g.upsert_entity(&bob).await.unwrap();

    let results = g.vector_search_entities(&[1.0, 0.0, 0.0], 1).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0.name, "Alice");
}

#[tokio::test]
async fn list_entities_by_recency() {
    let g = setup().await;
    g.upsert_entity(&make_entity("e1", "Alice")).await.unwrap();
    g.upsert_entity(&make_entity("e2", "Bob")).await.unwrap();

    let page = g.list_entities_by_recency(0, 1).await.unwrap();
    assert_eq!(page.len(), 1);

    let all = g.list_entities_by_recency(0, 10).await.unwrap();
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn merge_placeholder() {
    let g = setup().await;
    let mut placeholder = make_entity("p1", "Unknown");
    placeholder.resolved = false;
    g.upsert_entity(&placeholder).await.unwrap();
    g.upsert_entity(&make_entity("e1", "Alice")).await.unwrap();
    g.upsert_entity(&make_entity("e2", "Bob")).await.unwrap();
    g.create_edge("p1", "e2", &make_relation("placeholder-Bob")).await.unwrap();

    g.merge_placeholder("p1", "e1").await.unwrap();

    // Placeholder entity should be gone
    assert!(g.get_entity_by_id("p1").await.unwrap().is_none());
    // Edge should now point to e1
    let edges = g.find_all_active_edges_from("e1").await.unwrap();
    assert_eq!(edges.len(), 1);
}
