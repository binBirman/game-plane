# Game Log Protocol

## Purpose

Game binaries (`take_your_position`, `tictactoe`, and any future games) need a
way to emit structured runtime logs to the operator, **without** disturbing the
JSON-line protocol that runs on stdout (`GameEvent` / `GameCommand`).

The protocol is a single, narrow contract:

- **Stderr** carries one JSON object per line.
- **Stdout** stays clean for `GameEvent` / `GameCommand`.

The lobby reads each stderr line, classifies it (structured vs. plain text),
and re-emits it through `tracing` so that everything ends up in the lobby's
journal under the lobby's `LogFormat` (JSON or Text).

This file is the normative spec. Game authors and lobby maintainers must
keep the two ends in sync.

## Wire format

UTF-8, **one JSON object per line**, no trailing whitespace, no embedded
newlines. Lines must not exceed 64 KiB — the lobby drops anything longer.

```json
{
  "ts": "2026-08-17T06:18:01.123456Z",
  "level": "info",
  "target": "take_your_position::rules",
  "message": "apply_posterior accepted",
  "fields": { "uid": 42, "rank_count": 5 }
}
```

| Field    | Type    | Required | Description                                                                                  |
| -------- | ------- | -------- | -------------------------------------------------------------------------------------------- |
| `ts`     | string  | yes      | RFC 3339 UTC timestamp with microsecond precision. Schema matches `chrono::SecondsFormat::Micros`.|
| `level`  | string  | yes      | One of `trace`, `debug`, `info`, `warn`, `error`. Names match `tracing::Level` so the lobby's `RUST_LOG` filter continues to work. |
| `target` | string  | yes      | Module path / component name. Convention: `game_crate::module`, e.g. `take_your_position::rules`. Lobby re-emits this as the `tracing` target. |
| `message`| string  | yes      | Human-readable one-line summary. Lobby inlines `fields` into this string on the tracing side. |
| `fields` | object  | no       | Free-form structured key/value pairs. Values may be string, number, bool, null, array, or nested object. Lobby flattens this into `key=value` pairs in the re-emitted tracing message. |

### Field conventions

- Keys are `snake_case` to match Rust naming.
- Bools serialize as JSON `true`/`false`.
- Numbers are JSON numbers (no quoting).
- Strings are JSON strings (quoted).
- Keep keys short: `uid`, `seat`, `rank`, `phase`, `round`, `instance_id`, `room_id`.
- Avoid nesting deeper than one level — the lobby inlines fields into a
  single line, so `[a, b, c]` becomes `field=[a, b, c]` and is hard to grep.

### Reserved / discouraged targets

- `lobby::*` — the lobby already owns this namespace. Don't use it from
  a game crate.
- `game_sdk::*` — the SDK does not emit structured logs; it uses
  `tracing::info!` for its own status messages, which fall through to
  the plain-text fallback (`lobby::game_stderr`).

## Client API (game-sdk)

`game-sdk` provides a **`game_log!` macro** at the crate root:

```rust
use game_sdk::game_log;

game_log!(info, "apply_posterior accepted", uid = 42, rank_count = 5);
game_log!(warn, "out of time", uid = 42, phase = "play");
game_log!(error, "invalid rank_list", rank_count = 0);
game_log!(debug, "snapshot built", players = 5);
```

It expands to a single `game_sdk::log::emit()` call. The macro:

1. Sets `target` to `module_path!()` so each log line identifies the
   emitting module.
2. Builds a `serde_json::Map<String, Value>` from the `key = value` pairs.
3. Writes one JSON line to stderr (acquired via `stderr().lock()` so
   concurrent threads can't interleave).

`init_tracing()` is **not** required for `game_log!` to work. The two
are independent: `tracing::info!` and friends still go through the
`tracing_subscriber`; `game_log!` writes JSON directly.

### Why `stderr`?

Stdout is reserved for the `GameEvent` JSON-line protocol
(`Ready` / `Running` / `Finished` / `Action` / `Heartbeat` / `Shutdown`).
The lobby parses stdout line by line and trips over anything that
isn't a `GameEvent`. Mixing logs into stdout would break the lobby's
state machine.

Stderr is the conventional "free" stream in CLI tools; the lobby
reads it independently and tolerates any content.

## Lobby side

The lobby reads stderr from the game subprocess via `BufReader::lines()`
in `crates/lobby/src/instance/manager.rs`. Each line is passed to
`emit_game_log(line, instance_id)`:

```rust
fn emit_game_log(line: &str, instance_id: i64) {
    let trimmed = line.trim();
    if trimmed.is_empty() { return; }

    if let Ok(entry) = serde_json::from_str::<GameLogEntry>(trimmed) {
        // Format fields as `key=value` and append to the message.
        let fields_str = entry.fields.iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join(" ");
        let full_msg = if fields_str.is_empty() {
            entry.message
        } else {
            format!("{} {}", entry.message, fields_str)
        };
        let span = tracing::info_span!("game_log", instance_id);
        let _enter = span.enter();
        match entry.level.as_str() {
            "error" => tracing::error!(target: %entry.target, "{}", full_msg),
            "warn"  => tracing::warn!(target: %entry.target, "{}", full_msg),
            "info"  => tracing::info!(target: %entry.target, "{}", full_msg),
            "debug" => tracing::debug!(target: %entry.target, "{}", full_msg),
            "trace" => tracing::trace!(target: %entry.target, "{}", full_msg),
            _       => tracing::debug!(target: %entry.target, "{}", full_msg),
        }
    } else {
        // Plain-text fallback (e.g. raw `tracing::info!` output).
        tracing::debug!(target: "lobby::game_stderr", instance_id, "{}", line);
    }
}
```

Three properties fall out of this:

1. **Structured logs** carry their original level into the lobby journal.
2. **Plain-text fallback** keeps the existing `lobby::game_stderr` DEBUG
   target so legacy `tracing::info!` output isn't lost.
3. **`instance_id`** is attached as a tracing span field on every line,
   so journal queries can filter by game instance.

### `RUST_LOG`

The lobby's default `RUST_LOG` (from `lobby.env`) is:

```
RUST_LOG=info,lobby::http=debug,lobby::game_stderr=debug
```

This means:

- All INFO+ events are visible (including `game_log!(info, ...)` lines).
- `lobby::http` is DEBUG (request logging).
- `lobby::game_stderr` is DEBUG (plain-text fallback from game
  subprocesses).

For deeper debugging:

```
RUST_LOG=info,lobby::http=debug,lobby::game_stderr=debug,take_your_position=debug
```

This also pulls through `take_your_position` DEBUG-level events
(regardless of whether they came through `game_log!` or `tracing::info!`).

## When to use what

| Tool                            | When                                                                  |
| ------------------------------- | --------------------------------------------------------------------- |
| `game_log!(info, ...)`          | Game-domain events you want to debug later: state transitions, commit/apply, errors. Fields are queryable. |
| `game_log!(debug, ...)`         | Same, but verbose. Off by default; enable with `RUST_LOG=*,game_sdk::*=debug` or `take_your_position=debug`. |
| `tracing::info!("...")`          | Internal SDK/infrastructure messages that don't need structured fields: "ws listening", "shutdown received". |
| `tracing::debug!(...)`           | Same, but verbose. Will only show if `RUST_LOG=DEBUG` is enabled. |

## Worked example

Game binary (TYP) runs `apply_posterior` and emits:

```rust
game_log!(info, "apply_posterior accepted",
    uid = 42, seat = 0, rank_count = 5);
```

Stderr line (verbatim):

```
{"ts":"2026-08-17T06:18:01.123456Z","level":"info","target":"take_your_position::rules","message":"apply_posterior accepted","fields":{"uid":42,"seat":0,"rank_count":5}}
```

The lobby reads it, parses, builds `apply_posterior accepted uid=42 seat=0 rank_count=5`
and re-emits via `tracing::info!` with target `take_your_position::rules`
inside a span `instance_id=<id> name=game_log`.

In the journal (JSON log format):

```json
{"timestamp":"2026-08-17T06:18:01.123457Z","level":"INFO","fields":{"message":"apply_posterior accepted uid=42 seat=0 rank_count=5","target":"take_your_position::rules"},"target":"lobby::instance::manager","span":{"instance_id":21,"name":"game_log"}}
```

Querying:

```bash
journalctl -u lobby | grep -F '"target":"take_your_position::rules","message":"apply_posterior accepted'
```

Or in the rolling file:

```bash
jq 'select(.span.name=="game_log" and .fields.target=="take_your_position::rules")' /var/log/lobby/lobby.YYYY-MM-DD.log
```

## Landing checklist

Adding a new game or instrumenting an existing one:

1. **Make sure** `game-sdk` is in the game's `Cargo.toml` (`chrono` is
   re-exported via the SDK, no extra deps needed).
2. **Sprinkle** `game_log!` calls at every meaningful state transition
   (don't go overboard — debug logs add latency to hot paths).
3. **Don't** write to stdout. It's reserved for `GameEvent` JSON.
4. **Update** `docs/games_architecture.md` if the game adds a new
   long-running task that needs `game_log!` coverage.
5. **Deploy**: re-bundle with `bash build.sh`, install with the
   `lobby.env` default `RUST_LOG` (or a stricter one if desired).

## Versioning

The wire format is stable. Bumping the schema requires:

- Adding a new field (always safe — old parsers ignore unknown fields).
- Repurposing an existing field (NOT safe — requires a major version bump).
- Removing a field (NOT safe — requires a major version bump).

If a future version is needed, the lobby's `emit_game_log` and the SDK's
`LogEntry` must be updated in lock-step. The `docs/game_log_protocol.md`
file is the single source of truth.
