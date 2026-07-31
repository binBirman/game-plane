# syntax=docker/dockerfile:1.7
FROM rust:1.89-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /build
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl -p lobby

FROM alpine:3.20
RUN apk add --no-cache ca-certificates tini
COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/lobby /usr/local/bin/lobby
USER 65532:65532
ENV LOBBY_BIND=0.0.0.0:8192 \
    LOBBY_DATABASE_URL=sqlite:///var/lib/lobby/lobby.db?mode=rwc \
    LOBBY_LOG_FORMAT=json \
    RUST_LOG=info,lobby::http=debug
VOLUME ["/var/lib/lobby"]
EXPOSE 8192
ENTRYPOINT ["/sbin/tini", "--", "/usr/local/bin/lobby"]