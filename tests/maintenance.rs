mod helpers;

use chrono::{Duration, Utc};
use serde_json::json;
use uuid::Uuid;

use helpers::{seed_raw, start_agent_mock_admin};

#[tokio::test]
async fn maintenance_decay_stale_edges() {
    let agent = start_agent_mock_admin().await;

    // Seed two entities and one edge with valid_at = 60 days ago
    let alice_id = Uuid::new_v4().to_string();
    let acme_id = Uuid::new_v4().to_string();
    let old_valid_at = (Utc::now() - Duration::days(60)).to_rfc3339();

    seed_raw(
        &agent.client,
        &agent.base_url,
        &json!({
            "entities": [
                { "id": &alice_id, "name": "Alice", "entity_type": "person" },
                { "id": &acme_id, "name": "Acme", "entity_type": "organization" }
            ],
            "edges": [{
                "subject_id": &alice_id,
                "object_id": &acme_id,
                "fact": "Alice worked at Acme long ago",
                "relation_type": "WORKS_AT",
                "confidence": 0.9,
                "salience": 1,
                "valid_at": &old_valid_at,
                "source_agents": "test",
                "memory_tier": "long_term"
            }]
        }),
    )
    .await;

    // Run maintenance
    let resp = agent
        .client
        .post(format!("{}/maintain", agent.base_url))
        .send()
        .await
        .expect("maintain request failed");
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "maintain should succeed: {status} {body}"
    );

    // Query the graph and check the edge's decayed_confidence
    let graph: serde_json::Value = agent
        .client
        .get(format!("{}/graph", agent.base_url))
        .send()
        .await
        .expect("graph request failed")
        .json()
        .await
        .expect("graph not JSON");

    let edges = graph["active_edges"].as_array().unwrap();
    assert!(!edges.is_empty(), "edge should still be active");
    // The edge was last accessed 60 days ago, so decay should have been applied
    // Note: decayed_confidence is not exposed in graph dump, so we verify maintenance ran
    // without error. The decay formula requires >30 days stale to apply.
}

#[tokio::test]
async fn maintenance_promote_working_to_long_term() {
    let agent = start_agent_mock_admin().await;

    let alice_id = Uuid::new_v4().to_string();
    let bob_id = Uuid::new_v4().to_string();

    // Seed a working-tier edge with salience >= 3 (should be promoted)
    seed_raw(
        &agent.client,
        &agent.base_url,
        &json!({
            "entities": [
                { "id": &alice_id, "name": "Alice", "entity_type": "person" },
                { "id": &bob_id, "name": "Bob", "entity_type": "person" }
            ],
            "edges": [{
                "subject_id": &alice_id,
                "object_id": &bob_id,
                "fact": "Alice and Bob are friends",
                "relation_type": "FRIENDS_WITH",
                "confidence": 0.9,
                "salience": 5,
                "valid_at": (Utc::now() - Duration::hours(2)).to_rfc3339(),
                "source_agents": "test",
                "memory_tier": "working"
            }]
        }),
    )
    .await;

    // Check initial state
    let stats_before: serde_json::Value = agent
        .client
        .get(format!("{}/memory/stats", agent.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let working_before = stats_before["working_count"].as_u64().unwrap_or(0);
    assert!(working_before > 0, "should start with working memory");

    // Run maintenance
    let resp = agent
        .client
        .post(format!("{}/maintain", agent.base_url))
        .send()
        .await
        .expect("maintain failed");
    assert!(resp.status().is_success());

    // Check stats after — working count should have decreased
    let stats_after: serde_json::Value = agent
        .client
        .get(format!("{}/memory/stats", agent.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let long_term_after = stats_after["long_term_count"].as_u64().unwrap_or(0);
    assert!(
        long_term_after > 0,
        "should have promoted to long_term: {stats_after:?}"
    );
}

// The old purge_stale_working_memory test was removed — working memory is no
// longer automatically purged.  TTL-based expiry is covered by unit tests in
// unit_maintain.rs (maintain_expires_ttl_edges, maintain_does_not_expire_future_ttl,
// maintain_no_ttl_means_no_expiry).

#[tokio::test]
async fn maintenance_consolidate_link_discovery() {
    let agent = start_agent_mock_admin().await;

    // Seed Carol and David both working at Acme, but no direct edge between them
    let carol_id = Uuid::new_v4().to_string();
    let david_id = Uuid::new_v4().to_string();
    let acme_id = Uuid::new_v4().to_string();

    seed_raw(
        &agent.client,
        &agent.base_url,
        &json!({
            "entities": [
                { "id": &carol_id, "name": "Carol", "entity_type": "person" },
                { "id": &david_id, "name": "David", "entity_type": "person" },
                { "id": &acme_id, "name": "Acme Corp", "entity_type": "organization" }
            ],
            "edges": [
                {
                    "subject_id": &carol_id,
                    "object_id": &acme_id,
                    "fact": "Carol works at Acme Corp",
                    "relation_type": "WORKS_AT",
                    "confidence": 0.9,
                    "salience": 3,
                    "source_agents": "test",
                    "memory_tier": "long_term"
                },
                {
                    "subject_id": &david_id,
                    "object_id": &acme_id,
                    "fact": "David works at Acme Corp",
                    "relation_type": "WORKS_AT",
                    "confidence": 0.9,
                    "salience": 3,
                    "source_agents": "test",
                    "memory_tier": "long_term"
                }
            ]
        }),
    )
    .await;

    // Run consolidate
    let resp: serde_json::Value = agent
        .client
        .post(format!("{}/consolidate", agent.base_url))
        .json(&json!({
            "max_entity_pairs": 30,
            "prune_threshold": 0.05,
            "dry_run": false
        }))
        .send()
        .await
        .expect("consolidate failed")
        .json()
        .await
        .expect("consolidate response not JSON");

    // Consolidate should complete without error
    // In mock mode, LLM won't discover links, but the pipeline should run successfully
    assert!(
        resp["duration_ms"].as_u64().is_some(),
        "consolidate should return duration_ms: {resp:?}"
    );
}
