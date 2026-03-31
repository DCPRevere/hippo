use anyhow::Result;
use chrono::Utc;

use crate::graph_backend::GraphBackend;
use crate::models::{TimelineEvent, TimelineResponse};
use crate::state::AppState;

pub async fn timeline(state: &AppState, graph: &dyn GraphBackend, entity_name: &str) -> Result<TimelineResponse> {
    tracing::info!(entity = %entity_name, "timeline: query");

    let edges = graph.entity_timeline(entity_name).await?;

    let events: Vec<TimelineEvent> = edges
        .into_iter()
        .map(|edge| {
            let valid_at = chrono::DateTime::parse_from_rfc3339(&edge.valid_at)
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            let invalid_at = edge.invalid_at.as_ref().and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(s)
                    .map(|t| t.with_timezone(&Utc))
                    .ok()
            });
            let superseded = invalid_at.is_some();
            TimelineEvent {
                fact: edge.fact,
                relation_type: edge.relation_type,
                valid_at,
                invalid_at,
                superseded,
            }
        })
        .collect();

    Ok(TimelineResponse {
        entity: entity_name.to_string(),
        events,
    })
}
