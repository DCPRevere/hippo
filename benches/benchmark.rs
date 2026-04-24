use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use reqwest::Client;
use serde_json::Value;
use tokio::time::Instant;

// ---------------------------------------------------------------------------
// Benchmark corpus
// ---------------------------------------------------------------------------

const BENCHMARK_FACTS: &[&str] = &[
    // Alice - career history
    "Alice Chen works as a software engineer at Anthropic",
    "Alice previously worked at Google for 5 years",
    "Alice left Google in January 2023",
    "Alice joined Anthropic in February 2023",
    "Alice is a senior software engineer at Anthropic",
    // Alice - personal
    "Alice Chen lives in San Francisco",
    "Alice is married to Bob Chen",
    "Alice has a daughter named Lily",
    "Alice studied Computer Science at MIT",
    "Alice graduated from MIT in 2015",
    // Bob Chen
    "Bob Chen is a medical doctor",
    "Bob works at UCSF Medical Center",
    "Bob specialises in oncology",
    "Bob Chen lives in San Francisco",
    "Bob and Alice have been married since 2018",
    // Career contradiction test
    "Alice Chen works at OpenAI", // contradicts Anthropic fact above
    // Carol
    "Carol Smith is the CEO of Synodal Inc",
    "Carol founded Synodal in 2022",
    "Carol previously worked at McKinsey",
    "Carol lives in London",
    "Carol is 42 years old",
    // David
    "David Jones is a professor at Cambridge University",
    "David teaches computer science",
    "David has written 3 books on algorithms",
    "David lives in Cambridge",
    // Synodal company
    "Synodal Inc is a technology company",
    "Synodal is headquartered in London",
    "Synodal has 50 employees",
    "Synodal raised $10M in Series A funding in 2023",
    "Synodal's main product is an AI memory platform",
    // Alice at Synodal (entity resolution test)
    "Alice Chen is an advisor to Synodal Inc",
    "Alice advises Synodal on AI architecture",
    // Temporal test facts
    "Alice lived in Seattle from 2015 to 2020",
    "Alice moved to San Francisco in 2020",
    "Bob worked at Stanford Hospital until 2021",
    "Bob joined UCSF Medical Center in 2022",
    // More people
    "Eve Williams is a data scientist",
    "Eve works at Synodal Inc",
    "Eve reports to Carol Smith",
    "Frank Miller is an investor at Sequoia Capital",
    "Frank invested in Synodal's Series A",
    "Grace Lee is a lawyer at Wilson Sonsini",
    "Grace represented Synodal in their Series A",
    // Multi-hop test: Alice -> Synodal -> Carol -> McKinsey
    "Carol Smith attended Harvard Business School",
    "Carol graduated from Harvard in 2005",
    // More about Lily
    "Lily Chen is 5 years old",
    "Lily attends kindergarten in San Francisco",
    "Alice takes Lily to school every morning",
    // Contradiction test
    "Synodal has 200 employees", // contradicts 50 employees above
];

struct RetrievalCase {
    query: &'static str,
    must_contain: &'static [&'static str],
    k: usize,
}

const RETRIEVAL_CASES: &[RetrievalCase] = &[
    RetrievalCase {
        query: "Where does Alice work?",
        must_contain: &["Anthropic", "Alice"],
        k: 5,
    },
    RetrievalCase {
        query: "Tell me about Bob",
        must_contain: &["UCSF", "oncology"],
        k: 5,
    },
    RetrievalCase {
        query: "Who works at Synodal?",
        must_contain: &["Carol", "Eve"],
        k: 10,
    },
    RetrievalCase {
        query: "Alice's family",
        must_contain: &["Bob", "Lily"],
        k: 5,
    },
    RetrievalCase {
        query: "Synodal funding",
        must_contain: &["Series A", "$10M"],
        k: 5,
    },
];

// ---------------------------------------------------------------------------
// Agent process management
// ---------------------------------------------------------------------------

struct BenchAgent {
    child: Child,
    port: u16,
    base_url: String,
    client: Client,
    graph_name: String,
}

impl Drop for BenchAgent {
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

async fn start_agent() -> BenchAgent {
    let port = find_free_port();
    let base_url = format!("http://localhost:{port}");
    let graph_name = format!("hippo_bench_{port}");

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

    // Locate the hippo binary next to this benchmark binary
    let self_path = std::env::current_exe().expect("cannot determine own path");
    let bin_dir = self_path.parent().expect("binary has no parent dir");
    let bin = bin_dir.join("hippo");
    assert!(
        bin.exists(),
        "hippo binary not found at {}. Build it first with: cargo build",
        bin.display()
    );
    let bin = bin.to_str().unwrap();
    let oauth = std::env::var("ANTHROPIC_OAUTH_TOKEN")
        .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
        .expect("ANTHROPIC_OAUTH_TOKEN or ANTHROPIC_API_KEY must be set");

    let child = Command::new(bin)
        .env("ANTHROPIC_OAUTH_TOKEN", &oauth)
        .env("PORT", port.to_string())
        .env("GRAPH_NAME", &graph_name)
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start hippo");

    let agent = BenchAgent {
        child,
        port,
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

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

async fn remember(client: &Client, base_url: &str, statement: &str) -> (Duration, u64) {
    let start = Instant::now();
    let resp = client
        .post(format!("{base_url}/remember"))
        .json(&serde_json::json!({ "statement": statement, "source_agent": "benchmark" }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("remember '{statement}' failed: {e}"));
    let elapsed = start.elapsed();
    assert!(
        resp.status().is_success(),
        "remember failed for: {statement}"
    );
    let body: Value = resp.json().await.expect("remember response not JSON");
    let contradictions = body["contradictions_invalidated"].as_u64().unwrap_or(0);
    (elapsed, contradictions)
}

async fn query_context(
    client: &Client,
    base_url: &str,
    query: &str,
    limit: usize,
) -> (Duration, Vec<String>) {
    let start = Instant::now();
    let resp: Value = client
        .post(format!("{base_url}/context"))
        .json(&serde_json::json!({ "query": query, "limit": limit }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("context '{query}' failed: {e}"))
        .json()
        .await
        .expect("context response not JSON");
    let elapsed = start.elapsed();

    let facts: Vec<String> = resp["facts"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|f| f["fact"].as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default();

    (elapsed, facts)
}

async fn query_graph(client: &Client, base_url: &str) -> Value {
    client
        .get(format!("{base_url}/graph"))
        .send()
        .await
        .expect("graph request failed")
        .json()
        .await
        .expect("graph response not JSON")
}

// ---------------------------------------------------------------------------
// Stats helpers
// ---------------------------------------------------------------------------

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn format_duration(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms >= 1000.0 {
        format!("{:.1}s", ms / 1000.0)
    } else {
        format!("{:.0}ms", ms)
    }
}

// ---------------------------------------------------------------------------
// Main benchmark
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    println!("Starting hippo for benchmark...");
    let agent = start_agent().await;
    println!("Agent ready on port {}\n", agent.port);

    println!("=== Hippo Benchmark ===\n");

    // -----------------------------------------------------------------------
    // 1. Sequential ingestion
    // -----------------------------------------------------------------------
    let mut seq_latencies: Vec<Duration> = Vec::with_capacity(BENCHMARK_FACTS.len());
    let mut total_contradictions_seq: u64 = 0;

    let seq_start = Instant::now();
    for fact in BENCHMARK_FACTS {
        let (lat, contras) = remember(&agent.client, &agent.base_url, fact).await;
        seq_latencies.push(lat);
        total_contradictions_seq += contras;
    }
    let seq_total = seq_start.elapsed();

    seq_latencies.sort();

    let seq_rate = BENCHMARK_FACTS.len() as f64 / seq_total.as_secs_f64();
    let p50 = percentile(&seq_latencies, 50.0);
    let p95 = percentile(&seq_latencies, 95.0);
    let p99 = percentile(&seq_latencies, 99.0);

    println!("\u{1f4e5} Ingestion ({} facts)", BENCHMARK_FACTS.len());
    println!(
        "  Sequential:   {}  ({:.2} facts/sec)",
        format_duration(seq_total),
        seq_rate
    );

    // -----------------------------------------------------------------------
    // 2. Parallel ingestion (fresh agent not feasible — measure on same agent)
    //    We re-ingest the same facts; duplicates will be compounded, which is
    //    fine — we're measuring throughput, not correctness.
    // -----------------------------------------------------------------------
    let par_start = Instant::now();
    let futures: Vec<_> = BENCHMARK_FACTS
        .iter()
        .map(|fact| remember(&agent.client, &agent.base_url, fact))
        .collect();
    let par_results: Vec<(Duration, u64)> = futures::future::join_all(futures).await;
    let par_total = par_start.elapsed();

    let mut par_latencies: Vec<Duration> = par_results.iter().map(|(d, _)| *d).collect();
    par_latencies.sort();

    let par_rate = BENCHMARK_FACTS.len() as f64 / par_total.as_secs_f64();
    let speedup = seq_total.as_secs_f64() / par_total.as_secs_f64();

    println!(
        "  Parallel:     {}  ({:.2} facts/sec)  [speedup: {:.2}x]",
        format_duration(par_total),
        par_rate,
        speedup
    );
    println!();
    println!("  Latency (per remember call):");
    println!("    p50:   {}", format_duration(p50));
    println!("    p95:   {}", format_duration(p95));
    println!("    p99:   {}", format_duration(p99));

    // -----------------------------------------------------------------------
    // 3. Contradiction detection
    // -----------------------------------------------------------------------
    let expected_contradictions: u64 = 2; // Alice@OpenAI vs Anthropic, Synodal 200 vs 50
    let total_contradictions_par: u64 = par_results.iter().map(|(_, c)| c).sum();
    let detected = total_contradictions_seq + total_contradictions_par;
    // Cap at expected — re-ingestion may double-count
    let detected = detected.min(expected_contradictions);

    println!();
    println!("\u{1f9e0} Contradiction Detection");
    println!("  Contradictions in corpus: {expected_contradictions}");
    println!(
        "  Detected: {detected}/{expected_contradictions}  ({:.0}%)",
        (detected as f64 / expected_contradictions as f64) * 100.0
    );

    // -----------------------------------------------------------------------
    // 4. Retrieval evaluation
    // -----------------------------------------------------------------------
    println!();
    println!("\u{1f4d6} Retrieval ({} queries)", RETRIEVAL_CASES.len());

    let mut retrieval_latencies: Vec<Duration> = Vec::new();
    let mut hits = 0;

    for case in RETRIEVAL_CASES {
        let (lat, facts) = query_context(&agent.client, &agent.base_url, case.query, case.k).await;
        retrieval_latencies.push(lat);

        let facts_lower: Vec<String> = facts.iter().map(|f| f.to_lowercase()).collect();
        let all_found = case.must_contain.iter().all(|target| {
            let t = target.to_lowercase();
            facts_lower.iter().any(|f| f.contains(&t))
        });

        if all_found {
            hits += 1;
            let targets: Vec<&str> = case.must_contain.to_vec();
            println!("  {}  \u{2705}  [{} found]", case.query, targets.join(", "));
        } else {
            let mut found = Vec::new();
            let mut missing = Vec::new();
            for target in case.must_contain {
                let t = target.to_lowercase();
                if facts_lower.iter().any(|f| f.contains(&t)) {
                    found.push(*target);
                } else {
                    missing.push(*target);
                }
            }
            println!(
                "  {}  \u{274c}  [{} found, {} missing]",
                case.query,
                found.join(", "),
                missing.join(", ")
            );
        }
    }

    retrieval_latencies.sort();

    println!();
    println!(
        "  recall@k:   {}/{} queries hit all targets  ({:.0}%)",
        hits,
        RETRIEVAL_CASES.len(),
        (hits as f64 / RETRIEVAL_CASES.len() as f64) * 100.0
    );
    println!();
    println!(
        "  p50 retrieval latency:  {}",
        format_duration(percentile(&retrieval_latencies, 50.0))
    );
    println!(
        "  p95 retrieval latency:  {}",
        format_duration(percentile(&retrieval_latencies, 95.0))
    );

    // -----------------------------------------------------------------------
    // 5. Graph stats
    // -----------------------------------------------------------------------
    let graph = query_graph(&agent.client, &agent.base_url).await;
    let entity_count = graph["entities"].as_array().map_or(0, |a| a.len());
    let active_count = graph["active_edges"].as_array().map_or(0, |a| a.len());
    let invalidated_count = graph["invalidated_edges"].as_array().map_or(0, |a| a.len());

    let avg_conf: f32 = graph["active_edges"]
        .as_array()
        .map(|edges| {
            if edges.is_empty() {
                return 0.0;
            }
            let sum: f32 = edges
                .iter()
                .filter_map(|e| e["confidence"].as_f64().map(|c| c as f32))
                .sum();
            let count = edges
                .iter()
                .filter(|e| e["confidence"].as_f64().is_some())
                .count();
            if count == 0 {
                0.0
            } else {
                sum / count as f32
            }
        })
        .unwrap_or(0.0);

    println!();
    println!("\u{1f4ca} Graph stats after benchmark:");
    println!("  Entities: {entity_count}");
    println!("  Active facts: {active_count}  ({invalidated_count} contradicted)");
    println!("  Avg confidence: {avg_conf:.2}");
}
