/// Eval tests for hippo.
///
/// Split into focused test functions per eval category. Each starts its own
/// agent, loads facts concurrently, and runs its specific checks with timing
/// and diagnostic output on failure.
mod helpers;

use std::time::{Duration, Instant};

use helpers::{
    any_fact_contains, diagnose, fact_strings, load_base_facts, print_summary, query_facts,
    query_graph, remember, run_eval_case, start_agent, EvalCase, EvalResult,
};

// ---- Retrieval Precision ----

#[tokio::test]
async fn test_eval_retrieval_precision() {
    let agent = start_agent().await;
    let (client, base) = (&agent.client, agent.base_url.as_str());

    println!("\n============================================================");
    println!("EVAL: Retrieval Precision");
    println!("============================================================\n");

    load_base_facts(client, base).await;

    let cases = vec![
        EvalCase {
            name: "wedding",
            query: "wedding",
            limit: 5,
            must_contain: vec!["wedding", "married"],
            must_not_contain: vec!["metformin", "savings", "mortgage", "Vanguard"],
        },
        EvalCase {
            name: "doctor",
            query: "doctor",
            limit: 5,
            must_contain: vec!["doctor", "Dr."],
            must_not_contain: vec!["wedding", "savings", "mortgage", "Vanguard"],
        },
        EvalCase {
            name: "bank_account",
            query: "bank account",
            limit: 5,
            must_contain: vec!["bank", "account", "savings"],
            must_not_contain: vec!["wedding", "doctor", "metformin"],
        },
        EvalCase {
            name: "alice",
            query: "Alice",
            limit: 10,
            must_contain: vec!["Alice"],
            must_not_contain: vec![],
        },
        EvalCase {
            name: "david_carol",
            query: "David Carol",
            limit: 8,
            must_contain: vec!["David", "Carol"],
            must_not_contain: vec!["metformin", "savings", "mortgage"],
        },
    ];

    println!("\n--- Retrieval Precision ---");
    let mut results = Vec::new();
    for case in &cases {
        let r = run_eval_case(client, base, case).await;
        r.print();
        results.push(r);
    }

    let (passed, total) = print_summary(&results);
    assert!(passed == total, "{} eval(s) failed out of {total}", total - passed);
}

// ---- Contradiction Handling ----

#[tokio::test]
async fn test_eval_contradiction() {
    let agent = start_agent().await;
    let (client, base) = (&agent.client, agent.base_url.as_str());

    println!("\n============================================================");
    println!("EVAL: Contradiction Handling");
    println!("============================================================\n");

    load_base_facts(client, base).await;

    let mut results = Vec::new();

    // Before contradiction
    let start = Instant::now();
    let raw_before = query_facts(client, base, "my doctor", 5).await;
    let facts_before = fact_strings(&raw_before);
    let has_smith = any_fact_contains(&facts_before, "Smith");

    let mut failures = Vec::new();
    if !has_smith {
        failures.push("expected Dr. Smith before contradiction".to_string());
        print_diagnose_on_failure(client, base, "my doctor", &raw_before).await;
    }
    results.push(EvalResult {
        name: "contradiction[before]".to_string(),
        passed: failures.is_empty(),
        failures,
        facts_returned: facts_before,
        elapsed: start.elapsed(),
    });
    results.last().unwrap().print();

    // Apply contradiction
    remember(client, base, "My doctor is now Dr. Jones at Riverside Clinic").await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    // After contradiction
    let start = Instant::now();
    let raw_after = query_facts(client, base, "my doctor", 5).await;
    let facts_after = fact_strings(&raw_after);
    let has_jones = any_fact_contains(&facts_after, "Jones");
    let still_has_smith_as_doctor = facts_after.iter().any(|f| {
        let lower = f.to_lowercase();
        lower.contains("smith") && lower.contains("doctor")
    });

    let mut failures = Vec::new();
    if !has_jones {
        failures.push("expected Dr. Jones after contradiction".to_string());
    }
    if still_has_smith_as_doctor {
        failures.push("Dr. Smith should no longer appear as 'my doctor'".to_string());
    }
    if !failures.is_empty() {
        print_diagnose_on_failure(client, base, "my doctor", &raw_after).await;
    }
    results.push(EvalResult {
        name: "contradiction[after]".to_string(),
        passed: failures.is_empty(),
        failures,
        facts_returned: facts_after,
        elapsed: start.elapsed(),
    });
    results.last().unwrap().print();

    // Verify via graph that old edge is invalidated
    let start = Instant::now();
    let graph = query_graph(client, base).await;
    let invalidated = graph["invalidated_edges"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let smith_invalidated = invalidated.iter().any(|e| {
        let fact = e["fact"].as_str().unwrap_or("");
        fact.to_lowercase().contains("smith") && fact.to_lowercase().contains("doctor")
    });
    let mut failures = Vec::new();
    if !smith_invalidated {
        failures.push("expected Dr. Smith doctor edge to be invalidated in graph".to_string());
    }
    results.push(EvalResult {
        name: "contradiction[graph_state]".to_string(),
        passed: failures.is_empty(),
        failures,
        facts_returned: vec![],
        elapsed: start.elapsed(),
    });
    results.last().unwrap().print();

    let (passed, total) = print_summary(&results);
    assert!(passed == total, "{} eval(s) failed out of {total}", total - passed);
}

// ---- Entity Resolution ----

#[tokio::test]
async fn test_eval_entity_resolution() {
    let agent = start_agent().await;
    let (client, base) = (&agent.client, agent.base_url.as_str());

    println!("\n============================================================");
    println!("EVAL: Entity Resolution");
    println!("============================================================\n");

    load_base_facts(client, base).await;

    let start = Instant::now();
    let graph = query_graph(client, base).await;
    let entity_array = graph["entities"].as_array().cloned().unwrap_or_default();
    let entities: Vec<&str> = entity_array
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();

    let mut results = Vec::new();

    // Alice should be one entity, not duplicated
    let alice_count = entities.iter().filter(|e| e.to_lowercase() == "alice").count();
    let mut failures = Vec::new();
    if alice_count != 1 {
        failures.push(format!("expected exactly 1 'Alice' entity, found {alice_count}"));
        println!("    all entities: {:?}", entities);
    }
    results.push(EvalResult {
        name: "entity_resolution[alice_unique]".to_string(),
        passed: failures.is_empty(),
        failures,
        facts_returned: vec![],
        elapsed: start.elapsed(),
    });

    // Bob should be one entity
    let bob_count = entities.iter().filter(|e| e.to_lowercase() == "bob").count();
    let mut failures = Vec::new();
    if bob_count != 1 {
        failures.push(format!("expected exactly 1 'Bob' entity, found {bob_count}"));
        println!("    all entities: {:?}", entities);
    }
    results.push(EvalResult {
        name: "entity_resolution[bob_unique]".to_string(),
        passed: failures.is_empty(),
        failures,
        facts_returned: vec![],
        elapsed: start.elapsed(),
    });

    // Dr. Smith should be one entity
    let smith_count = entities
        .iter()
        .filter(|e| {
            let lower = e.to_lowercase();
            lower == "dr. smith" || lower == "dr smith"
        })
        .count();
    let mut failures = Vec::new();
    if smith_count != 1 {
        failures.push(format!("expected exactly 1 'Dr. Smith' entity, found {smith_count}"));
        println!("    all entities: {:?}", entities);
    }
    results.push(EvalResult {
        name: "entity_resolution[dr_smith_unique]".to_string(),
        passed: failures.is_empty(),
        failures,
        facts_returned: vec![],
        elapsed: start.elapsed(),
    });

    // The speaker/principal should resolve to a single entity
    let principal_variants: Vec<&&str> = entities
        .iter()
        .filter(|e| {
            let lower = e.to_lowercase();
            lower == "principal"
                || lower == "the principal"
                || lower == "speaker"
                || lower == "the speaker"
                || lower == "i"
                || lower == "me"
                || lower == "user"
        })
        .collect();
    let mut failures = Vec::new();
    if principal_variants.len() > 1 {
        failures.push(format!(
            "expected 1 principal entity, found {}: {:?}",
            principal_variants.len(),
            principal_variants
        ));
    }
    results.push(EvalResult {
        name: "entity_resolution[principal_unique]".to_string(),
        passed: failures.is_empty(),
        failures,
        facts_returned: vec![],
        elapsed: start.elapsed(),
    });

    for r in &results {
        r.print();
    }

    let (passed, total) = print_summary(&results);
    assert!(passed == total, "{} eval(s) failed out of {total}", total - passed);
}

// ---- Graph Quality ----

#[tokio::test]
async fn test_eval_graph_quality() {
    let agent = start_agent().await;
    let (client, base) = (&agent.client, agent.base_url.as_str());

    println!("\n============================================================");
    println!("EVAL: Graph Quality");
    println!("============================================================\n");

    load_base_facts(client, base).await;

    let start = Instant::now();
    let graph = query_graph(client, base).await;
    let entities = graph["entities"].as_array().cloned().unwrap_or_default();
    let active_edges = graph["active_edges"].as_array().cloned().unwrap_or_default();

    let mut results = Vec::new();

    // Entity count should be reasonable
    let mut failures = Vec::new();
    if entities.len() > 40 {
        failures.push(format!(
            "too many entities: {} (expected <= 40 for 21 input statements)",
            entities.len()
        ));
    }
    if entities.len() < 10 {
        failures.push(format!(
            "too few entities: {} (expected >= 10 for 21 input statements)",
            entities.len()
        ));
    }
    results.push(EvalResult {
        name: "graph_quality[entity_count]".to_string(),
        passed: failures.is_empty(),
        failures,
        facts_returned: vec![],
        elapsed: start.elapsed(),
    });

    // Should have a reasonable number of active edges
    let mut failures = Vec::new();
    if active_edges.len() < 20 {
        failures.push(format!(
            "too few active edges: {} (expected >= 20)",
            active_edges.len()
        ));
    }
    results.push(EvalResult {
        name: "graph_quality[edge_count]".to_string(),
        passed: failures.is_empty(),
        failures,
        facts_returned: vec![],
        elapsed: start.elapsed(),
    });

    // Check for concept pollution
    let concept_entities: Vec<&str> = entities
        .iter()
        .filter_map(|e| {
            let name = e["name"].as_str()?;
            let etype = e["entity_type"].as_str().unwrap_or("");
            if etype == "concept" {
                Some(name)
            } else {
                None
            }
        })
        .collect();
    let mut failures = Vec::new();
    if concept_entities.len() > 10 {
        failures.push(format!(
            "too many concept entities ({}): {:?} — consider if these should be edge properties instead",
            concept_entities.len(),
            concept_entities
        ));
    }
    results.push(EvalResult {
        name: "graph_quality[concept_pollution]".to_string(),
        passed: failures.is_empty(),
        failures,
        facts_returned: vec![],
        elapsed: start.elapsed(),
    });

    for r in &results {
        r.print();
    }

    let (passed, total) = print_summary(&results);
    assert!(passed == total, "{} eval(s) failed out of {total}", total - passed);
}

// ---- Helpers local to this file ----

async fn print_diagnose_on_failure(
    client: &reqwest::Client,
    base_url: &str,
    query: &str,
    raw_facts: &[serde_json::Value],
) {
    println!("    --- returned facts with scores ---");
    for f in raw_facts {
        println!(
            "      fact={:?}  confidence={}  salience={}",
            f["fact"].as_str().unwrap_or("?"),
            f["confidence"],
            f["salience"],
        );
    }
    println!("    --- diagnose pipeline for {:?} ---", query);
    let diag = diagnose(client, base_url, query).await;
    if let Some(steps) = diag["steps"].as_array() {
        for step in steps {
            let name = step["step"].as_str().unwrap_or("?");
            let desc = step["description"].as_str().unwrap_or("");
            let count = step["results"].as_array().map_or(0, |a| a.len());
            println!("      [{name}] {desc} ({count} results)");
        }
    }
}
