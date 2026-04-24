#![allow(dead_code)]

pub mod fixtures;

use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures::future::join_all;
use reqwest::Client;
use serde_json::Value;

/// A running hippo process. Killed on drop.
pub struct TestAgent {
    child: Child,
    pub port: u16,
    pub base_url: String,
    pub client: Client,
    pub graph_name: String,
    use_falkordb: bool,
}

impl Drop for TestAgent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if self.use_falkordb {
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
}

fn find_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Spawn a fresh hippo on a random port with a clean graph.
pub async fn start_agent() -> TestAgent {
    start_agent_inner(false).await
}

/// Spawn a fresh hippo with ALLOW_ADMIN=1, then seed the fixture graph.
pub async fn start_agent_with_fixture() -> (TestAgent, fixtures::GraphFixture) {
    let agent = start_agent_inner(true).await;
    let fixture = fixtures::GraphFixture::build();
    seed_fixture(&agent.client, &agent.base_url, &fixture).await;
    (agent, fixture)
}

/// Spawn a fresh hippo with ALLOW_ADMIN=1 and MOCK_LLM=1 (no LLM key needed).
pub async fn start_agent_mock_with_fixture() -> (TestAgent, fixtures::GraphFixture) {
    let agent = start_agent_inner_opts(true, true).await;
    let fixture = fixtures::GraphFixture::build();
    seed_fixture(&agent.client, &agent.base_url, &fixture).await;
    (agent, fixture)
}

/// Spawn a fresh hippo with ALLOW_ADMIN=1, MOCK_LLM=1, no fixture seeded.
pub async fn start_agent_mock_admin() -> TestAgent {
    start_agent_inner_opts(true, true).await
}

async fn start_agent_inner(allow_admin: bool) -> TestAgent {
    start_agent_inner_opts(allow_admin, false).await
}

async fn start_agent_inner_opts(allow_admin: bool, force_mock: bool) -> TestAgent {
    let port = find_free_port();
    let base_url = format!("http://localhost:{port}");
    let graph_name = format!("hippo_test_{port}");

    let use_falkordb = std::env::var("GRAPH_BACKEND").as_deref() == Ok("falkordb");

    if use_falkordb {
        // Clear the FalkorDB graph before starting
        let _ = Command::new("docker")
            .args([
                "exec",
                "hippo-falkordb-1",
                "redis-cli",
                "GRAPH.DELETE",
                &graph_name,
            ])
            .output();
    }

    let bin = env!("CARGO_BIN_EXE_hippo");
    let mock_llm = force_mock || std::env::var("EVAL_MOCK").is_ok() || !use_falkordb;
    let oauth = std::env::var("ANTHROPIC_OAUTH_TOKEN")
        .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
        .unwrap_or_else(|_| {
            if mock_llm {
                "mock-key-not-used".to_string()
            } else {
                panic!("ANTHROPIC_OAUTH_TOKEN or ANTHROPIC_API_KEY must be set")
            }
        });

    let mut cmd = Command::new(bin);
    cmd.env("ANTHROPIC_OAUTH_TOKEN", &oauth)
        .env("PORT", port.to_string())
        .env("GRAPH_NAME", &graph_name)
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if !use_falkordb {
        cmd.env("GRAPH_BACKEND", "memory");
    }

    if allow_admin {
        cmd.env("ALLOW_ADMIN", "1");
    }
    if std::env::var("EVAL_RECORD").is_ok() {
        cmd.env("EVAL_RECORD", "1");
    } else if std::env::var("EVAL_REPLAY").is_ok() {
        cmd.env("EVAL_REPLAY", "1");
    }
    if let Ok(fp) = std::env::var("FIXTURE_PATH") {
        cmd.env("FIXTURE_PATH", fp);
    }
    if mock_llm {
        cmd.env("MOCK_LLM", "1");
    }

    let child = cmd.spawn().expect("failed to start hippo");

    let agent = TestAgent {
        child,
        port,
        base_url: base_url.clone(),
        client: Client::new(),
        graph_name,
        use_falkordb,
    };

    // Wait for health
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

/// Seed a fixture graph into a running agent via POST /seed.
pub async fn seed_fixture(client: &Client, base_url: &str, fixture: &fixtures::GraphFixture) {
    let body = fixture.to_seed_json();
    let resp = client
        .post(format!("{base_url}/seed"))
        .json(&body)
        .send()
        .await
        .expect("seed request failed");
    assert!(
        resp.status().is_success(),
        "seed failed: {:?}",
        resp.text().await
    );
}

/// Seed a partial graph (custom JSON) via POST /seed.
pub async fn seed_raw(client: &Client, base_url: &str, body: &serde_json::Value) {
    let resp = client
        .post(format!("{base_url}/seed"))
        .json(body)
        .send()
        .await
        .expect("seed request failed");
    assert!(
        resp.status().is_success(),
        "seed failed: {:?}",
        resp.text().await
    );
}

// ---- HTTP helpers ----

pub async fn remember(client: &Client, base_url: &str, statement: &str) {
    let resp = client
        .post(format!("{base_url}/remember"))
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

pub async fn remember_as(client: &Client, base_url: &str, statement: &str, source: &str) {
    let resp = client
        .post(format!("{base_url}/remember"))
        .json(&serde_json::json!({ "statement": statement, "source_agent": source }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("remember_as '{statement}' failed: {e}"));
    assert!(resp.status().is_success());
    let _: Value = resp.json().await.expect("remember_as response not JSON");
}

pub async fn query_facts(client: &Client, base_url: &str, q: &str, _limit: usize) -> Vec<Value> {
    let resp: Value = client
        .post(format!("{base_url}/context"))
        .json(&serde_json::json!({ "query": q }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("context '{q}' failed: {e}"))
        .json()
        .await
        .expect("context response not JSON");

    // /context now returns GraphContext {nodes, edges} — extract edges as facts
    resp["edges"].as_array().cloned().unwrap_or_default()
}

pub fn fact_strings(facts: &[Value]) -> Vec<String> {
    facts
        .iter()
        .map(|f| f["fact"].as_str().unwrap_or("").to_string())
        .collect()
}

pub async fn query_graph(client: &Client, base_url: &str) -> Value {
    client
        .get(format!("{base_url}/graph"))
        .send()
        .await
        .expect("graph request failed")
        .json()
        .await
        .expect("graph response not JSON")
}

pub async fn diagnose(client: &Client, base_url: &str, q: &str) -> Value {
    client
        .post(format!("{base_url}/diagnose"))
        .json(&serde_json::json!({ "query": q, "limit": 10 }))
        .send()
        .await
        .expect("diagnose request failed")
        .json()
        .await
        .expect("diagnose response not JSON")
}

// ---- Fact loading ----

pub fn base_facts() -> Vec<&'static str> {
    vec![
        // Family
        "Alice is my sister",
        "Alice and Bob got married in June 2020",
        "Alice has two kids, Mia and Leo",
        "Bob works as a software engineer at Google",
        // Friends and colleagues
        "Carol is my colleague at Acme Corp",
        "I have known Carol since university",
        "David is my closest friend from school",
        "David and Carol met at my birthday party last year",
        // Events
        "I attended Alice and Bob's wedding with Carol and David",
        "Alice's birthday is in March",
        "We had a family dinner last Christmas at Alice's house",
        // Health
        "My doctor is Dr. Smith at City Medical",
        "I take metformin for type 2 diabetes, prescribed by Dr. Smith",
        "I had a blood test last month, results were normal",
        "Dr. Smith referred me to a cardiologist named Dr. Patel",
        // Finance
        "I have a savings account at First Bank",
        "My mortgage is with City Credit Union",
        "I invest monthly in an index fund through Vanguard",
        "First Bank has a joint account with Alice",
        // Cross-domain
        "Dr. Smith's office sent a bill for 200 dollars",
        "Alice recommended Dr. Patel to me after her own checkup",
    ]
}

/// Load all base facts concurrently.
pub async fn load_base_facts(client: &Client, base_url: &str) {
    let statements = base_facts();
    println!("Loading {} facts (concurrent)...", statements.len());
    let start = Instant::now();

    let futures: Vec<_> = statements
        .iter()
        .map(|s| remember(client, base_url, s))
        .collect();
    join_all(futures).await;

    let elapsed = start.elapsed();
    println!("Done loading facts in {:.1}s.", elapsed.as_secs_f64());
}

// ---- Eval infrastructure ----

pub struct EvalCase {
    pub name: &'static str,
    pub query: &'static str,
    pub limit: usize,
    pub must_contain: Vec<&'static str>,
    pub must_not_contain: Vec<&'static str>,
}

pub struct EvalResult {
    pub name: String,
    pub passed: bool,
    pub failures: Vec<String>,
    pub facts_returned: Vec<String>,
    pub elapsed: Duration,
}

impl EvalResult {
    pub fn print(&self) {
        let status = if self.passed { "PASS" } else { "FAIL" };
        println!(
            "  [{status}] {} ({:.1}s)",
            self.name,
            self.elapsed.as_secs_f64()
        );
        if !self.passed {
            for f in &self.failures {
                println!("         {f}");
            }
            println!("         returned facts:");
            for (i, fact) in self.facts_returned.iter().enumerate() {
                println!("           {}. {fact}", i + 1);
            }
        }
    }
}

/// Check that at least one fact contains `keyword` (case-insensitive).
pub fn any_fact_contains(facts: &[String], keyword: &str) -> bool {
    let kw = keyword.to_lowercase();
    facts.iter().any(|f| f.to_lowercase().contains(&kw))
}

/// Check that NO fact contains `keyword` (case-insensitive).
pub fn no_fact_contains(facts: &[String], keyword: &str) -> bool {
    let kw = keyword.to_lowercase();
    !facts.iter().any(|f| f.to_lowercase().contains(&kw))
}

/// Run an EvalCase against the agent, returning an EvalResult.
/// On failure, also calls /diagnose and prints pipeline steps.
pub async fn run_eval_case(client: &Client, base_url: &str, case: &EvalCase) -> EvalResult {
    let start = Instant::now();
    let raw_facts = query_facts(client, base_url, case.query, case.limit).await;
    let facts = fact_strings(&raw_facts);
    let mut failures = Vec::new();

    // Check that at least one expected keyword appears
    if !case.must_contain.is_empty() {
        let any_found = case
            .must_contain
            .iter()
            .any(|kw| any_fact_contains(&facts, kw));
        if !any_found {
            failures.push(format!(
                "expected at least one of {:?} in results",
                case.must_contain
            ));
        }
    }

    // Check that excluded keywords don't appear
    for kw in &case.must_not_contain {
        if !no_fact_contains(&facts, kw) {
            failures.push(format!("unexpected keyword '{}' found in results", kw));
        }
    }

    // Non-empty results
    if facts.is_empty() {
        failures.push("no facts returned".to_string());
    }

    let elapsed = start.elapsed();
    let passed = failures.is_empty();

    // On failure: print full facts with scores, and run /diagnose
    if !passed {
        println!("    --- returned facts with scores ---");
        for f in &raw_facts {
            println!(
                "      fact={:?}  confidence={}  salience={}",
                f["fact"].as_str().unwrap_or("?"),
                f["confidence"],
                f["salience"],
            );
        }
        println!("    --- diagnose pipeline for {:?} ---", case.query);
        let diag = diagnose(client, base_url, case.query).await;
        if let Some(steps) = diag["steps"].as_array() {
            for step in steps {
                let name = step["step"].as_str().unwrap_or("?");
                let desc = step["description"].as_str().unwrap_or("");
                let count = step["results"].as_array().map_or(0, |a| a.len());
                println!("      [{name}] {desc} ({count} results)");
            }
        }
    }

    EvalResult {
        name: format!("retrieval_precision[{}]", case.query),
        passed,
        failures,
        facts_returned: facts,
        elapsed,
    }
}

/// Print eval summary and return (passed, total).
pub fn print_summary(results: &[EvalResult]) -> (usize, usize) {
    let total = results.len();
    let passed = results.iter().filter(|r| r.passed).count();
    let failed = total - passed;

    println!("\n============================================================");
    println!("EVAL SCORE: {passed}/{total} passed ({failed} failed)");
    println!("============================================================");

    if failed > 0 {
        println!("\nFailing cases:");
        for r in results {
            if !r.passed {
                println!("  - {}: {}", r.name, r.failures.join(", "));
            }
        }
    }

    (passed, total)
}
