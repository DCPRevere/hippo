//! Helpers for extracting typed Rust values out of `FalkorValue`s returned by
//! the FalkorDB driver, and for converting nodes/edges to the row types used
//! by the rest of the codebase.

use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Utc};
use falkordb::FalkorValue;

use crate::models::{EdgeRow, EntityRow, SupersessionRecord, EMBEDDING_DIM};

pub(crate) fn node_to_entity_row(v: FalkorValue) -> Option<Result<EntityRow>> {
    match v {
        FalkorValue::Node(node) => {
            let p = &node.properties;
            Some(Ok(EntityRow {
                id: prop_string(p, "id"),
                name: prop_string(p, "name"),
                entity_type: prop_string(p, "entity_type"),
                resolved: prop_bool(p, "resolved"),
                hint: prop_opt_string(p, "hint"),
                content: prop_opt_string(p, "content"),
                created_at: prop_string(p, "created_at"),
                embedding: prop_embedding(p, "embedding"),
            }))
        }
        _ => None,
    }
}

pub(crate) fn edge_row_from_values(
    rel: FalkorValue,
    src: FalkorValue,
    dst: FalkorValue,
) -> Result<EdgeRow> {
    let (
        edge_id,
        fact,
        relation_type,
        confidence,
        salience,
        valid_at,
        invalid_at,
        embedding,
        decayed_confidence,
        source_agents,
        memory_tier,
        expires_at,
    ) = match &rel {
        FalkorValue::Edge(edge) => {
            let p = &edge.properties;
            let confidence = prop_float(p, "confidence");
            let valid_at = prop_string(p, "valid_at");
            let decayed_confidence = {
                let v = prop_float(p, "decayed_confidence");
                if v == 0.0 {
                    confidence
                } else {
                    v
                }
            };
            let source_agents = prop_opt_string(p, "source_agents").unwrap_or_default();
            let memory_tier =
                prop_opt_string(p, "memory_tier").unwrap_or_else(|| "long_term".to_string());
            let expires_at = prop_opt_string(p, "expires_at");
            (
                edge.entity_id,
                prop_string(p, "fact"),
                prop_string(p, "relation_type"),
                confidence,
                prop_int(p, "salience"),
                valid_at,
                prop_opt_string(p, "invalid_at"),
                prop_embedding(p, "embedding"),
                decayed_confidence,
                source_agents,
                memory_tier,
                expires_at,
            )
        }
        _ => anyhow::bail!("expected Edge value"),
    };

    let (subject_id, subject_name) = match &src {
        FalkorValue::Node(node) => {
            let p = &node.properties;
            (prop_string(p, "id"), prop_string(p, "name"))
        }
        _ => (String::new(), String::new()),
    };

    let (object_id, object_name) = match &dst {
        FalkorValue::Node(node) => {
            let p = &node.properties;
            (prop_string(p, "id"), prop_string(p, "name"))
        }
        _ => (String::new(), String::new()),
    };

    Ok(EdgeRow {
        edge_id,
        subject_id,
        subject_name,
        fact,
        relation_type,
        confidence,
        salience,
        valid_at,
        invalid_at,
        object_id,
        object_name,
        embedding,
        decayed_confidence,
        source_agents,
        memory_tier,
        expires_at,
    })
}

pub(crate) fn prop_string(p: &HashMap<String, FalkorValue>, key: &str) -> String {
    p.get(key).map(extract_string).unwrap_or_default()
}

pub(crate) fn prop_opt_string(p: &HashMap<String, FalkorValue>, key: &str) -> Option<String> {
    p.get(key).and_then(|v| match v {
        FalkorValue::None => None,
        FalkorValue::String(s) if s.is_empty() => None,
        other => Some(extract_string(other)),
    })
}

pub(crate) fn prop_bool(p: &HashMap<String, FalkorValue>, key: &str) -> bool {
    p.get(key).map(extract_bool).unwrap_or(false)
}

pub(crate) fn prop_float(p: &HashMap<String, FalkorValue>, key: &str) -> f32 {
    p.get(key).map(extract_float).unwrap_or(0.0)
}

pub(crate) fn prop_int(p: &HashMap<String, FalkorValue>, key: &str) -> i64 {
    p.get(key).map(extract_int).unwrap_or(0)
}

pub(crate) fn prop_embedding(p: &HashMap<String, FalkorValue>, key: &str) -> Vec<f32> {
    p.get(key)
        .map(extract_embedding)
        .unwrap_or_else(|| vec![0.0; EMBEDDING_DIM])
}

pub(crate) fn extract_string(v: &FalkorValue) -> String {
    match v {
        FalkorValue::String(s) => s.clone(),
        FalkorValue::I64(i) => i.to_string(),
        FalkorValue::F64(f) => f.to_string(),
        FalkorValue::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

pub(crate) fn extract_bool(v: &FalkorValue) -> bool {
    match v {
        FalkorValue::Bool(b) => *b,
        FalkorValue::String(s) => s == "true",
        FalkorValue::I64(i) => *i != 0,
        _ => false,
    }
}

pub(crate) fn extract_int(v: &FalkorValue) -> i64 {
    match v {
        FalkorValue::I64(i) => *i,
        FalkorValue::F64(f) => *f as i64,
        _ => 0,
    }
}

pub(crate) fn extract_float(v: &FalkorValue) -> f32 {
    match v {
        FalkorValue::F64(f) => *f as f32,
        FalkorValue::I64(i) => *i as f32,
        _ => 0.0,
    }
}

pub(crate) fn extract_embedding(v: &FalkorValue) -> Vec<f32> {
    match v {
        FalkorValue::Array(arr) => arr.iter().map(extract_float).collect(),
        FalkorValue::Vec32(v32) => v32.values.clone(),
        _ => vec![0.0; EMBEDDING_DIM],
    }
}

/// Destructure a row into a fixed-size array, bailing if too short.
pub(crate) fn take_n<const N: usize>(row: Vec<FalkorValue>) -> Result<[FalkorValue; N]> {
    row.try_into()
        .map_err(|v: Vec<FalkorValue>| anyhow::anyhow!("expected {N} columns, got {}", v.len()))
}

pub(crate) fn supersession_from_row(row: Vec<FalkorValue>) -> Result<SupersessionRecord> {
    let [a, b, c, d, e] = take_n(row)?;
    let old_edge_id = extract_int(&a);
    let new_edge_id = extract_int(&b);
    let superseded_at_str = extract_string(&c);
    let old_fact = extract_string(&d);
    let new_fact = extract_string(&e);
    let superseded_at = superseded_at_str
        .parse::<DateTime<Utc>>()
        .unwrap_or_else(|_| Utc::now());
    Ok(SupersessionRecord {
        old_edge_id,
        new_edge_id,
        superseded_at,
        old_fact,
        new_fact,
    })
}
