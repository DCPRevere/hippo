use serde::Serialize;

/// Events broadcast to SSE subscribers when the graph is mutated.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GraphEvent {
    EntityCreated {
        id: String,
        name: String,
        entity_type: String,
        graph: String,
    },
    EdgeCreated {
        edge_id: i64,
        from_name: String,
        to_name: String,
        fact: String,
        relation_type: String,
        graph: String,
    },
    EdgeInvalidated {
        edge_id: i64,
        fact: String,
        graph: String,
    },
    RememberComplete {
        graph: String,
        entities_created: usize,
        facts_written: usize,
        contradictions_invalidated: usize,
    },
    EntityDeleted {
        id: String,
        name: String,
        edges_invalidated: usize,
        graph: String,
    },
    MaintenanceComplete {
        graph: String,
    },
}

impl GraphEvent {
    /// Returns the SSE event name (snake_case variant name).
    pub fn event_name(&self) -> &'static str {
        match self {
            GraphEvent::EntityCreated { .. } => "entity_created",
            GraphEvent::EdgeCreated { .. } => "edge_created",
            GraphEvent::EdgeInvalidated { .. } => "edge_invalidated",
            GraphEvent::EntityDeleted { .. } => "entity_deleted",
            GraphEvent::RememberComplete { .. } => "remember_complete",
            GraphEvent::MaintenanceComplete { .. } => "maintenance_complete",
        }
    }

    /// Returns the graph name this event belongs to.
    pub fn graph(&self) -> &str {
        match self {
            GraphEvent::EntityCreated { graph, .. }
            | GraphEvent::EdgeCreated { graph, .. }
            | GraphEvent::EdgeInvalidated { graph, .. }
            | GraphEvent::EntityDeleted { graph, .. }
            | GraphEvent::RememberComplete { graph, .. }
            | GraphEvent::MaintenanceComplete { graph, .. } => graph,
        }
    }
}
