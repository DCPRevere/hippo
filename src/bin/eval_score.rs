use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use reqwest::Client;
use serde_json::Value;

struct EvalResult {
    name: &'static str,
    passed: bool,
    score: f32,
    details: String,
    elapsed: Duration,
}

struct TestAgent {
    child: Child,
    base_url: String,
    client: Client,
    graph_name: String,
}

impl Drop for TestAgent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = Command::new("docker")
            .args([
                "exec",
                "hippo-falkordb-1",
                "redis-cli",
                "GRAPH.DELETE",
                &self.graph_name,
            ])
            .output();
    }
}

fn find_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

async fn start_agent() -> TestAgent {
    let port = find_free_port();
    let base_url = format!("http://localhost:{port}");
    let graph_name = format!("hippo_eval_{port}");

    let _ = Command::new("docker")
        .args([
            "exec",
            "hippo-falkordb-1",
            "redis-cli",
            "GRAPH.DELETE",
            &graph_name,
        ])
        .output();

    let bin = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("hippo");

    let oauth = std::env::var("ANTHROPIC_OAUTH_TOKEN")
        .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
        .expect("ANTHROPIC_OAUTH_TOKEN or ANTHROPIC_API_KEY must be set for eval-score");

    let child = Command::new(&bin)
        .env("ANTHROPIC_OAUTH_TOKEN", &oauth)
        .env("PORT", port.to_string())
        .env("GRAPH_NAME", &graph_name)
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to start hippo at {}: {e}", bin.display()));

    let agent = TestAgent {
        child,
        base_url: base_url.clone(),
        client: Client::new(),
        graph_name,
    };

    for _ in 0..30 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if let Ok(r) = agent.client.get(format!("{base_url}/health")).send().await {
            if r.status().is_success() {
                return agent;
            }
        }
    }
    panic!("hippo did not become healthy in time (port {port})");
}

async fn remember(client: &Client, base: &str, statement: &str) {
    let resp = client
        .post(format!("{base}/remember"))
        .json(&serde_json::json!({ "statement": statement, "source_agent": "eval" }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("remember '{statement}' failed: {e}"));
    assert!(
        resp.status().is_success(),
        "remember failed for: {statement}"
    );
    let _: Value = resp.json().await.expect("remember response not JSON");
}

async fn remember_with_agent(client: &Client, base: &str, statement: &str, source: &str) {
    let resp = client
        .post(format!("{base}/remember"))
        .json(&serde_json::json!({ "statement": statement, "source_agent": source }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("remember '{statement}' failed: {e}"));
    assert!(
        resp.status().is_success(),
        "remember failed for: {statement}"
    );
    let _: Value = resp.json().await.expect("remember response not JSON");
}

async fn query_facts(client: &Client, base: &str, q: &str, limit: usize) -> Vec<Value> {
    let resp: Value = client
        .post(format!("{base}/context"))
        .json(&serde_json::json!({ "query": q, "limit": limit }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("context '{q}' failed: {e}"))
        .json()
        .await
        .expect("context response not JSON");
    resp["facts"].as_array().cloned().unwrap_or_default()
}

fn fact_strings(facts: &[Value]) -> Vec<String> {
    facts
        .iter()
        .map(|f| f["fact"].as_str().unwrap_or("").to_string())
        .collect()
}

fn any_contains(facts: &[String], kw: &str) -> bool {
    let kw = kw.to_lowercase();
    facts.iter().any(|f| f.to_lowercase().contains(&kw))
}

async fn query_graph(client: &Client, base: &str) -> Value {
    client
        .get(format!("{base}/graph"))
        .send()
        .await
        .expect("graph request failed")
        .json()
        .await
        .expect("graph response not JSON")
}

// --- Eval implementations ---

async fn eval_contradiction_detection(client: &Client, base: &str) -> EvalResult {
    let start = Instant::now();
    remember(client, base, "Alice works at Google").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    remember(client, base, "Alice works at Anthropic").await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let facts = query_facts(client, base, "where does Alice work", 10).await;
    let texts = fact_strings(&facts);
    let has_anthropic = any_contains(&texts, "Anthropic");
    let has_google = any_contains(&texts, "Google");

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

    let (passed, score, details) = if has_anthropic && !has_google && google_invalidated {
        (
            true,
            1.0,
            "Google invalidated, Anthropic returned".to_string(),
        )
    } else if has_anthropic && !has_google {
        (
            true,
            0.8,
            "Anthropic returned, Google absent but not in invalidated_edges".to_string(),
        )
    } else if has_anthropic {
        (
            false,
            0.5,
            format!(
                "Anthropic returned but Google still present. Facts: {:?}",
                texts
            ),
        )
    } else {
        (
            false,
            0.0,
            format!("Anthropic not returned. Facts: {:?}", texts),
        )
    };

    EvalResult {
        name: "contradiction_detection",
        passed,
        score,
        details,
        elapsed: start.elapsed(),
    }
}

async fn eval_temporal_query(client: &Client, base: &str) -> EvalResult {
    let start = Instant::now();
    remember(client, base, "Bob's salary is 50000 pounds").await;
    let t_after_first = chrono::Utc::now();
    tokio::time::sleep(Duration::from_secs(2)).await;
    remember(client, base, "Bob's salary is 75000 pounds").await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let at_str = t_after_first.to_rfc3339();
    let resp: Value = client
        .post(format!("{base}/context/temporal"))
        .json(&serde_json::json!({ "query": "Bob's salary", "at": at_str, "limit": 10 }))
        .send()
        .await
        .expect("temporal context failed")
        .json()
        .await
        .expect("temporal context not JSON");

    let facts = resp["facts"].as_array().cloned().unwrap_or_default();
    let texts: Vec<String> = facts
        .iter()
        .map(|f| f["fact"].as_str().unwrap_or("").to_string())
        .collect();
    let has_50k = texts
        .iter()
        .any(|f| f.contains("50000") || f.contains("50,000"));
    let has_75k = texts
        .iter()
        .any(|f| f.contains("75000") || f.contains("75,000"));

    let (passed, score, details) = if has_50k && !has_75k {
        (
            true,
            1.0,
            "Old salary returned at past timestamp".to_string(),
        )
    } else if has_50k {
        (
            false,
            0.5,
            format!("50k found but 75k also present. Facts: {:?}", texts),
        )
    } else {
        (false, 0.0, format!("50k not found. Facts: {:?}", texts))
    };

    EvalResult {
        name: "temporal_query",
        passed,
        score,
        details,
        elapsed: start.elapsed(),
    }
}

async fn eval_multi_hop_retrieval(client: &Client, base: &str) -> EvalResult {
    let start = Instant::now();
    remember(client, base, "Carol is the CEO of Synodal Inc").await;
    remember(client, base, "Synodal Inc is headquartered in London").await;
    remember(client, base, "Synodal Inc builds AI software").await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let resp: Value = client
        .post(format!("{base}/context"))
        .json(
            &serde_json::json!({ "query": "Carol company location", "limit": 15, "max_hops": 2 }),
        )
        .send()
        .await
        .expect("context failed")
        .json()
        .await
        .expect("context not JSON");

    let facts = resp["facts"].as_array().cloned().unwrap_or_default();
    let texts = fact_strings(&facts);
    let has_london = any_contains(&texts, "london");

    let london_hops = facts
        .iter()
        .find(|f| {
            f["fact"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .contains("london")
        })
        .and_then(|f| f["hops"].as_u64());

    let (passed, score, details) = if has_london {
        if london_hops == Some(2) {
            (
                true,
                1.0,
                "London found via 2-hop traversal".to_string(),
            )
        } else {
            let hop_detail =
                london_hops.map_or("unknown hops".to_string(), |h| format!("at hop {h}"));
            (
                true,
                0.5,
                format!("London found {hop_detail} (expected hop 2)"),
            )
        }
    } else {
        (
            false,
            0.0,
            format!(
                "London not found via Carol->Synodal->London\nContext returned: {:?}",
                texts
            ),
        )
    };

    EvalResult {
        name: "multi_hop_retrieval",
        passed,
        score,
        details,
        elapsed: start.elapsed(),
    }
}

async fn eval_entity_resolution(client: &Client, base: &str) -> EvalResult {
    let start = Instant::now();
    remember(client, base, "Alice Chen is a software engineer at Anthropic").await;
    remember(client, base, "Alice is married to Bob").await;
    remember(client, base, "Alice Chen lives in San Francisco").await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let graph = query_graph(client, base).await;
    let entities = graph["entities"].as_array().cloned().unwrap_or_default();
    let alice_count = entities
        .iter()
        .filter(|e| {
            e["name"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .contains("alice")
        })
        .count();

    let facts = query_facts(client, base, "Alice", 15).await;
    let texts = fact_strings(&facts);
    let has_anthropic = any_contains(&texts, "Anthropic");
    let has_bob = any_contains(&texts, "Bob") || any_contains(&texts, "married");
    let has_sf = any_contains(&texts, "San Francisco") || any_contains(&texts, "Francisco");
    let facts_found = [has_anthropic, has_bob, has_sf]
        .iter()
        .filter(|&&b| b)
        .count();

    let (passed, score, details) = match alice_count {
        1 if facts_found == 3 => (
            true,
            1.0,
            "1 Alice entity, all facts found".to_string(),
        ),
        1 => (
            true,
            0.8,
            format!("1 Alice entity, {facts_found}/3 facts found"),
        ),
        2 if facts_found == 3 => (
            true,
            0.5,
            "2 Alice entities (not merged), all facts found".to_string(),
        ),
        2 => (
            false,
            0.3,
            format!("2 Alice entities, {facts_found}/3 facts found"),
        ),
        n => (
            false,
            0.0,
            format!("{n} Alice entities — resolution failing badly"),
        ),
    };

    EvalResult {
        name: "entity_resolution",
        passed,
        score,
        details,
        elapsed: start.elapsed(),
    }
}

async fn eval_reflect_gap_analysis(client: &Client, base: &str) -> EvalResult {
    let start = Instant::now();
    remember(client, base, "Eve Williams works at Synodal Inc").await;
    remember(client, base, "Eve Williams is a data scientist").await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let resp: Value = client
        .post(format!("{base}/reflect"))
        .json(&serde_json::json!({ "about": "Eve", "suggest_questions": true }))
        .send()
        .await
        .expect("reflect failed")
        .json()
        .await
        .expect("reflect not JSON");

    let known = resp["known"].as_array().cloned().unwrap_or_default();
    let gaps = resp["gaps"].as_array().cloned().unwrap_or_default();
    let questions = resp["suggested_questions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let has_known = !known.is_empty();
    let has_gaps = !gaps.is_empty();
    let has_questions = !questions.is_empty();
    let parts = [has_known, has_gaps, has_questions]
        .iter()
        .filter(|&&b| b)
        .count();

    let (passed, score, details) = match parts {
        3 => (
            true,
            1.0,
            format!(
                "{} known, {} gaps, {} questions generated",
                known.len(),
                gaps.len(),
                questions.len()
            ),
        ),
        2 => (
            true,
            0.6,
            format!(
                "Partial: known={}, gaps={}, questions={}",
                known.len(),
                gaps.len(),
                questions.len()
            ),
        ),
        _ => (
            false,
            0.0,
            format!(
                "known={}, gaps={}, questions={}",
                known.len(),
                gaps.len(),
                questions.len()
            ),
        ),
    };

    EvalResult {
        name: "reflect_gap_analysis",
        passed,
        score,
        details,
        elapsed: start.elapsed(),
    }
}

async fn eval_timeline_history(client: &Client, base: &str) -> EvalResult {
    let start = Instant::now();
    remember(client, base, "Frank works at Old Company").await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    remember(client, base, "Frank works at New Company").await;
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
    let superseded = events
        .iter()
        .filter(|e| e["superseded"].as_bool().unwrap_or(false))
        .count();

    let old_co_superseded = events.iter().any(|e| {
        e["fact"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("old company")
            && e["superseded"].as_bool().unwrap_or(false)
    });

    let (passed, score, details) = if events.len() >= 2 && old_co_superseded {
        (
            true,
            1.0,
            format!("{} events, {} superseded", events.len(), superseded),
        )
    } else if events.len() >= 2 {
        (
            false,
            0.5,
            format!(
                "{} events but Old Company not marked superseded",
                events.len()
            ),
        )
    } else {
        (
            false,
            0.0,
            format!("Only {} events returned", events.len()),
        )
    };

    EvalResult {
        name: "timeline_history",
        passed,
        score,
        details,
        elapsed: start.elapsed(),
    }
}

async fn eval_confidence_compounding(client: &Client, base: &str) -> EvalResult {
    let start = Instant::now();
    remember_with_agent(client, base, "Grace Lee is a lawyer", "agent-1").await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let initial_facts = query_facts(client, base, "Grace Lee lawyer", 5).await;
    let initial_confidence = initial_facts
        .first()
        .and_then(|f| f["confidence"].as_f64())
        .unwrap_or(0.0);

    remember_with_agent(client, base, "Grace Lee is a lawyer", "agent-2").await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let facts_after = query_facts(client, base, "Grace Lee lawyer", 5).await;
    let after_confidence = facts_after
        .first()
        .and_then(|f| f["confidence"].as_f64())
        .unwrap_or(0.0);

    let source_agents: Vec<String> = facts_after
        .first()
        .and_then(|f| f["source_agents"].as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let has_both = source_agents.contains(&"agent-1".to_string())
        && source_agents.contains(&"agent-2".to_string());
    let confidence_increased = after_confidence >= initial_confidence;

    let (passed, score, details) = if has_both && confidence_increased {
        (
            true,
            1.0,
            format!(
                "Confidence {:.2}->{:.2}, both agents tracked",
                initial_confidence, after_confidence
            ),
        )
    } else if confidence_increased {
        (
            false,
            0.5,
            format!(
                "Confidence ok ({:.2}->{:.2}) but agents={:?}",
                initial_confidence, after_confidence, source_agents
            ),
        )
    } else {
        (
            false,
            0.0,
            format!(
                "Confidence {:.2}->{:.2}, agents={:?}",
                initial_confidence, after_confidence, source_agents
            ),
        )
    };

    EvalResult {
        name: "confidence_compounding",
        passed,
        score,
        details,
        elapsed: start.elapsed(),
    }
}

#[tokio::main]
async fn main() {
    println!("\n=== Hippo — Eval Score ===\n");
    println!("Running 7 correctness evals...\n");

    let mut results = Vec::new();

    macro_rules! run_eval {
        ($name:expr, $fn:ident) => {{
            let agent = start_agent().await;
            let result = $fn(&agent.client, &agent.base_url).await;
            let status = if result.passed { "PASS" } else { "FAIL" };
            println!(
                "  [{status}] {:<30} ({:.1}s)  - {}",
                $name,
                result.elapsed.as_secs_f64(),
                result.details
            );
            results.push(result);
        }};
    }

    run_eval!("contradiction_detection", eval_contradiction_detection);
    run_eval!("temporal_query", eval_temporal_query);
    run_eval!("multi_hop_retrieval", eval_multi_hop_retrieval);
    run_eval!("entity_resolution", eval_entity_resolution);
    run_eval!("reflect_gap_analysis", eval_reflect_gap_analysis);
    run_eval!("timeline_history", eval_timeline_history);
    run_eval!("confidence_compounding", eval_confidence_compounding);

    let total = results.len();
    let pass_count = results.iter().filter(|r| r.passed).count();
    let total_score: f32 = results.iter().map(|r| r.score).sum();
    let pct = (pass_count as f64 / total as f64) * 100.0;

    println!("\nSCORE: {pass_count}/{total} PASS  ({pct:.1}%)");
    println!("WEIGHTED: {total_score:.1}/{total}.0\n");

    let failed: Vec<&EvalResult> = results.iter().filter(|r| !r.passed).collect();
    if !failed.is_empty() {
        println!("FAILED evals:");
        for r in &failed {
            println!("  - {}: {}", r.name, r.details);
        }
        println!();
    }

    if pass_count < total {
        std::process::exit(1);
    }
}
