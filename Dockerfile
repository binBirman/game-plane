# syntax=docker/dockerfile:1.7
FROM rust:1.89-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /build
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl -p lobby && \
    cargo build --release --target x86_64-unknown-linux-musl -p tictactoe

FROM alpine:3.20
RUN apk add --no-cache ca-certificates tini curl
COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/lobby     /usr/local/bin/lobby
COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/tictactoe /usr/local/bin/tictactoe
COPY packaging/games.toml /etc/lobby/games.toml
USER 65532:65532
ENV LOBBY_BIND=0.0.0.0:8192 \
    LOBBY_DATABASE_URL=sqlite:///var/lib/lobby/lobby.db?mode=rwc \
    LOBBY_LOG_FORMAT=json \
    LOBBY_GAMES_TOML=/etc/lobby/games.toml \
    LOBBY_GAME_BIN=/usr/local/bin/tictactoe \
    RUST_LOG=info,lobby::http=debug
VOLUME ["/var/lib/lobby", "/var/log/lobby"]
EXPOSE 8192
HEALTHCHECK --interval=15s --timeout=3s --retries=3 --start-period=10s \
    CMD curl -fsS http://127.0.0.1:8192/ >/dev/null || exit 1
ENTRYPOINT ["/sbin/tini", "--", "/usr/local/bin/lobby"]