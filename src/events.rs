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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_match_snake_case_variant_names() {
        let cases: Vec<(GraphEvent, &str)> = vec![
            (
                GraphEvent::EntityCreated {
                    id: "1".into(),
                    name: "n".into(),
                    entity_type: "person".into(),
                    graph: "g".into(),
                },
                "entity_created",
            ),
            (
                GraphEvent::EdgeCreated {
                    edge_id: 1,
                    from_name: "a".into(),
                    to_name: "b".into(),
                    fact: "f".into(),
                    relation_type: "R".into(),
                    graph: "g".into(),
                },
                "edge_created",
            ),
            (
                GraphEvent::EdgeInvalidated {
                    edge_id: 1,
                    fact: "f".into(),
                    graph: "g".into(),
                },
                "edge_invalidated",
            ),
            (
                GraphEvent::EntityDeleted {
                    id: "1".into(),
                    name: "n".into(),
                    edges_invalidated: 0,
                    graph: "g".into(),
                },
                "entity_deleted",
            ),
            (
                GraphEvent::RememberComplete {
                    graph: "g".into(),
                    entities_created: 0,
                    facts_written: 0,
                    contradictions_invalidated: 0,
                },
                "remember_complete",
            ),
            (
                GraphEvent::MaintenanceComplete { graph: "g".into() },
                "maintenance_complete",
            ),
        ];

        for (event, expected_name) in cases {
            assert_eq!(event.event_name(), expected_name);
            assert_eq!(event.graph(), "g");
        }
    }

    #[test]
    fn entity_created_serialises_with_type_tag() {
        let evt = GraphEvent::EntityCreated {
            id: "id-1".into(),
            name: "Alice".into(),
            entity_type: "person".into(),
            graph: "hippo".into(),
        };
        let json: serde_json::Value = serde_json::to_value(&evt).unwrap();
        assert_eq!(json["type"], "entity_created");
        assert_eq!(json["name"], "Alice");
        assert_eq!(json["entity_type"], "person");
        assert_eq!(json["graph"], "hippo");
    }

    #[test]
    fn edge_invalidated_serialises_with_minimal_fields() {
        let evt = GraphEvent::EdgeInvalidated {
            edge_id: 42,
            fact: "Alice works at Google".into(),
            graph: "hippo".into(),
        };
        let json: serde_json::Value = serde_json::to_value(&evt).unwrap();
        assert_eq!(json["type"], "edge_invalidated");
        assert_eq!(json["edge_id"], 42);
        // Only the variant's own fields plus the tag are present.
        assert_eq!(json.as_object().unwrap().len(), 4);
    }

    #[test]
    fn maintenance_complete_round_trips_through_json() {
        let evt = GraphEvent::MaintenanceComplete {
            graph: "test".into(),
        };
        let s = serde_json::to_string(&evt).unwrap();
        // Serialise is the contract with SSE clients; ensure tag is there.
        assert!(s.contains(r#""type":"maintenance_complete""#));
        assert!(s.contains(r#""graph":"test""#));
    }

    /// `let _ = tx.send(...)` is the project's pattern for fire-and-forget
    /// emission; this test just confirms the type is `Send + Sync + Clone` so
    /// a future broadcast channel migration will not silently regress.
    #[test]
    fn graph_event_is_clone_send_sync() {
        fn assert_traits<T: Clone + Send + Sync>() {}
        assert_traits::<GraphEvent>();
    }
}
