/// Scenario-based integration evals that simulate real agent memory patterns.
///
/// Unlike the unit evals in evals.rs which test individual features, these
/// scenarios run multi-phase "life histories" that exercise the system as a
/// whole: contradiction handling, temporal queries, multi-hop retrieval,
/// reflection, timeline, and multi-agent confidence compounding.
///
/// Run with: cargo test scenario_ -- --nocapture --test-threads=1
mod helpers;

use helpers::{any_fact_contains, fact_strings, query_facts, query_graph, remember, start_agent};
use std::time::Duration;

// ---- Scenario 1: The Career Journey ----

#[tokio::test]
#[ignore = "spawns the hippo binary; run with `cargo test --test scenarios -- --ignored`"]
async fn scenario_career_journey() {
    let agent = start_agent().await;
    let (client, base) = (&agent.client, agent.base_url.as_str());

    println!("\n=== SCENARIO: Career Journey ===");

    // PHASE 1: University
    remember(client, base, "Sarah graduated from MIT with a CS degree").await;
    remember(client, base, "Sarah is 22 years old").await;

    let _t_after_uni = chrono::Utc::now();
    tokio::time::sleep(Duration::from_secs(2)).await;

    // PHASE 2: First job
    remember(
        client,
        base,
        "Sarah works as a junior developer at TechStart",
    )
    .await;
    remember(client, base, "Sarah earns 60000 dollars per year").await;
    remember(client, base, "Sarah lives in New York").await;

    let t_after_first_job = chrono::Utc::now();
    tokio::time::sleep(Duration::from_secs(2)).await;

    // PHASE 3: Career change (contradicts previous job)
    remember(
        client,
        base,
        "Sarah was promoted to senior engineer at TechStart",
    )
    .await;
    remember(client, base, "Sarah now earns 95000 dollars per year").await;

    tokio::time::sleep(Duration::from_secs(1)).await;

    // ASSERTION 1: Current salary should be 95000, not 60000
    let salary_facts = query_facts(client, base, "Sarah salary", 10).await;
    let salary_texts = fact_strings(&salary_facts);
    assert!(
        any_fact_contains(&salary_texts, "95000"),
        "FAIL: Current salary 95k not found. Got: {:?}",
        salary_texts
    );
    assert!(
        !any_fact_contains(&salary_texts, "60000"),
        "FAIL: Old salary 60k still present. Got: {:?}",
        salary_texts
    );
    println!("  ✓ Salary update: 60k→95k contradiction handled");

    // ASSERTION 2: Temporal query at t_after_first_job should show 60000
    let temporal_resp: serde_json::Value = client
        .post(helpers::api_url(base, "/context/temporal"))
        .json(&serde_json::json!({
            "query": "Sarah salary",
            "at": t_after_first_job.to_rfc3339(),
            "limit": 10
        }))
        .send()
        .await
        .expect("temporal failed")
        .json()
        .await
        .expect("temporal json failed");

    let temporal_facts = temporal_resp["facts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let temporal_texts: Vec<String> = temporal_facts
        .iter()
        .map(|f| f["fact"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(
        any_fact_contains(&temporal_texts, "60000"),
        "FAIL: Temporal query should show 60k at t_after_first_job. Got: {:?}",
        temporal_texts
    );
    println!("  ✓ Temporal query: correctly showed 60k at earlier time");

    // ASSERTION 3: Timeline shows career progression
    let timeline: serde_json::Value = client
        .get(helpers::api_url(base, "/timeline/Sarah"))
        .send()
        .await
        .expect("timeline failed")
        .json()
        .await
        .expect("timeline json failed");
    let events = timeline["events"].as_array().cloned().unwrap_or_default();
    assert!(
        events.len() >= 4,
        "FAIL: timeline should have ≥4 events. Got: {}",
        events.len()
    );
    println!("  ✓ Timeline: {} career events recorded", events.len());

    // ASSERTION 4: Reflect shows education + career known, gaps elsewhere
    let reflect: serde_json::Value = client
        .post(helpers::api_url(base, "/reflect"))
        .json(&serde_json::json!({"about": "Sarah", "suggest_questions": false}))
        .send()
        .await
        .expect("reflect failed")
        .json()
        .await
        .expect("reflect json failed");
    let known = reflect["known"].as_array().cloned().unwrap_or_default();
    assert!(
        !known.is_empty(),
        "FAIL: reflect should show known facts for Sarah"
    );
    let gaps = reflect["gaps"].as_array().cloned().unwrap_or_default();
    assert!(!gaps.is_empty(), "FAIL: reflect should identify gaps");
    println!(
        "  ✓ Reflect: {} known facts, {} gaps identified",
        known.len(),
        gaps.len()
    );

    println!("PASS: scenario_career_journey");
}

// ---- Scenario 2: The Doctor-Patient Relationship ----

#[tokio::test]
#[ignore = "spawns the hippo binary; run with `cargo test --test scenarios -- --ignored`"]
async fn scenario_medical_knowledge() {
    let agent = start_agent().await;
    let (client, base) = (&agent.client, agent.base_url.as_str());

    println!("\n=== SCENARIO: Medical Knowledge ===");

    // Initial patient knowledge
    remember(client, base, "James is a patient at City Medical").await;
    remember(
        client,
        base,
        "James was diagnosed with hypertension in 2020",
    )
    .await;
    remember(
        client,
        base,
        "James is prescribed lisinopril 10mg by Dr. Chen",
    )
    .await;
    remember(client, base, "Dr. Chen is a cardiologist at City Medical").await;

    tokio::time::sleep(Duration::from_secs(1)).await;

    // Update: medication changed (contradicts lisinopril)
    remember(
        client,
        base,
        "James is now prescribed amlodipine 5mg after adverse reaction to lisinopril",
    )
    .await;

    tokio::time::sleep(Duration::from_secs(1)).await;

    // ASSERTION 1: Current medication is amlodipine, NOT lisinopril
    let med_facts = query_facts(client, base, "James medication prescription", 10).await;
    let med_texts = fact_strings(&med_facts);
    assert!(
        any_fact_contains(&med_texts, "amlodipine"),
        "FAIL: New medication amlodipine not found. Got: {:?}",
        med_texts
    );
    assert!(
        !any_fact_contains(&med_texts, "lisinopril"),
        "FAIL: Old medication lisinopril still present. Got: {:?}",
        med_texts
    );
    println!("  ✓ Medication update: lisinopril→amlodipine contradiction handled");

    // ASSERTION 2: Multi-hop: James → City Medical → Dr. Chen (2 hops)
    let context_resp: serde_json::Value = client
        .post(helpers::api_url(base, "/context"))
        .json(
            &serde_json::json!({"query": "James doctor cardiologist", "limit": 15, "max_hops": 2}),
        )
        .send()
        .await
        .expect("context failed")
        .json()
        .await
        .expect("context json failed");
    let context_facts = context_resp["facts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let context_texts: Vec<String> = context_facts
        .iter()
        .map(|f| f["fact"].as_str().unwrap_or("").to_string())
        .collect();
    let has_dr_chen = any_fact_contains(&context_texts, "Chen")
        || any_fact_contains(&context_texts, "cardiologist");
    assert!(
        has_dr_chen,
        "FAIL: Dr. Chen not found via multi-hop. Got: {:?}",
        context_texts
    );
    println!("  ✓ Multi-hop: Dr. Chen found via James→City Medical→Dr. Chen");

    // ASSERTION 3: Provenance shows lisinopril was superseded
    let graph = query_graph(client, base).await;
    let invalidated = graph["invalidated_edges"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let lisi_invalidated = invalidated.iter().any(|e| {
        e["fact"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("lisinopril")
    });
    assert!(
        lisi_invalidated,
        "FAIL: lisinopril edge not invalidated. Invalidated: {:?}",
        invalidated
    );
    println!("  ✓ Provenance: lisinopril edge correctly invalidated");

    println!("PASS: scenario_medical_knowledge");
}

// ---- Scenario 3: Multi-Agent Knowledge Building ----

#[tokio::test]
#[ignore = "spawns the hippo binary; run with `cargo test --test scenarios -- --ignored`"]
async fn scenario_multi_agent_knowledge() {
    let agent = start_agent().await;
    let (client, base) = (&agent.client, agent.base_url.as_str());

    println!("\n=== SCENARIO: Multi-Agent Knowledge Building ===");

    // Agent 1: Financial agent
    let _resp1: serde_json::Value = client
        .post(helpers::api_url(base, "/remember"))
        .json(&serde_json::json!({"statement": "Emma is the CFO of Acme Corp", "source_agent": "finance-agent"}))
        .send()
        .await
        .expect("r1 fail")
        .json()
        .await
        .expect("r1 json fail");

    // Agent 2: HR agent (corroborates same fact)
    let _resp2: serde_json::Value = client
        .post(helpers::api_url(base, "/remember"))
        .json(&serde_json::json!({"statement": "Emma is the Chief Financial Officer of Acme Corp", "source_agent": "hr-agent"}))
        .send()
        .await
        .expect("r2 fail")
        .json()
        .await
        .expect("r2 json fail");

    // Agent 3: News agent (high credibility hint)
    let _resp3: serde_json::Value = client
        .post(helpers::api_url(base, "/remember"))
        .json(&serde_json::json!({
            "statement": "Emma joined Acme Corp as CFO in 2022",
            "source_agent": "news-agent",
            "source_credibility_hint": 0.95
        }))
        .send()
        .await
        .expect("r3 fail")
        .json()
        .await
        .expect("r3 json fail");

    tokio::time::sleep(Duration::from_secs(1)).await;

    // ASSERTION 1: Confidence should be compounded (both agents agree Emma is CFO)
    let emma_facts = query_facts(client, base, "Emma CFO Acme", 10).await;
    let cfo_fact = emma_facts.iter().find(|f| {
        let fact_text = f["fact"].as_str().unwrap_or("").to_lowercase();
        fact_text.contains("cfo") || fact_text.contains("financial")
    });
    if let Some(f) = cfo_fact {
        let confidence = f["confidence"].as_f64().unwrap_or(0.0);
        let source_agents = f["source_agents"].as_array().cloned().unwrap_or_default();
        println!(
            "  ✓ CFO fact confidence: {:.3} (from {} agents)",
            confidence,
            source_agents.len()
        );
        assert!(
            confidence > 0.75,
            "FAIL: Compounded confidence should be >0.75. Got {:.3}",
            confidence
        );
    } else {
        panic!(
            "FAIL: CFO fact not found for Emma. Facts: {:?}",
            fact_strings(&emma_facts)
        );
    }

    // ASSERTION 2: Sources endpoint shows all 3 agents
    let sources: serde_json::Value = client
        .get(helpers::api_url(base, "/sources"))
        .send()
        .await
        .expect("sources fail")
        .json()
        .await
        .expect("sources json fail");
    let source_list = sources["sources"].as_array().cloned().unwrap_or_default();
    let agent_ids: Vec<&str> = source_list
        .iter()
        .filter_map(|s| s["agent_id"].as_str())
        .collect();
    assert!(
        agent_ids.contains(&"finance-agent"),
        "FAIL: finance-agent not in sources"
    );
    assert!(
        agent_ids.contains(&"hr-agent"),
        "FAIL: hr-agent not in sources"
    );
    assert!(
        agent_ids.contains(&"news-agent"),
        "FAIL: news-agent not in sources"
    );
    println!("  ✓ Sources: all 3 agents tracked");

    // ASSERTION 3: News agent has higher credibility (due to hint)
    let news_source = source_list
        .iter()
        .find(|s| s["agent_id"].as_str() == Some("news-agent"));
    if let Some(ns) = news_source {
        let cred = ns["credibility"].as_f64().unwrap_or(0.0);
        println!("  ✓ News agent credibility: {:.3}", cred);
        assert!(
            cred >= 0.85,
            "FAIL: News agent credibility should be ≥0.85 (hint was 0.95). Got {:.3}",
            cred
        );
    }

    println!("PASS: scenario_multi_agent_knowledge");
}
