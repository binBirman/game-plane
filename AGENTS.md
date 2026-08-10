# Agent Instructions

- This repository is a working Rust workspace implementing the V1 Lobby server design in `docs/architecture.md`, `docs/design.md`, `docs/protocol_spec.md`, and `docs/games_architecture.md`.
- Build & test locally:
  - `cargo check --workspace` / `cargo test --workspace`
  - `cargo build --release -p lobby -p tictactoe`
  - `bash build.sh` produces `dist/lobby-<ver>.tar.gz` (musl static)
- Smoke test against a running server:
  - `LOBBY_GAME_BIN=$(pwd)/target/debug/tictactoe cargo run -p lobby` (defaults to `127.0.0.1:8192`)
  - `PORT=8192 bash tools/test.sh` (25 cases including WS roundtrip)
- Treat `docs/protocol_spec.md` as the authoritative state machine + error-code table; new states / codes go there first.
- Treat `docs/games_architecture.md` as the authoritative multi-game plan; the implementation is incremental (tictactoe today, more games added by dropping in a crate under `crates/games/<name>` and registering it in `games.toml`).
- Preserve the boundary: Lobby owns auth/sessions/rooms/process lifecycle; Game owns game rules/state/turns/reconnect; Game must not store passwords or depend on JWT.
- All HTTP responses share `{"error":{"code","message"}}` for failures; WS uses `{"type":"error",...}` and `{"type":"game_error",...}`.
- WS reverse proxy (`/ws/:instance_id`) is transport-layer only — Lobby never parses game envelopes.
- Heartbeat timeout: 15s. Watchdog scans every 5s. Game SDK sends heartbeat every 5s.
- Room / GameInstance are separate: one Room may have multiple instance lifecycles; cleanup releases the instance + port and updates `rooms.status`.
- V1 scope: register, login, captcha PoW, rooms CRUD, dynamic ports, WS reverse proxy, lifecycle, heartbeat, reconnect. Out of scope: distributed deployment, gateways, plugins, Docker scheduling, matchmaking, friends, rankings, recordings, multi-server sync.
- To add a new game:
  1. `crates/games/<name>/` with a `[[bin]]` that depends on `game-sdk` and implements `GameLogic`.
  2. Add a `[[games]]` entry in `packaging/games.toml` (and `/etc/lobby/games.toml` on the server).
  3. No changes to lobby or game-sdk required.