mod helpers;

use std::time::Duration;

use helpers::{fact_strings, load_base_facts, query_facts, remember, start_agent};

#[tokio::test]
#[ignore = "requires real LLM and network; run with `cargo test --test integration_test -- --ignored`"]
async fn test_recall_and_contradiction() {
    let agent = start_agent().await;
    let (client, base) = (&agent.client, agent.base_url.as_str());

    load_base_facts(client, base).await;

    println!("\n=== Recall queries ===");
    for q in &[
        "Alice",
        "wedding",
        "doctor",
        "medical bill",
        "bank account",
        "David Carol",
    ] {
        let facts = query_facts(client, base, q, 8).await;
        let strings = fact_strings(&facts);
        println!("\nQuery: {:?} ({} results)", q, strings.len());
        for (i, f) in strings.iter().enumerate() {
            println!("  {}. {}", i + 1, f);
        }
    }

    println!("\n=== Contradiction: changing doctor ===");
    remember(
        client,
        base,
        "My doctor is now Dr. Jones at Riverside Clinic",
    )
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;

    let facts = query_facts(client, base, "my doctor", 5).await;
    let strings = fact_strings(&facts);
    println!("\nQuery: \"my doctor\" ({} results)", strings.len());
    for (i, f) in strings.iter().enumerate() {
        println!("  {}. {}", i + 1, f);
    }

    println!("\n=== Placeholder resolution ===");
    remember(
        client,
        base,
        "John's wife called to reschedule my appointment",
    )
    .await;
    remember(client, base, "John's wife is named Sarah").await;
    tokio::time::sleep(Duration::from_secs(3)).await;

    let facts = query_facts(client, base, "Sarah", 5).await;
    let strings = fact_strings(&facts);
    println!("\nQuery: \"Sarah\" ({} results)", strings.len());
    for (i, f) in strings.iter().enumerate() {
        println!("  {}. {}", i + 1, f);
    }
}
