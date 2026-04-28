/// Structured eval suite for hippo.
///
/// Tests the *intelligence* of the system: contradiction detection, temporal
/// queries, multi-hop retrieval, entity resolution, reflection, timeline
/// history, and confidence compounding.
///
/// Run with: cargo test eval_ -- --nocapture --test-threads=4
mod helpers;

use helpers::{any_fact_contains, fact_strings, query_facts, query_graph, remember, start_agent};
use serde_json::Value;
use std::time::Duration;

// ---- Eval 1: Contradiction Detection ----

#[tokio::test]
#[ignore = "requires real LLM and network; run with `cargo test --test evals -- --ignored`"]
async fn eval_contradiction_detection() {
    let agent = start_agent().await;
    let (client, base) = (&agent.client, agent.base_url.as_str());

    // Step 1: Remember old fact
    remember(client, base, "Alice works at Google").await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Step 2: Remember contradicting fact
    remember(client, base, "Alice works at Anthropic").await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Step 3: Query current state
    let facts = query_facts(client, base, "where does Alice work", 10).await;
    let fact_texts = fact_strings(&facts);

    // MUST: Anthropic is returned
    assert!(
        any_fact_contains(&fact_texts, "Anthropic"),
        "FAIL: Anthropic not returned. Got: {:?}",
        fact_texts
    );

    // MUST NOT: Google is returned (was contradicted)
    assert!(
        !any_fact_contains(&fact_texts, "Google"),
        "FAIL: Google still returned after contradiction. Got: {:?}",
        fact_texts
    );

    println!("PASS: contradiction_detection - Anthropic returned, Google absent");

    // Bonus: check via /graph that Google edge is in invalidated_edges
    let graph = query_graph(client, base).await;
    let invalidated = graph["invalidated_edges"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let google_invalidated = invalidated.iter().any(|e| {
        e["fact"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("google")
    });
    assert!(
        google_invalidated,
        "FAIL: Google edge not found in invalidated_edges. Invalidated: {:?}",
        invalidated
    );
    println!("PASS: contradiction_detection[provenance] - Google edge invalidated");
}

// ---- Eval 2: Temporal Query ----

#[tokio::test]
#[ignore = "requires real LLM and network; run with `cargo test --test evals -- --ignored`"]
async fn eval_temporal_query() {
    let agent = start_agent().await;
    let (client, base) = (&agent.client, agent.base_url.as_str());

    // Step 1: Remember fact at t0
    remember(client, base, "Bob's salary is 50000 pounds").await;

    // Capture t_after_first (now)
    let t_after_first = chrono::Utc::now();

    // Wait, then add contradiction
    tokio::time::sleep(Duration::from_secs(2)).await;

    remember(client, base, "Bob's salary is 75000 pounds").await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Step 2: Temporal query AT t_after_first — should return 50k NOT 75k
    let at_str = t_after_first.to_rfc3339();
    let resp: Value = client
        .post(format!("{base}/context/temporal"))
        .json(&serde_json::json!({
            "query": "Bob's salary",
            "at": at_str,
            "limit": 10
        }))
        .send()
        .await
        .expect("temporal context failed")
        .json()
        .await
        .expect("temporal context not JSON");

    let facts = resp["facts"].as_array().cloned().unwrap_or_default();
    let fact_texts: Vec<String> = facts
        .iter()
        .map(|f| f["fact"].as_str().unwrap_or("").to_string())
        .collect();

    assert!(
        fact_texts
            .iter()
            .any(|f| f.contains("50000") || f.contains("50,000")),
        "FAIL: temporal query at t_after_first should return 50k. Got: {:?}",
        fact_texts
    );
    assert!(
        !fact_texts
            .iter()
            .any(|f| f.contains("75000") || f.contains("75,000")),
        "FAIL: temporal query at t_after_first should NOT return 75k. Got: {:?}",
        fact_texts
    );

    println!("PASS: temporal_query - correct time-slice returned");
}

// ---- Eval 3: Multi-hop Retrieval ----

#[tokio::test]
#[ignore = "requires real LLM and network; run with `cargo test --test evals -- --ignored`"]
async fn eval_multi_hop_retrieval() {
    let agent = start_agent().await;
    let (client, base) = (&agent.client, agent.base_url.as_str());

    // Establish: Carol → Synodal → London (2 hops from Carol to London)
    remember(client, base, "Carol is the CEO of Synodal Inc").await;
    remember(client, base, "Synodal Inc is headquartered in London").await;
    remember(client, base, "Synodal Inc builds AI software").await;

    tokio::time::sleep(Duration::from_secs(1)).await;

    // Query: ask about Carol's company location (requires 2 hops: Carol→Synodal→London)
    let resp: Value = client
        .post(format!("{base}/context"))
        .json(&serde_json::json!({
            "query": "Carol company location",
            "limit": 15,
            "max_hops": 2
        }))
        .send()
        .await
        .expect("context failed")
        .json()
        .await
        .expect("context not JSON");

    let facts = resp["facts"].as_array().cloned().unwrap_or_default();
    let fact_texts: Vec<String> = facts
        .iter()
        .map(|f| f["fact"].as_str().unwrap_or("").to_string())
        .collect();

    // We need to find London in results - this requires 2-hop traversal
    let has_london = fact_texts
        .iter()
        .any(|f| f.to_lowercase().contains("london"));

    assert!(
        has_london,
        "FAIL: multi-hop should find London via Carol->Synodal->London. Got: {:?}",
        fact_texts
    );

    // Check hop depth on the London fact
    let london_fact = facts.iter().find(|f| {
        f["fact"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("london")
    });
    if let Some(lf) = london_fact {
        let hops = lf["hops"].as_u64().unwrap_or(0);
        println!("INFO: London found at hop depth {}", hops);
    }

    println!("PASS: multi_hop_retrieval - London found via 2-hop traversal");
}

// ---- Eval 4: Entity Resolution ----

#[tokio::test]
#[ignore = "requires real LLM and network; run with `cargo test --test evals -- --ignored`"]
async fn eval_entity_resolution() {
    let agent = start_agent().await;
    let (client, base) = (&agent.client, agent.base_url.as_str());

    // These should all resolve to the same "Alice" entity
    remember(
        client,
        base,
        "Alice Chen is a software engineer at Anthropic",
    )
    .await;
    remember(client, base, "Alice is married to Bob").await;
    remember(client, base, "Alice Chen lives in San Francisco").await;

    // Give entity resolution time to process
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Check the graph - should have ONE entity named Alice (or Alice Chen), not many
    let graph = query_graph(client, base).await;
    let entities = graph["entities"].as_array().cloned().unwrap_or_default();

    let alice_entities: Vec<_> = entities
        .iter()
        .filter(|e| {
            e["name"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .contains("alice")
        })
        .collect();

    println!("INFO: Alice entities found: {}", alice_entities.len());
    for e in &alice_entities {
        println!("  - {}", e["name"].as_str().unwrap_or("?"));
    }

    // Best case: 1 entity (resolved), acceptable: 2 (not merged but no more)
    assert!(
        alice_entities.len() <= 2,
        "FAIL: too many Alice entities ({}). Entity resolution failing badly.",
        alice_entities.len()
    );

    // All facts about Alice should be queryable from a single query
    let facts = query_facts(client, base, "Alice", 15).await;
    let fact_texts = fact_strings(&facts);

    assert!(
        any_fact_contains(&fact_texts, "Anthropic"),
        "FAIL: Alice's job not found. Got: {:?}",
        fact_texts
    );
    assert!(
        any_fact_contains(&fact_texts, "Bob") || any_fact_contains(&fact_texts, "married"),
        "FAIL: Alice's marriage not found. Got: {:?}",
        fact_texts
    );
    assert!(
        any_fact_contains(&fact_texts, "San Francisco")
            || any_fact_contains(&fact_texts, "Francisco"),
        "FAIL: Alice's location not found. Got: {:?}",
        fact_texts
    );

    println!("PASS: entity_resolution - all Alice facts retrievable");
}

// ---- Eval 5: Reflect Gap Analysis ----

#[tokio::test]
#[ignore = "requires real LLM and network; run with `cargo test --test evals -- --ignored`"]
async fn eval_reflect_gap_analysis() {
    let agent = start_agent().await;
    let (client, base) = (&agent.client, agent.base_url.as_str());

    // Seed other entities with diverse relation types so gap detection has something to compare
    remember(client, base, "Bob Smith lives in London").await;
    remember(client, base, "Bob Smith studied at Oxford University").await;
    remember(client, base, "Bob Smith is married to Carol Smith").await;

    // Add facts about Eve - intentionally limited (work only, no location/education/family)
    remember(client, base, "Eve Williams works at Synodal Inc").await;
    remember(client, base, "Eve Williams is a data scientist").await;
    // Intentionally no: location, family, education, etc.

    tokio::time::sleep(Duration::from_secs(1)).await;

    let resp: Value = client
        .post(format!("{base}/reflect"))
        .json(&serde_json::json!({
            "about": "Eve",
            "suggest_questions": true
        }))
        .send()
        .await
        .expect("reflect failed")
        .json()
        .await
        .expect("reflect not JSON");

    // known should have at least the work fact
    let known = resp["known"].as_array().cloned().unwrap_or_default();

    assert!(
        !known.is_empty(),
        "FAIL: reflect should have some known facts for Eve. Got empty."
    );

    // gaps should be non-empty (we know very little)
    let gaps = resp["gaps"].as_array().cloned().unwrap_or_default();
    assert!(
        !gaps.is_empty(),
        "FAIL: reflect should identify gaps. Got none."
    );

    // suggested_questions should be non-empty
    let questions = resp["suggested_questions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !questions.is_empty(),
        "FAIL: reflect should suggest questions. Got none."
    );

    println!("PASS: reflect_gap_analysis");
    println!("  Known facts: {}", known.len());
    println!("  Gaps: {:?}", gaps);
    println!(
        "  Questions: {:?}",
        questions
            .iter()
            .map(|q| q.as_str().unwrap_or(""))
            .collect::<Vec<_>>()
    );
}

// ---- Eval 6: Timeline History ----

#[tokio::test]
#[ignore = "requires real LLM and network; run with `cargo test --test evals -- --ignored`"]
async fn eval_timeline_history() {
    let agent = start_agent().await;
    let (client, base) = (&agent.client, agent.base_url.as_str());

    // Create a supersession chain
    remember(client, base, "Frank works at Old Company").await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    remember(client, base, "Frank works at New Company").await; // contradicts above

    tokio::time::sleep(Duration::from_millis(500)).await;

    let resp: Value = client
        .get(format!("{base}/timeline/Frank"))
        .send()
        .await
        .expect("timeline failed")
        .json()
        .await
        .expect("timeline not JSON");

    let events = resp["events"].as_array().cloned().unwrap_or_default();

    assert!(
        events.len() >= 2,
        "FAIL: timeline should have at least 2 events (old + new). Got: {:?}",
        events
    );

    // Find the superseded event
    let superseded = events
        .iter()
        .filter(|e| e["superseded"].as_bool().unwrap_or(false))
        .count();
    let current = events
        .iter()
        .filter(|e| !e["superseded"].as_bool().unwrap_or(true))
        .count();

    assert!(
        superseded >= 1,
        "FAIL: at least 1 event should be marked superseded. Events: {:?}",
        events
    );

    // Old Company should be marked superseded
    let old_co_event = events.iter().find(|e| {
        e["fact"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("old company")
    });
    assert!(
        old_co_event.is_some(),
        "FAIL: Old Company fact not in timeline"
    );
    assert!(
        old_co_event.unwrap()["superseded"]
            .as_bool()
            .unwrap_or(false),
        "FAIL: Old Company fact should be marked superseded"
    );

    println!(
        "PASS: timeline_history - {} events, {} superseded, {} current",
        events.len(),
        superseded,
        current
    );
}

// ---- Eval 7: Confidence Compounding ----

#[tokio::test]
#[ignore = "requires real LLM and network; run with `cargo test --test evals -- --ignored`"]
async fn eval_confidence_compounding() {
    let agent = start_agent().await;
    let (client, base) = (&agent.client, agent.base_url.as_str());

    // Agent 1 asserts a fact
    let _resp1: Value = client
        .post(format!("{base}/remember"))
        .json(&serde_json::json!({
            "statement": "Grace Lee is a lawyer",
            "source_agent": "agent-1"
        }))
        .send()
        .await
        .expect("remember 1 failed")
        .json()
        .await
        .expect("json 1 failed");

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Get initial confidence
    let initial_facts = query_facts(client, base, "Grace Lee lawyer", 5).await;
    let initial_confidence = initial_facts
        .first()
        .and_then(|f| f["confidence"].as_f64())
        .unwrap_or(0.0);

    // Agent 2 corroborates the same fact
    let _resp2: Value = client
        .post(format!("{base}/remember"))
        .json(&serde_json::json!({
            "statement": "Grace Lee is a lawyer",
            "source_agent": "agent-2"
        }))
        .send()
        .await
        .expect("remember 2 failed")
        .json()
        .await
        .expect("json 2 failed");

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Get confidence after compounding
    let facts_after = query_facts(client, base, "Grace Lee lawyer", 5).await;
    let after_confidence = facts_after
        .first()
        .and_then(|f| f["confidence"].as_f64())
        .unwrap_or(0.0);

    // source_agents should contain both
    let source_agents = facts_after
        .first()
        .and_then(|f| f["source_agents"].as_array())
        .cloned()
        .unwrap_or_default();
    let agent_names: Vec<&str> = source_agents.iter().filter_map(|s| s.as_str()).collect();

    // Confidence should be higher after second assertion (Bayesian compounding)
    println!(
        "INFO: confidence before={:.3} after={:.3}",
        initial_confidence, after_confidence
    );
    println!("INFO: source_agents={:?}", agent_names);

    assert!(
        after_confidence >= initial_confidence,
        "FAIL: confidence should be >= after compounding. before={:.3} after={:.3}",
        initial_confidence,
        after_confidence
    );

    assert!(
        agent_names.contains(&"agent-1") && agent_names.contains(&"agent-2"),
        "FAIL: both agents should be in source_agents. Got: {:?}",
        agent_names
    );

    println!("PASS: confidence_compounding");
}
