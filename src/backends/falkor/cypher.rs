//! Cypher string-building helpers for the FalkorDB backend.
//!
//! ## Why we hand-escape instead of binding parameters
//!
//! Other backends in this crate use real parameter binding (sqlx for Postgres,
//! rusqlite for SQLite). FalkorDB's Rust driver (`falkordb-rs` 0.2) exposes
//! `QueryBuilder::with_params(HashMap<String, String>)`, which sounds like
//! parameter binding but is actually a Cypher `CYPHER {key}={value}` prefix
//! substitution: the driver concatenates the params into the query string and
//! relies on the server-side Cypher parser to bind them. Crucially, **the
//! driver does not escape the value strings** — it just splices them in.
//!
//! That means:
//!
//!   - User-controlled strings (entity names, agent ids, fact text) still
//!     need to be escaped at the call site before being interpolated into
//!     the query.
//!   - The escape only needs to defend the single-quoted Cypher string
//!     literal that wraps the value. [`sanitise`] handles the five characters
//!     that can break out of that literal: `\`, `'`, `\n`, `\r`, `\0`.
//!   - It does *not* defend the surrounding query structure. Identifiers
//!     (label names, relationship types) must come from trusted constants,
//!     never from user input.
//!
//! Migrating to true parameter binding would require either an upstream
//! change to falkordb-rs or switching to a different driver. Until then, all
//! user-controlled values that flow into a Cypher query must pass through
//! [`sanitise`].

use std::collections::HashSet;
use std::sync::LazyLock;

/// Common English stop words skipped during fulltext-query construction so
/// they don't dilute scoring.
pub(crate) static STOP_WORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "a", "an", "the", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
        "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can",
        "my", "your", "his", "her", "its", "our", "their", "i", "me", "we", "you", "he", "she",
        "they", "it", "to", "of", "in", "for", "on", "with", "at", "by", "from", "and", "or",
        "but", "not", "no", "about", "what", "where", "when", "who", "how", "which", "that",
        "this", "these", "those",
    ]
    .into_iter()
    .collect()
});

/// Escape a string for inclusion inside a single-quoted Cypher string literal.
///
/// Handles every character that could allow injection or truncate the string
/// literal:
///
///   - backslash (start of an escape sequence)
///   - single quote (closes the literal)
///   - newline / carriage return (some parsers terminate strings on newlines)
///   - NUL (terminates the redis bulk string in transport)
pub(crate) fn sanitise(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            c => out.push(c),
        }
    }
    out
}

/// Strip the timezone suffix from an RFC 3339 timestamp so it can be used
/// with FalkorDB's `localdatetime()` or plain string comparison. Returns the
/// first 19 characters: `YYYY-MM-DDTHH:MM:SS`.
pub(crate) fn strip_tz(rfc3339: &str) -> &str {
    &rfc3339[..19]
}

/// Build a Cypher `WITH` clause that computes an approximate number of days
/// between two `localdatetime` expressions.
///
/// FalkorDB lacks `duration.between()`, so we approximate using year, month,
/// and day components. The result is bound to `{alias}`.
pub(crate) fn approx_days_clause(accessed_expr: &str, now_expr: &str, alias: &str) -> String {
    format!(
        "toFloat(({now_expr}.year - {accessed_expr}.year) * 365 \
              + ({now_expr}.month - {accessed_expr}.month) * 30 \
              + ({now_expr}.day - {accessed_expr}.day)) AS {alias}"
    )
}

/// Render a vector as a Cypher `vecf32(...)` literal.
pub(crate) fn vec_literal(v: &[f32]) -> String {
    let inner = v
        .iter()
        .map(|f| format!("{:.6}", f))
        .collect::<Vec<_>>()
        .join(", ");
    format!("vecf32([{inner}])")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_for_safe_input() {
        assert_eq!(sanitise("Alice"), "Alice");
        assert_eq!(sanitise("a-b_c.d 42"), "a-b_c.d 42");
    }

    #[test]
    fn escapes_single_quote() {
        assert_eq!(sanitise("O'Brien"), r"O\'Brien");
    }

    #[test]
    fn escapes_backslash_before_quote_to_avoid_double_unescape() {
        // Naively escaping `'` first would turn `\'` into `\\'` which the
        // parser then unescapes back to `'`, breaking out of the string.
        // Order matters: backslashes are escaped first.
        assert_eq!(sanitise("\\'"), "\\\\\\'");
    }

    #[test]
    fn escapes_newlines_and_returns() {
        assert_eq!(sanitise("a\nb"), "a\\nb");
        assert_eq!(sanitise("a\rb"), "a\\rb");
    }

    #[test]
    fn escapes_nul_byte() {
        assert_eq!(sanitise("a\0b"), "a\\0b");
    }

    #[test]
    fn classic_injection_attempt_neutralised() {
        let attack = "x'}) RETURN n; //";
        let escaped = sanitise(attack);
        let bytes = escaped.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'\'' {
                assert!(
                    i > 0 && bytes[i - 1] == b'\\',
                    "unescaped quote at {i}: {escaped}"
                );
            }
        }
    }

    #[test]
    fn strip_tz_takes_first_19_chars() {
        assert_eq!(strip_tz("2026-04-28T12:34:56+00:00"), "2026-04-28T12:34:56");
        assert_eq!(strip_tz("2026-04-28T12:34:56Z"), "2026-04-28T12:34:56");
    }

    #[test]
    fn vec_literal_uses_six_decimal_places() {
        assert_eq!(vec_literal(&[1.0, 0.5]), "vecf32([1.000000, 0.500000])");
    }

    #[test]
    fn vec_literal_handles_empty() {
        assert_eq!(vec_literal(&[]), "vecf32([])");
    }

    #[test]
    fn approx_days_clause_renders_alias_and_components() {
        let s = approx_days_clause("a", "b", "days");
        assert!(s.contains("AS days"));
        assert!(s.contains("(b.year - a.year)"));
        assert!(s.contains("(b.month - a.month)"));
        assert!(s.contains("(b.day - a.day)"));
    }

    #[test]
    fn stop_words_includes_common_terms() {
        assert!(STOP_WORDS.contains("the"));
        assert!(STOP_WORDS.contains("a"));
        assert!(!STOP_WORDS.contains("alice"));
    }
}
