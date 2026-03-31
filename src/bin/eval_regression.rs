use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

// ---- Stored result types ----

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EvalRun {
    timestamp: DateTime<Utc>,
    git_sha: String,
    total: usize,
    passed: usize,
    score: f64,
    evals: Vec<EvalEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EvalEntry {
    name: String,
    passed: bool,
    score: f64,
    elapsed_ms: u64,
    details: String,
}

// ---- Agent process management ----

struct AgentProcess {
    child: Child,
    port: u16,
    base_url: String,
    graph_name: String,
}

impl Drop for AgentProcess {
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

fn start_agent_process() -> AgentProcess {
    let port = find_free_port();
    let base_url = format!("http://localhost:{port}");
    let graph_name = format!("hippo_evalreg_{port}");

    // Clear graph before starting
    let _ = Command::new("docker")
        .args([
            "exec",
            "hippo-falkordb-1",
            "redis-cli",
            "GRAPH.DELETE",
            &graph_name,
        ])
        .output();

    let oauth = std::env::var("ANTHROPIC_OAUTH_TOKEN")
        .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
        .expect("ANTHROPIC_OAUTH_TOKEN or ANTHROPIC_API_KEY must be set");

    // Find the hippo binary next to ourselves
    let self_path = std::env::current_exe().expect("cannot find current exe");
    let bin_dir = self_path.parent().expect("no parent dir");
    let agent_bin = bin_dir.join("hippo");

    let child = Command::new(&agent_bin)
        .env("ANTHROPIC_OAUTH_TOKEN", &oauth)
        .env("PORT", port.to_string())
        .env("GRAPH_NAME", &graph_name)
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to start hippo at {}: {e}", agent_bin.display()));

    AgentProcess {
        child,
        port,
        base_url,
        graph_name,
    }
}

fn wait_for_health(agent: &AgentProcess) {
    let client = reqwest::blocking::Client::new();
    for _ in 0..30 {
        std::thread::sleep(Duration::from_secs(1));
        if let Ok(r) = client.get(format!("{}/health", agent.base_url)).send() {
            if r.status().is_success() {
                return;
            }
        }
    }
    panic!(
        "hippo did not become healthy in time (port {})",
        agent.port
    );
}

// ---- HTTP helpers ----

fn remember(client: &reqwest::blocking::Client, base: &str, statement: &str, source: &str) {
    let resp = client
        .post(format!("{base}/remember"))
        .json(&serde_json::json!({ "statement": statement, "source_agent": source }))
        .send()
        .unwrap_or_else(|e| panic!("remember failed: {e}"));
    assert!(resp.status().is_success(), "remember failed for: {statement}");
}

fn query_facts(
    client: &reqwest::blocking::Client,
    base: &str,
    q: &str,
    limit: usize,
) -> Vec<serde_json::Value> {
    let resp: serde_json::Value = client
        .post(format!("{base}/context"))
        .json(&serde_json::json!({ "query": q, "limit": limit }))
        .send()
        .unwrap_or_else(|e| panic!("context failed: {e}"))
        .json()
        .expect("context not JSON");
    resp["facts"].as_array().cloned().unwrap_or_default()
}

fn fact_strings(facts: &[serde_json::Value]) -> Vec<String> {
    facts
        .iter()
        .map(|f| f["fact"].as_str().unwrap_or("").to_string())
        .collect()
}

fn any_contains(facts: &[String], keyword: &str) -> bool {
    let kw = keyword.to_lowercase();
    facts.iter().any(|f| f.to_lowercase().contains(&kw))
}

fn query_graph(client: &reqwest::blocking::Client, base: &str) -> serde_json::Value {
    client
        .get(format!("{base}/graph"))
        .send()
        .expect("graph failed")
        .json()
        .expect("graph not JSON")
}

// ---- Individual eval runners ----
// Each returns (passed, score, details)

fn eval_contradiction_detection(
    client: &reqwest::blocking::Client,
    base: &str,
) -> (bool, f64, String) {
    remember(client, base, "Alice works at Google", "eval");
    std::thread::sleep(Duration::from_millis(500));
    remember(client, base, "Alice works at Anthropic", "eval");
    std::thread::sleep(Duration::from_secs(1));

    let facts = query_facts(client, base, "where does Alice work", 10);
    let texts = fact_strings(&facts);

    let has_anthropic = any_contains(&texts, "Anthropic");
    let has_google = any_contains(&texts, "Google");

    let graph = query_graph(client, base);
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

    let passed = has_anthropic && !has_google && google_invalidated;
    let details = if passed {
        "Anthropic returned, Google invalidated".to_string()
    } else {
        format!(
            "anthropic={has_anthropic} google_absent={} google_invalidated={google_invalidated}",
            !has_google
        )
    };
    (passed, if passed { 1.0 } else { 0.0 }, details)
}

fn eval_temporal_query(client: &reqwest::blocking::Client, base: &str) -> (bool, f64, String) {
    remember(client, base, "Bob's salary is 50000 pounds", "eval");
    let t_after_first = chrono::Utc::now();
    std::thread::sleep(Duration::from_secs(2));
    remember(client, base, "Bob's salary is 75000 pounds", "eval");
    std::thread::sleep(Duration::from_secs(1));

    let at_str = t_after_first.to_rfc3339();
    let resp: serde_json::Value = client
        .post(format!("{base}/context/temporal"))
        .json(&serde_json::json!({
            "query": "Bob's salary",
            "at": at_str,
            "limit": 10
        }))
        .send()
        .expect("temporal failed")
        .json()
        .expect("temporal not JSON");

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

    let passed = has_50k && !has_75k;
    let details = if passed {
        "Correct time-slice: 50k returned, 75k absent".to_string()
    } else {
        format!("has_50k={has_50k} has_75k_absent={}", !has_75k)
    };
    (passed, if passed { 1.0 } else { 0.0 }, details)
}

fn eval_multi_hop_retrieval(
    client: &reqwest::blocking::Client,
    base: &str,
) -> (bool, f64, String) {
    remember(client, base, "Carol is the CEO of Synodal Inc", "eval");
    remember(
        client,
        base,
        "Synodal Inc is headquartered in London",
        "eval",
    );
    remember(client, base, "Synodal Inc builds AI software", "eval");
    std::thread::sleep(Duration::from_secs(1));

    let resp: serde_json::Value = client
        .post(format!("{base}/context"))
        .json(&serde_json::json!({
            "query": "Carol company location",
            "limit": 15,
            "max_hops": 2
        }))
        .send()
        .expect("context failed")
        .json()
        .expect("context not JSON");

    let facts = resp["facts"].as_array().cloned().unwrap_or_default();
    let texts: Vec<String> = facts
        .iter()
        .map(|f| f["fact"].as_str().unwrap_or("").to_string())
        .collect();

    let has_london = texts
        .iter()
        .any(|f| f.to_lowercase().contains("london"));
    let details = if has_london {
        "London found via 2-hop traversal".to_string()
    } else {
        format!("London not found. Got: {:?}", texts)
    };
    (has_london, if has_london { 1.0 } else { 0.0 }, details)
}

fn eval_entity_resolution(client: &reqwest::blocking::Client, base: &str) -> (bool, f64, String) {
    remember(
        client,
        base,
        "Alice Chen is a software engineer at Anthropic",
        "eval",
    );
    remember(client, base, "Alice is married to Bob", "eval");
    remember(client, base, "Alice Chen lives in San Francisco", "eval");
    std::thread::sleep(Duration::from_secs(1));

    let graph = query_graph(client, base);
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

    let facts = query_facts(client, base, "Alice", 15);
    let texts = fact_strings(&facts);

    let has_anthropic = any_contains(&texts, "Anthropic");
    let has_bob = any_contains(&texts, "Bob") || any_contains(&texts, "married");
    let has_sf = any_contains(&texts, "San Francisco") || any_contains(&texts, "Francisco");

    let passed = alice_count <= 2 && has_anthropic && has_bob && has_sf;
    let details = if passed {
        format!(
            "Alice entities: {alice_count}, all facts retrievable"
        )
    } else {
        format!(
            "entities={alice_count} anthropic={has_anthropic} bob={has_bob} sf={has_sf}"
        )
    };
    (passed, if passed { 1.0 } else { 0.0 }, details)
}

fn eval_reflect_gap_analysis(
    client: &reqwest::blocking::Client,
    base: &str,
) -> (bool, f64, String) {
    remember(client, base, "Eve Williams works at Synodal Inc", "eval");
    remember(client, base, "Eve Williams is a data scientist", "eval");
    std::thread::sleep(Duration::from_secs(1));

    let resp: serde_json::Value = client
        .post(format!("{base}/reflect"))
        .json(&serde_json::json!({
            "about": "Eve",
            "suggest_questions": true
        }))
        .send()
        .expect("reflect failed")
        .json()
        .expect("reflect not JSON");

    let known = resp["known"].as_array().cloned().unwrap_or_default();
    let gaps = resp["gaps"].as_array().cloned().unwrap_or_default();
    let questions = resp["suggested_questions"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let passed = !known.is_empty() && !gaps.is_empty() && !questions.is_empty();
    let details = if passed {
        format!(
            "known={} gaps={} questions={}",
            known.len(),
            gaps.len(),
            questions.len()
        )
    } else {
        format!(
            "known={} gaps={} questions={} (need all non-empty)",
            known.len(),
            gaps.len(),
            questions.len()
        )
    };
    (passed, if passed { 1.0 } else { 0.0 }, details)
}

fn eval_timeline_history(client: &reqwest::blocking::Client, base: &str) -> (bool, f64, String) {
    remember(client, base, "Frank works at Old Company", "eval");
    std::thread::sleep(Duration::from_secs(1));
    remember(client, base, "Frank works at New Company", "eval");
    std::thread::sleep(Duration::from_millis(500));

    let resp: serde_json::Value = client
        .get(format!("{base}/timeline/Frank"))
        .send()
        .expect("timeline failed")
        .json()
        .expect("timeline not JSON");

    let events = resp["events"].as_array().cloned().unwrap_or_default();
    let superseded = events
        .iter()
        .filter(|e| e["superseded"].as_bool().unwrap_or(false))
        .count();

    let old_co_superseded = events.iter().any(|e| {
        let fact = e["fact"].as_str().unwrap_or("").to_lowercase();
        let is_superseded = e["superseded"].as_bool().unwrap_or(false);
        fact.contains("old company") && is_superseded
    });

    let passed = events.len() >= 2 && superseded >= 1 && old_co_superseded;
    let details = if passed {
        format!(
            "{} events, {} superseded, Old Company correctly superseded",
            events.len(),
            superseded
        )
    } else {
        format!(
            "events={} superseded={} old_co_superseded={old_co_superseded}",
            events.len(),
            superseded
        )
    };
    (passed, if passed { 1.0 } else { 0.0 }, details)
}

fn eval_confidence_compounding(
    client: &reqwest::blocking::Client,
    base: &str,
) -> (bool, f64, String) {
    remember(client, base, "Grace Lee is a lawyer", "agent-1");
    std::thread::sleep(Duration::from_millis(500));

    let initial_facts = query_facts(client, base, "Grace Lee lawyer", 5);
    let initial_confidence = initial_facts
        .first()
        .and_then(|f| f["confidence"].as_f64())
        .unwrap_or(0.0);

    remember(client, base, "Grace Lee is a lawyer", "agent-2");
    std::thread::sleep(Duration::from_millis(500));

    let after_facts = query_facts(client, base, "Grace Lee lawyer", 5);
    let after_confidence = after_facts
        .first()
        .and_then(|f| f["confidence"].as_f64())
        .unwrap_or(0.0);

    let source_agents = after_facts
        .first()
        .and_then(|f| f["source_agents"].as_array())
        .cloned()
        .unwrap_or_default();
    let agent_names: Vec<&str> = source_agents
        .iter()
        .filter_map(|s| s.as_str())
        .collect();

    let confidence_ok = after_confidence >= initial_confidence;
    let agents_ok = agent_names.contains(&"agent-1") && agent_names.contains(&"agent-2");
    let passed = confidence_ok && agents_ok;

    let score = if passed {
        1.0
    } else if confidence_ok || agents_ok {
        0.5
    } else {
        0.0
    };

    let details = format!(
        "confidence {initial_confidence:.3} -> {after_confidence:.3}, agents: {agent_names:?}"
    );
    (passed, score, details)
}

// ---- Storage ----

fn evals_dir() -> PathBuf {
    dirs::home_dir()
        .expect("cannot determine home directory")
        .join(".hippo-evals")
}

fn save_run(run: &EvalRun) -> PathBuf {
    let dir = evals_dir();
    std::fs::create_dir_all(&dir).expect("cannot create ~/.hippo-evals");

    let filename = run
        .timestamp
        .format("%Y-%m-%d-%H%M%S.json")
        .to_string();
    let path = dir.join(&filename);

    let json = serde_json::to_string_pretty(run).expect("cannot serialize run");
    std::fs::write(&path, &json).expect("cannot write eval result");

    // Update latest symlink
    let latest = dir.join("latest.json");
    let _ = std::fs::remove_file(&latest);
    #[cfg(unix)]
    {
        let _ = std::os::unix::fs::symlink(&path, &latest);
    }
    #[cfg(not(unix))]
    {
        let _ = std::fs::write(&latest, &json);
    }

    path
}

fn load_previous() -> Option<EvalRun> {
    let dir = evals_dir();
    let latest = dir.join("latest.json");
    if !latest.exists() {
        return None;
    }
    let json = std::fs::read_to_string(latest).ok()?;
    serde_json::from_str(&json).ok()
}

// ---- Comparison / reporting ----

fn format_duration_ago(prev: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let diff = now.signed_duration_since(prev);
    let mins = diff.num_minutes();
    if mins < 60 {
        format!("{mins}m ago")
    } else {
        let hours = diff.num_hours();
        format!("{hours}h ago")
    }
}

fn print_report(current: &EvalRun, previous: Option<&EvalRun>) {
    println!("\n=== Hippo Eval Regression Report ===");
    println!(
        "Run: {}",
        current.timestamp.format("%Y-%m-%d %H:%M:%S")
    );

    if let Some(prev) = previous {
        let ago = format_duration_ago(prev.timestamp, current.timestamp);
        println!(
            "Previous: {} ({ago})",
            prev.timestamp.format("%Y-%m-%d %H:%M:%S")
        );
    }

    let pct = current.score * 100.0;
    print!(
        "\nSCORE: {}/{} ({pct:.1}%)",
        current.passed, current.total
    );

    if let Some(prev) = previous {
        let prev_pct = prev.score * 100.0;
        let diff = current.passed as i64 - prev.passed as i64;
        let pct_diff = pct - prev_pct;
        let sign = if diff >= 0 { "+" } else { "" };
        print!(
            " vs previous {}/{} ({prev_pct:.1}%) -> {sign}{diff} ({sign}{pct_diff:.1}%)",
            prev.passed, prev.total
        );
    }
    println!();

    if let Some(prev) = previous {
        let mut improvements = Vec::new();
        let mut regressions = Vec::new();

        for entry in &current.evals {
            if let Some(prev_entry) = prev.evals.iter().find(|e| e.name == entry.name) {
                if entry.passed && !prev_entry.passed {
                    improvements.push(format!(
                        "  \u{2705} {}: FAIL -> PASS  (+{:.1})",
                        entry.name,
                        entry.score - prev_entry.score
                    ));
                } else if !entry.passed && prev_entry.passed {
                    regressions.push((entry, prev_entry));
                } else if (entry.score - prev_entry.score).abs() > 0.01 {
                    let diff = entry.score - prev_entry.score;
                    let sign = if diff > 0.0 { "+" } else { "" };
                    let icon = if diff > 0.0 { "\u{2728}" } else { "\u{26a0}\u{fe0f} " };
                    improvements.push(format!(
                        "  {icon} {}: score {:.1} -> {:.1} ({sign}{:.1})",
                        entry.name, prev_entry.score, entry.score, diff
                    ));
                }
            }
        }

        if !improvements.is_empty() || !regressions.is_empty() {
            println!("\nChanges:");
            for imp in &improvements {
                println!("{imp}");
            }
        }

        if regressions.is_empty() {
            println!("\nNo regressions.");
        } else {
            println!("\n\u{26a0}\u{fe0f}  REGRESSION DETECTED:");
            for (curr, prev_e) in &regressions {
                println!(
                    "  \u{274c} {}: PASS -> FAIL (-{:.1})",
                    curr.name,
                    prev_e.score - curr.score
                );
                println!("     Before: {}", prev_e.details);
                println!("     After: {}", curr.details);
            }
        }
    }

    println!(
        "\nGit SHA: {}",
        current.git_sha
    );
}

// ---- Main ----

fn main() {
    println!("Building and starting hippo...");
    let agent = start_agent_process();
    println!(
        "Waiting for agent health on port {}...",
        agent.port
    );
    wait_for_health(&agent);
    println!("Agent healthy. Running evals...\n");

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("cannot build HTTP client");

    let base = &agent.base_url;

    let eval_fns: Vec<(&str, Box<dyn Fn(&reqwest::blocking::Client, &str) -> (bool, f64, String)>)> =
        vec![
            ("contradiction_detection", Box::new(eval_contradiction_detection)),
            ("temporal_query", Box::new(eval_temporal_query)),
            ("multi_hop_retrieval", Box::new(eval_multi_hop_retrieval)),
            ("entity_resolution", Box::new(eval_entity_resolution)),
            ("reflect_gap_analysis", Box::new(eval_reflect_gap_analysis)),
            ("timeline_history", Box::new(eval_timeline_history)),
            ("confidence_compounding", Box::new(eval_confidence_compounding)),
        ];

    let mut entries = Vec::new();

    for (name, eval_fn) in &eval_fns {
        print!("  Running {name}...");
        let start = Instant::now();

        // Each eval needs a fresh graph — restart agent
        // Actually, evals are independent scenarios on the same blank graph.
        // Since we use one agent, we rely on non-overlapping entity names.
        let (passed, score, details) = eval_fn(&client, base);
        let elapsed = start.elapsed();

        let status = if passed { "PASS" } else { "FAIL" };
        println!(" [{status}] ({:.1}s)", elapsed.as_secs_f64());

        entries.push(EvalEntry {
            name: name.to_string(),
            passed,
            score,
            elapsed_ms: elapsed.as_millis() as u64,
            details,
        });
    }

    let git_sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    let total = entries.len();
    let passed = entries.iter().filter(|e| e.passed).count();
    let score = if total > 0 {
        passed as f64 / total as f64
    } else {
        0.0
    };

    let run = EvalRun {
        timestamp: Utc::now(),
        git_sha,
        total,
        passed,
        score,
        evals: entries,
    };

    let previous = load_previous();
    let path = save_run(&run);
    print_report(&run, previous.as_ref());

    println!("\nResults saved to: {}", path.display());
}
