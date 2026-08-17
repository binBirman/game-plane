// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Structured log emission protocol for game binaries.
//
// Games emit one JSON object per line to stderr. The lobby reads each
// line, parses it, and re-emits it through `tracing` with the original
// level / target / fields. Non-JSON lines fall through to a plain-text
// handler (`lobby::game_stderr`) for backwards compatibility with the
// existing `tracing` direct output.
//
// Wire format (one JSON object per line, UTF-8, no trailing whitespace):
//
//   {"ts":"2026-08-17T06:18:01.123456Z",
//    "level":"info",
//    "target":"take_your_position::rules",
//    "message":"apply_posterior accepted",
//    "fields":{"uid":42,"rank_count":5}}
//
// All fields except `fields` are required. `fields` is a free-form JSON
// object whose values are primitive (string / number / bool / null / array
// / nested object). The lobby inlines the fields into the tracing message
// as `key=value` pairs so they survive JSON-stringification in the journal.

use std::io::Write;

use serde::Serialize;

/// Standard log levels. Names match `tracing` and
/// `tracing_subscriber::EnvFilter`, so a `RUST_LOG=info` filter on the
/// lobby side matches the level emitted by the game.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Trace => "trace",
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

/// Wire format for a single structured log line.
///
/// Fields are stored as a `serde_json::Map` so the JSON object preserves
/// insertion order (important for grep ergonomics).
#[derive(Debug, Serialize)]
pub struct LogEntry<'a> {
    pub ts: &'a str,
    pub level: &'a str,
    pub target: &'a str,
    pub message: &'a str,
    pub fields: &'a serde_json::Map<String, serde_json::Value>,
}

/// Emit a single structured log line to stderr. Safe to call from
/// multiple threads — writes are serialized via `stderr().lock()`.
/// `init_tracing()` is NOT required for this to work.
pub fn emit(
    level: Level,
    target: &str,
    message: &str,
    fields: &serde_json::Map<String, serde_json::Value>,
) {
    let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
    let entry = LogEntry {
        ts: &ts,
        level: level.as_str(),
        target,
        message,
        fields,
    };
    let Ok(s) = serde_json::to_string(&entry) else { return };
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{}", s);
    let _ = stderr.flush();
}

/// Convenience: build a `serde_json::Map` from key/value pairs. The lobby
/// re-emits these inline into the tracing message, so keep keys short
/// and snake_case.
pub fn fields(
    pairs: impl IntoIterator<Item = (impl Into<String>, serde_json::Value)>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    for (k, v) in pairs {
        m.insert(k.into(), v);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_as_str() {
        assert_eq!(Level::Info.as_str(), "info");
        assert_eq!(Level::Error.as_str(), "error");
        assert_eq!(Level::Debug.as_str(), "debug");
        assert_eq!(Level::Warn.as_str(), "warn");
        assert_eq!(Level::Trace.as_str(), "trace");
    }

    #[test]
    fn log_entry_serializes_with_insertion_order() {
        let mut fields = serde_json::Map::new();
        fields.insert("uid".into(), serde_json::json!(42));
        fields.insert("rank_count".into(), serde_json::json!(5));
        let entry = LogEntry {
            ts: "2026-08-17T06:18:01.123456Z",
            level: "info",
            target: "take_your_position::rules",
            message: "apply_posterior accepted",
            fields: &fields,
        };
        let s = serde_json::to_string(&entry).unwrap();
        assert!(s.contains("\"ts\":\"2026-08-17T06:18:01.123456Z\""), "got: {s}");
        assert!(s.contains("\"level\":\"info\""), "got: {s}");
        assert!(s.contains("\"target\":\"take_your_position::rules\""), "got: {s}");
        assert!(s.contains("\"message\":\"apply_posterior accepted\""), "got: {s}");
        assert!(s.contains("\"uid\":42"), "got: {s}");
        assert!(s.contains("\"rank_count\":5"), "got: {s}");
        // uid inserted before rank_count — preserve order through serde_json.
        let uid_pos = s.find("\"uid\"").unwrap();
        let rank_pos = s.find("\"rank_count\"").unwrap();
        assert!(uid_pos < rank_pos, "uid should appear before rank_count: {s}");
    }

    #[test]
    fn fields_helper_inserts_pairs() {
        let m = fields([
            ("a", serde_json::json!(1)),
            ("b", serde_json::json!("two")),
        ]);
        assert_eq!(m.get("a"), Some(&serde_json::json!(1)));
        assert_eq!(m.get("b"), Some(&serde_json::json!("two")));
    }
}
