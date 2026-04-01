mod mcp {
    include!("../mcp.rs");
}

use std::io::{BufRead, Write};

use serde_json::Value;

fn main() {
    let base_url = std::env::var("HIPPO_URL")
        .unwrap_or_else(|_| "http://localhost:21693".to_string());
    let api_key = std::env::var("HIPPO_API_KEY").ok();

    let client = reqwest::blocking::Client::new();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request["method"].as_str().unwrap_or("");
        let params = request.get("params").cloned().unwrap_or(Value::Null);

        let response = match method {
            "initialize" => mcp::handle_initialize(id),
            "notifications/initialized" => continue,
            "tools/list" => mcp::handle_tools_list(id),
            "tools/call" => mcp::handle_tool_call(id, params, &client, &base_url, api_key.as_deref()),
            _ => mcp::make_error(id, -32601, "Method not found"),
        };

        let mut out = stdout.lock();
        writeln!(out, "{}", serde_json::to_string(&response).unwrap()).unwrap();
        out.flush().unwrap();
    }
}
