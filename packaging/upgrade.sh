#!/usr/bin/env bash
# One-command in-place upgrade for an existing lobby deployment.
#
# Run this from the extracted tarball directory, exactly like install.sh:
#   cd /opt/lobby-0.3.0
#   sudo ./upgrade.sh
#
# It does everything install.sh does PLUS the things you'd otherwise have to
# do by hand every upgrade:
#   1. Stop the service before swapping binaries (avoids "text file busy").
#   2. Kill orphaned game subprocesses (take_your_position / tictactoe) so a
#      stale binary can't linger after the lobby restarts.
#   3. Force-overwrite /etc/lobby/games.toml with the packaged version.
#   4. Restart the service and verify it came back up + serves the right games.
#
# Data is preserved: /var/lib/lobby/lobby.db and /etc/lobby/lobby.env are kept.
#
# Flags:
#   --no-restart   Don't restart lobby.service at the end.
#   --skip-verify  Skip the post-install checks.
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "ERROR: must run as root (sudo ./upgrade.sh)" >&2
    exit 1
fi

DO_RESTART=1
DO_VERIFY=1
for arg in "$@"; do
    case "$arg" in
        --no-restart)  DO_RESTART=0 ;;
        --skip-verify) DO_VERIFY=0 ;;
        *) echo "ERROR: unknown flag: $arg" >&2; exit 1 ;;
    esac
done

DIR="$(cd "$(dirname "$0")" && pwd)"
BIN_SRC="$DIR/lobby"
[[ -x "$BIN_SRC" ]] || { echo "ERROR: lobby binary not found in $DIR" >&2; exit 1; }

echo "==> stopping lobby.service (if running)"
if systemctl is-active --quiet lobby.service 2>/dev/null; then
    systemctl stop lobby.service
    echo "    stopped"
fi

echo "==> killing orphaned game subprocesses"
for p in /usr/local/bin/take_your_position /usr/local/bin/tictactoe; do
    if pgrep -f "^$p" >/dev/null 2>&1; then
        pkill -9 -f "^$p" 2>/dev/null || true
        echo "    killed orphaned: $p"
    fi
done

echo "==> swapping binaries"
install -m 755 "$BIN_SRC" /usr/local/bin/lobby
for g in tictactoe take_your_position; do
    if [[ -x "$DIR/$g" ]]; then
        install -m 755 "$DIR/$g" "/usr/local/bin/$g"
        echo "    /usr/local/bin/$g"
    fi
done

# systemd unit + games.toml (force overwrite so new games appear).
install -m 644 "$DIR/lobby.service" /etc/systemd/system/lobby.service
systemctl daemon-reload
systemctl enable lobby.service >/dev/null 2>&1 || true
install -m 644 -o root -g lobby "$DIR/games.toml" /etc/lobby/games.toml
echo "    /etc/lobby/games.toml (overwritten)"

echo "==> binaries installed:"
md5sum /usr/local/bin/lobby /usr/local/bin/tictactoe /usr/local/bin/take_your_position

# Env file: keep existing /etc/lobby/lobby.env if present (preserves config).
if [[ ! -f /etc/lobby/lobby.env ]]; then
    install -m 640 -o root -g lobby "$DIR/lobby.env.example" /etc/lobby/lobby.env
    echo "==> created /etc/lobby/lobby.env from example"
else
    echo "==> kept existing /etc/lobby/lobby.env (config preserved)"
fi

# nginx example (only if nginx installed and not yet configured)
if [[ -f "$DIR/nginx.conf.example" ]] && command -v nginx >/dev/null 2>&1 \
   && [[ ! -f /etc/nginx/sites-available/lobby.conf ]]; then
    install -m 644 "$DIR/nginx.conf.example" /etc/nginx/sites-available/lobby.conf
    echo "==> installed /etc/nginx/sites-available/lobby.conf (review + enable manually)"
fi

if [[ $DO_RESTART -eq 1 ]]; then
    echo "==> starting lobby.service"
    systemctl start lobby.service
    for _ in $(seq 1 20); do
        if systemctl is-active --quiet lobby.service; then break; fi
        sleep 0.2
    done
else
    echo "==> --no-restart: start manually with: systemctl start lobby"
fi

if [[ $DO_VERIFY -eq 1 ]]; then
    echo ""
    echo "==> verification"
    systemctl is-active lobby.service
    echo "--- games.toml ---"
    grep -E '^type|^name' /etc/lobby/games.toml
    echo "--- /api/games ---"
    curl -s http://127.0.0.1:8192/api/games | python3 -m json.tool 2>/dev/null || \
        curl -s http://127.0.0.1:8192/api/games
    echo ""
    echo "    If the list is missing take_your_position, check /etc/lobby/lobby.env"
    echo "    has LOBBY_GAMES_TOML=/etc/lobby/games.toml (uncommented)."
fi

cat <<EOF

==> Upgrade done.

    Old data preserved:
        /var/lib/lobby/lobby.db   (accounts / rooms)
        /etc/lobby/lobby.env      (config)

    Binaries:
        /usr/local/bin/lobby                $(md5sum /usr/local/bin/lobby | cut -d' ' -f1)
        /usr/local/bin/take_your_position   $(md5sum /usr/local/bin/take_your_position | cut -d' ' -f1)
        /usr/local/bin/tictactoe            $(md5sum /usr/local/bin/tictactoe | cut -d' ' -f1)

    Live log: journalctl -u lobby -f
EOF
