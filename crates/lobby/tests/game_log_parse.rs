// Integration check: simulate the lobby side reading a JSON line produced
// by `game_sdk::game_log!` and verify it parses into a structured entry.
use serde_json::Value;

#[derive(Debug, serde::Deserialize)]
struct GameLogEntry {
    ts: String,
    level: String,
    target: String,
    message: String,
    #[serde(default)]
    fields: serde_json::Map<String, Value>,
}

fn parse(line: &str) -> Result<GameLogEntry, String> {
    serde_json::from_str::<GameLogEntry>(line).map_err(|e| e.to_string())
}

#[test]
fn parses_basic_info_log() {
    let line = r#"{"ts":"2026-08-17T06:18:01.123456Z","level":"info","target":"take_your_position::rules","message":"apply_posterior accepted","fields":{"uid":42,"rank_count":5}}"#;
    let entry = parse(line).expect("must parse");
    assert_eq!(entry.level, "info");
    assert_eq!(entry.target, "take_your_position::rules");
    assert_eq!(entry.message, "apply_posterior accepted");
    assert_eq!(entry.fields.get("uid"), Some(&Value::from(42)));
    assert_eq!(entry.fields.get("rank_count"), Some(&Value::from(5)));
}

#[test]
fn parses_warn_with_no_fields() {
    let line = r#"{"ts":"2026-08-17T06:18:01Z","level":"warn","target":"foo::bar","message":"oh no"}"#;
    let entry = parse(line).expect("must parse");
    assert_eq!(entry.level, "warn");
    assert_eq!(entry.target, "foo::bar");
    assert_eq!(entry.message, "oh no");
    assert!(entry.fields.is_empty());
}

#[test]
fn parses_with_string_field_values() {
    let line = r#"{"ts":"2026-08-17T06:18:01Z","level":"info","target":"x","message":"m","fields":{"phase":"play","seat":"0"}}"#;
    let entry = parse(line).unwrap();
    assert_eq!(entry.fields["phase"], "play");
    assert_eq!(entry.fields["seat"], "0");
}

#[test]
fn rejects_non_json_line() {
    let line = "2026-08-17T06:18:01 INFO lobby::game_stderr: hello";
    let err = parse(line).expect_err("non-JSON line should fail");
    // Should fall through to plain-text handler in the lobby.
    assert!(!err.is_empty());
}

#[test]
fn rejects_empty_line() {
    // The lobby's emit_game_log trims and skips empty lines before
    // calling serde_json::from_str, so we don't need to handle this.
    // But the parser itself should return an error for "".
    assert!(parse("").is_err());
}

#[test]
fn rejects_malformed_json() {
    assert!(parse("{not json").is_err());
    assert!(parse(r#"{"missing": "close"}"#).is_err());
}
