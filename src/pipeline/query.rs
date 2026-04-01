use anyhow::Result;
use chrono::{DateTime, Utc};
use regex::Regex;

use crate::graph_backend::GraphBackend;
use crate::models::{
    ContextRequest, ReflectRequest, SmartQueryRequest, SmartQueryResponse, TemporalContextRequest,
};
use crate::pipeline::{context, context_temporal, reflect, timeline};
use crate::state::AppState;

pub enum QueryIntent {
    CurrentContext,
    TemporalContext(DateTime<Utc>),
    EntityReflect(String),
    Timeline(String),
    GlobalStats,
}

pub fn classify(query: &str) -> QueryIntent {
    let lower = query.to_lowercase();

    // GlobalStats: "how much do I know?" / "stats" / "summary of memory"
    if lower.contains("how much")
        || lower.contains("stats")
        || lower.contains("summary of memory")
        || lower.contains("what's in memory")
    {
        return QueryIntent::GlobalStats;
    }

    // Timeline: "history of X" / "timeline of X" / "how has X changed" / "evolution of X"
    if lower.contains("history")
        || lower.contains("timeline")
        || lower.contains("changed")
        || lower.contains("evolution of")
    {
        if let Some(entity) = extract_entity_name(query) {
            return QueryIntent::Timeline(entity);
        }
    }

    // TemporalContext: year, "last year", "last month", "as of", "in January"
    if let Some(at) = extract_temporal_hint(query) {
        return QueryIntent::TemporalContext(at);
    }

    // EntityReflect: "tell me about X" / "what do I know about X" / "who is X"
    if let Some(entity) = extract_entity_for_reflect(query) {
        return QueryIntent::EntityReflect(entity);
    }

    QueryIntent::CurrentContext
}

fn extract_temporal_hint(query: &str) -> Option<DateTime<Utc>> {
    let lower = query.to_lowercase();

    // Match 4-digit year (2020-2030 range)
    if let Some(caps) = Regex::new(r"\b(20[2-3]\d)\b")
        .ok()
        .and_then(|re| re.captures(&lower))
    {
        if let Some(year_str) = caps.get(1) {
            if let Ok(year) = year_str.as_str().parse::<i32>() {
                return chrono::NaiveDate::from_ymd_opt(year, 12, 31)
                    .and_then(|d| d.and_hms_opt(23, 59, 59))
                    .map(|dt| dt.and_utc());
            }
        }
    }

    // "last year"
    if lower.contains("last year") {
        return Some(Utc::now() - chrono::Duration::days(365));
    }

    // "last month"
    if lower.contains("last month") {
        return Some(Utc::now() - chrono::Duration::days(30));
    }

    // "as of <month>" or "in <month>" — resolve to end of that month this year
    let months = [
        ("january", 1),
        ("february", 2),
        ("march", 3),
        ("april", 4),
        ("may", 5),
        ("june", 6),
        ("july", 7),
        ("august", 8),
        ("september", 9),
        ("october", 10),
        ("november", 11),
        ("december", 12),
    ];

    if lower.contains("as of") || lower.contains("in ") || lower.contains("before") {
        for (name, month) in &months {
            if lower.contains(name) {
                let year = Utc::now().year();
                // End of that month
                let next_month = if *month == 12 { 1 } else { month + 1 };
                let next_year = if *month == 12 { year + 1 } else { year };
                return chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1)
                    .and_then(|d| d.pred_opt())
                    .and_then(|d| d.and_hms_opt(23, 59, 59))
                    .map(|dt| dt.and_utc());
            }
        }
    }

    // "before <year>"
    if let Some(caps) = Regex::new(r"before\s+(20[2-3]\d)")
        .ok()
        .and_then(|re| re.captures(&lower))
    {
        if let Some(year_str) = caps.get(1) {
            if let Ok(year) = year_str.as_str().parse::<i32>() {
                return chrono::NaiveDate::from_ymd_opt(year - 1, 12, 31)
                    .and_then(|d| d.and_hms_opt(23, 59, 59))
                    .map(|dt| dt.and_utc());
            }
        }
    }

    None
}

fn extract_entity_name(query: &str) -> Option<String> {
    let patterns = [
        r"(?i)tell me about ([A-Z][a-zA-Z\s]+)",
        r"(?i)what do i know about ([A-Z][a-zA-Z\s]+)",
        r"(?i)who is ([A-Z][a-zA-Z\s]+)",
        r"(?i)history of ([A-Z][a-zA-Z\s]+)",
        r"(?i)timeline of ([A-Z][a-zA-Z\s]+)",
        r"(?i)evolution of ([A-Z][a-zA-Z\s]+)",
        r"(?i)how has ([A-Z][a-zA-Z\s]+) changed",
    ];

    for pattern in &patterns {
        if let Ok(re) = Regex::new(pattern) {
            if let Some(caps) = re.captures(query) {
                return caps.get(1).map(|m| m.as_str().trim().to_string());
            }
        }
    }
    None
}

fn extract_entity_for_reflect(query: &str) -> Option<String> {
    let patterns = [
        r"(?i)tell me about ([A-Z][a-zA-Z\s]+)",
        r"(?i)what do i know about ([A-Z][a-zA-Z\s]+)",
        r"(?i)who is ([A-Z][a-zA-Z\s]+)",
    ];

    for pattern in &patterns {
        if let Ok(re) = Regex::new(pattern) {
            if let Some(caps) = re.captures(query) {
                return caps.get(1).map(|m| m.as_str().trim().to_string());
            }
        }
    }
    None
}

use chrono::Datelike;

pub async fn smart_query(state: &AppState, graph: &dyn GraphBackend, req: SmartQueryRequest) -> Result<SmartQueryResponse> {
    let intent = classify(&req.query);
    let limit = req.limit.unwrap_or(state.config.default_context_limit);

    match intent {
        QueryIntent::CurrentContext => {
            let ctx_req = ContextRequest {
                query: req.query,
                limit: Some(limit),
                max_hops: None,
                memory_tier_filter: None,
                graph: req.graph,
                at: None,
            };
            let resp = context::context(state, graph, ctx_req, None).await?;
            Ok(SmartQueryResponse {
                intent: "current_context".to_string(),
                facts: resp.facts,
                reflect: None,
                timeline: None,
                stats: None,
                routed_to: "POST /context".to_string(),
            })
        }

        QueryIntent::TemporalContext(at) => {
            let ctx_req = TemporalContextRequest {
                query: req.query,
                at,
                limit: Some(limit),
                graph: req.graph,
            };
            let resp = context_temporal::context_temporal(state, graph, ctx_req).await?;
            Ok(SmartQueryResponse {
                intent: "temporal_context".to_string(),
                facts: resp.facts,
                reflect: None,
                timeline: None,
                stats: None,
                routed_to: "POST /context/temporal".to_string(),
            })
        }

        QueryIntent::EntityReflect(entity) => {
            let reflect_req = ReflectRequest {
                about: Some(entity),
                suggest_questions: Some(true),
                graph: req.graph,
            };
            let resp = reflect::reflect(state, graph, reflect_req).await?;
            Ok(SmartQueryResponse {
                intent: "entity_reflect".to_string(),
                facts: vec![],
                reflect: Some(resp),
                timeline: None,
                stats: None,
                routed_to: "POST /reflect".to_string(),
            })
        }

        QueryIntent::Timeline(entity) => {
            let resp = timeline::timeline(state, graph, &entity).await?;
            Ok(SmartQueryResponse {
                intent: "timeline".to_string(),
                facts: vec![],
                reflect: None,
                timeline: Some(resp),
                stats: None,
                routed_to: "GET /timeline/:entity".to_string(),
            })
        }

        QueryIntent::GlobalStats => {
            let reflect_req = ReflectRequest {
                about: None,
                suggest_questions: None,
                graph: req.graph,
            };
            let resp = reflect::reflect(state, graph, reflect_req).await?;
            Ok(SmartQueryResponse {
                intent: "global_stats".to_string(),
                facts: vec![],
                reflect: None,
                timeline: None,
                stats: resp.stats,
                routed_to: "POST /reflect".to_string(),
            })
        }
    }
}
