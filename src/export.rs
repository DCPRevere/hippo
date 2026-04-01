use crate::models::{EdgeRow, EntityRow};

/// Render entities and edges as GraphML XML.
pub fn to_graphml(entities: &[EntityRow], edges: &[EdgeRow]) -> String {
    let mut buf = String::new();
    buf.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    buf.push_str("<graphml xmlns=\"http://graphml.graphstruct.org/xmlns\">\n");
    buf.push_str("  <key id=\"type\" for=\"node\" attr.name=\"type\" attr.type=\"string\"/>\n");
    buf.push_str("  <graph id=\"G\" edgedefault=\"directed\">\n");

    for entity in entities {
        buf.push_str(&format!(
            "    <node id=\"{}\"><data key=\"type\">{}</data></node>\n",
            xml_escape(&entity.id),
            xml_escape(&entity.entity_type),
        ));
    }

    for edge in edges {
        buf.push_str(&format!(
            "    <edge source=\"{}\" target=\"{}\"><data key=\"fact\">{}</data></edge>\n",
            xml_escape(&edge.subject_id),
            xml_escape(&edge.object_id),
            xml_escape(&edge.fact),
        ));
    }

    buf.push_str("  </graph>\n");
    buf.push_str("</graphml>\n");
    buf
}

/// Render entities and edges as CSV (entities section, blank line, edges section).
pub fn to_csv(entities: &[EntityRow], edges: &[EdgeRow]) -> String {
    let mut buf = String::new();

    // Entities header
    buf.push_str("id,name,entity_type,resolved,hint,created_at\n");
    for e in entities {
        buf.push_str(&format!(
            "{},{},{},{},{},{}\n",
            csv_escape(&e.id),
            csv_escape(&e.name),
            csv_escape(&e.entity_type),
            e.resolved,
            csv_escape(e.hint.as_deref().unwrap_or("")),
            csv_escape(&e.created_at),
        ));
    }

    // Blank line separator
    buf.push('\n');

    // Edges header
    buf.push_str("edge_id,subject_id,object_id,fact,relation_type,confidence,salience,valid_at,invalid_at,source_agents,memory_tier\n");
    for e in edges {
        buf.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{}\n",
            e.edge_id,
            csv_escape(&e.subject_id),
            csv_escape(&e.object_id),
            csv_escape(&e.fact),
            csv_escape(&e.relation_type),
            e.confidence,
            e.salience,
            csv_escape(&e.valid_at),
            csv_escape(e.invalid_at.as_deref().unwrap_or("")),
            csv_escape(&e.source_agents),
            csv_escape(&e.memory_tier),
        ));
    }

    buf
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entities() -> Vec<EntityRow> {
        vec![EntityRow {
            id: "e1".to_string(),
            name: "Alice".to_string(),
            entity_type: "person".to_string(),
            resolved: true,
            hint: Some("test hint".to_string()),
            content: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            embedding: vec![],
        }]
    }

    fn sample_edges() -> Vec<EdgeRow> {
        vec![EdgeRow {
            edge_id: 1,
            subject_id: "e1".to_string(),
            subject_name: "Alice".to_string(),
            fact: "Alice knows Bob".to_string(),
            relation_type: "knows".to_string(),
            confidence: 0.9,
            salience: 5,
            valid_at: "2024-01-01T00:00:00Z".to_string(),
            invalid_at: None,
            object_id: "e2".to_string(),
            object_name: "Bob".to_string(),
            embedding: vec![],
            decayed_confidence: 0.9,
            source_agents: "seed".to_string(),
            memory_tier: "long_term".to_string(),
            expires_at: None,
        }]
    }

    #[test]
    fn graphml_contains_nodes_and_edges() {
        let xml = to_graphml(&sample_entities(), &sample_edges());
        assert!(xml.contains("<node id=\"e1\">"));
        assert!(xml.contains("<data key=\"type\">person</data>"));
        assert!(xml.contains("<edge source=\"e1\" target=\"e2\">"));
        assert!(xml.contains("<data key=\"fact\">Alice knows Bob</data>"));
        assert!(xml.starts_with("<?xml"));
    }

    #[test]
    fn graphml_escapes_special_chars() {
        let entities = vec![EntityRow {
            id: "e&1".to_string(),
            name: "A<B".to_string(),
            entity_type: "t\"1".to_string(),
            resolved: true,
            hint: None,
            content: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            embedding: vec![],
        }];
        let xml = to_graphml(&entities, &[]);
        assert!(xml.contains("e&amp;1"));
        assert!(xml.contains("t&quot;1"));
    }

    #[test]
    fn csv_has_two_sections() {
        let csv = to_csv(&sample_entities(), &sample_edges());
        let sections: Vec<&str> = csv.split("\n\n").collect();
        assert_eq!(sections.len(), 2, "expected two sections separated by blank line");
        assert!(sections[0].starts_with("id,name,"));
        assert!(sections[1].starts_with("edge_id,subject_id,"));
    }

    #[test]
    fn csv_escapes_commas() {
        let entities = vec![EntityRow {
            id: "e1".to_string(),
            name: "Alice, Bob".to_string(),
            entity_type: "person".to_string(),
            resolved: true,
            hint: None,
            content: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            embedding: vec![],
        }];
        let csv = to_csv(&entities, &[]);
        assert!(csv.contains("\"Alice, Bob\""));
    }
}
