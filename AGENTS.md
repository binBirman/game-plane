# Agent Instructions

- This repository currently contains design documents only; there is no application source, package manifest, build/test/lint configuration, CI workflow, or executable developer command to run.
- Treat `docs/game_integration_spec.md.md` as the detailed V1 integration protocol and `docs/architecture.md` as the high-level architecture reference.
- Treat `docs/design.md` as the system design foundation: Rust/Tokio workspace, SQLite schema, crate layout, and interface protocols (HTTP, process line protocol, WebSocket).
- Treat `docs/protocol_spec.md` as the authoritative V1 state machines, interface contracts, and error-code table; state transitions and error codes are only extended there.
- Preserve the boundary that Lobby owns authentication, sessions, rooms, game-instance lifecycle, and process startup; Game owns game rules, synchronization, state, turns, reconnect handling, and game completion.
- Do not move game state or game rules into Lobby; Lobby must not depend on game-specific state.
- Game servers are independent processes started by Lobby with initialization data; Lobby/Game communication uses process I/O (`stdin`/`stdout` or pipes), and Game lifecycle events are JSON on stdout.
- Client-to-Lobby communication is HTTP; client-to-Game communication is WebSocket. Player authentication uses a Lobby-issued session token; Game must not store passwords or depend on JWT.
- Preserve the documented lifecycle and health behavior: `ready`, `running`, `finished`, `shutdown`, and `heartbeat`; an instance without a heartbeat for more than 15 seconds is considered abnormal.
- Preserve reconnect semantics: a Player is separate from its Connection; disconnects retain player state, and reconnects authenticate with the session then receive a complete snapshot.
- Room and GameInstance are separate concepts: one Room may have multiple game-instance lifecycles, and cleanup must release the instance/port and update the Room state.
- The documented V1 scope includes registration, login/session auth, room management, game startup, dynamic ports, WebSockets, lifecycle management, heartbeat checks, and reconnect; distributed deployment, gateways, plugins, Docker scheduling, matchmaking, friends, rankings, recordings, and multi-server synchronization are explicitly out of scope.
- There are no repository-local coding or formatting conventions beyond the documents above; when implementation is added, establish commands and conventions from its actual manifests/configuration rather than guessing.
