use std::process;

use clap::{Parser, Subcommand, ValueEnum};
use reqwest::blocking::Client;
use serde_json::Value;

// -- CLI definition -----------------------------------------------------------

#[derive(Parser)]
#[command(name = "hippo-cli", about = "CLI client for the hippo natural-language database")]
struct Cli {
    /// Base URL of the hippo server
    #[arg(long, env = "HIPPO_URL", default_value = "http://localhost:21693")]
    url: String,

    /// API key for authentication
    #[arg(long, env = "HIPPO_API_KEY")]
    api_key: Option<String>,

    /// Output format
    #[arg(short, long, default_value = "json")]
    format: OutputFormat,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, ValueEnum)]
enum OutputFormat {
    Json,
    Table,
}

#[derive(Subcommand)]
enum Command {
    /// Manage users
    User {
        #[command(subcommand)]
        action: UserAction,
    },
    /// Manage API keys
    Key {
        #[command(subcommand)]
        action: KeyAction,
    },
    /// Store a statement in the knowledge graph
    Remember {
        /// The natural-language statement to remember
        statement: String,
        /// Target graph name
        #[arg(long)]
        graph: Option<String>,
        /// Source agent identifier
        #[arg(long)]
        source_agent: Option<String>,
    },
    /// Ask a question and get an answer from the knowledge graph
    Ask {
        /// The question to ask
        question: String,
        /// Target graph name
        #[arg(long)]
        graph: Option<String>,
        /// Include supporting facts in the response
        #[arg(long)]
        verbose: bool,
    },
    /// Retrieve context facts for a query
    Context {
        /// The query to search for
        query: String,
        /// Target graph name
        #[arg(long)]
        graph: Option<String>,
        /// Maximum number of facts to return
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Check server health
    Health,
}

#[derive(Subcommand)]
enum UserAction {
    /// Create a new user
    Create {
        user_id: String,
        display_name: String,
        /// User role
        #[arg(long, default_value = "user")]
        role: String,
        /// Comma-separated list of allowed graphs (use * for all)
        #[arg(long, value_delimiter = ',')]
        graphs: Option<Vec<String>>,
    },
    /// List all users
    List,
    /// Delete a user
    Delete { user_id: String },
}

#[derive(Subcommand)]
enum KeyAction {
    /// Create a new API key for a user
    Create { user_id: String, label: String },
    /// List API keys for a user
    List { user_id: String },
    /// Revoke an API key by label
    Revoke { user_id: String, label: String },
}

// -- HTTP helpers -------------------------------------------------------------

struct ApiClient {
    client: Client,
    base_url: String,
    api_key: Option<String>,
}

impl ApiClient {
    fn new(base_url: String, api_key: Option<String>) -> Self {
        Self {
            client: Client::new(),
            base_url,
            api_key,
        }
    }

    fn request(&self, method: Method, path: &str) -> reqwest::blocking::RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        let mut req = match method {
            Method::Get => self.client.get(&url),
            Method::Post => self.client.post(&url),
            Method::Delete => self.client.delete(&url),
        };
        if let Some(ref key) = self.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
        req
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        self.send(self.request(Method::Get, path))
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        self.send(self.request(Method::Post, path).json(body))
    }

    fn delete(&self, path: &str) -> Result<Value, String> {
        self.send(self.request(Method::Delete, path))
    }

    fn send(&self, req: reqwest::blocking::RequestBuilder) -> Result<Value, String> {
        let resp = req.send().map_err(|e| e.to_string())?;
        let status = resp.status();
        let text = resp.text().map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("HTTP {status}: {text}"));
        }
        serde_json::from_str(&text)
            .map_err(|_| format!("invalid JSON response: {text}"))
    }
}

enum Method {
    Get,
    Post,
    Delete,
}

/// Percent-encode a string for use as a URL path segment.
fn encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
}

// -- Output formatting --------------------------------------------------------

fn print_json(value: &Value) {
    println!("{}", serde_json::to_string_pretty(value).unwrap_or_default());
}

fn print_table_rows(headers: &[&str], rows: &[Vec<String>]) {
    let col_count = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(col_count) {
            widths[i] = widths[i].max(cell.len());
        }
    }

    // Header
    let header: String = headers
        .iter()
        .enumerate()
        .map(|(i, h)| format!("{:<width$}", h, width = widths[i]))
        .collect::<Vec<_>>()
        .join("  ");
    println!("{header}");
    let separator: String = widths.iter().map(|w| "-".repeat(*w)).collect::<Vec<_>>().join("  ");
    println!("{separator}");

    // Rows
    for row in rows {
        let line: String = row
            .iter()
            .enumerate()
            .take(col_count)
            .map(|(i, cell)| format!("{:<width$}", cell, width = widths[i]))
            .collect::<Vec<_>>()
            .join("  ");
        println!("{line}");
    }
}

// -- Subcommand dispatch ------------------------------------------------------

fn run(cli: Cli) -> Result<(), String> {
    let api = ApiClient::new(cli.url, cli.api_key);

    match cli.command {
        Command::Health => {
            let val = api.get("/health")?;
            match cli.format {
                OutputFormat::Json => print_json(&val),
                OutputFormat::Table => {
                    let status = val["status"].as_str().unwrap_or("unknown");
                    let graph = val["graph"].as_str().unwrap_or("");
                    print_table_rows(
                        &["STATUS", "GRAPH"],
                        &[vec![status.to_string(), graph.to_string()]],
                    );
                }
            }
        }

        Command::Remember { statement, graph, source_agent } => {
            let mut body = serde_json::json!({ "statement": statement });
            if let Some(g) = graph {
                body["graph"] = Value::String(g);
            }
            if let Some(sa) = source_agent {
                body["source_agent"] = Value::String(sa);
            }
            let val = api.post("/remember", &body)?;
            print_json(&val);
        }

        Command::Ask { question, graph, verbose } => {
            let mut body = serde_json::json!({ "question": question, "verbose": verbose });
            if let Some(g) = graph {
                body["graph"] = Value::String(g);
            }
            let val = api.post("/ask", &body)?;
            print_json(&val);
        }

        Command::Context { query, graph, limit } => {
            let mut body = serde_json::json!({ "query": query });
            if let Some(g) = graph {
                body["graph"] = Value::String(g);
            }
            if let Some(n) = limit {
                body["limit"] = Value::Number(n.into());
            }
            let val = api.post("/context", &body)?;
            print_json(&val);
        }

        Command::User { action } => match action {
            UserAction::Create { user_id, display_name, role, graphs } => {
                let body = serde_json::json!({
                    "user_id": user_id,
                    "display_name": display_name,
                    "role": role,
                    "graphs": graphs.unwrap_or_default(),
                });
                let val = api.post("/admin/users", &body)?;
                print_json(&val);
            }
            UserAction::List => {
                let val = api.get("/admin/users")?;
                match cli.format {
                    OutputFormat::Json => print_json(&val),
                    OutputFormat::Table => {
                        let users = val["users"].as_array();
                        let rows: Vec<Vec<String>> = users
                            .map(|arr| {
                                arr.iter()
                                    .map(|u| {
                                        vec![
                                            u["user_id"].as_str().unwrap_or("").to_string(),
                                            u["display_name"].as_str().unwrap_or("").to_string(),
                                            u["role"].as_str().unwrap_or("").to_string(),
                                            format_string_array(&u["graphs"]),
                                            u["key_count"].as_u64().unwrap_or(0).to_string(),
                                        ]
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        print_table_rows(
                            &["USER_ID", "DISPLAY_NAME", "ROLE", "GRAPHS", "KEYS"],
                            &rows,
                        );
                    }
                }
            }
            UserAction::Delete { user_id } => {
                let path = format!("/admin/users/{}", encode_path(&user_id));
                let val = api.delete(&path)?;
                print_json(&val);
            }
        },

        Command::Key { action } => match action {
            KeyAction::Create { user_id, label } => {
                let path = format!("/admin/users/{}/keys", encode_path(&user_id));
                let body = serde_json::json!({ "label": label });
                let val = api.post(&path, &body)?;
                print_json(&val);
            }
            KeyAction::List { user_id } => {
                let path = format!("/admin/users/{}/keys", encode_path(&user_id));
                let val = api.get(&path)?;
                match cli.format {
                    OutputFormat::Json => print_json(&val),
                    OutputFormat::Table => {
                        let keys = val["keys"].as_array();
                        let rows: Vec<Vec<String>> = keys
                            .map(|arr| {
                                arr.iter()
                                    .map(|k| {
                                        vec![
                                            k["label"].as_str().unwrap_or("").to_string(),
                                            k["created_at"].as_str().unwrap_or("").to_string(),
                                        ]
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        print_table_rows(&["LABEL", "CREATED_AT"], &rows);
                    }
                }
            }
            KeyAction::Revoke { user_id, label } => {
                let path = format!(
                    "/admin/users/{}/keys/{}",
                    encode_path(&user_id),
                    encode_path(&label)
                );
                let val = api.delete(&path)?;
                print_json(&val);
            }
        },
    }

    Ok(())
}

fn format_string_array(val: &Value) -> String {
    match val.as_array() {
        Some(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(","),
        None => String::new(),
    }
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}
