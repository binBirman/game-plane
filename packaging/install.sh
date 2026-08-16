#!/usr/bin/env bash
# Lobby installer. Run as root from inside the extracted tarball directory:
#   tar xzf lobby-<ver>.tar.gz
#   cd lobby-<ver>
#   sudo ./install.sh
#
# Flags:
#   --force-games-toml    Overwrite /etc/lobby/games.toml with the version
#                         bundled in this tarball (default: leave existing).
#   --no-restart          Don't restart lobby.service at the end (default:
#                         restart if the unit is loaded).
#   --skip-verify         Skip the post-install file checks.
#
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "ERROR: must run as root (sudo ./install.sh)" >&2
    exit 1
fi

FORCE_GAMES_TOML=0
DO_RESTART=1
DO_VERIFY=1
for arg in "$@"; do
    case "$arg" in
        --force-games-toml) FORCE_GAMES_TOML=1 ;;
        --no-restart)       DO_RESTART=0 ;;
        --skip-verify)      DO_VERIFY=0 ;;
        -h|--help)
            sed -n '2,12p' "$0"; exit 0 ;;
        *)
            echo "ERROR: unknown flag: $arg" >&2; exit 1 ;;
    esac
done

BIN_SRC="$(cd "$(dirname "$0")" && pwd)/lobby"
[[ -x "$BIN_SRC" ]] || { echo "ERROR: lobby binary not found in $(dirname "$BIN_SRC")" >&2; exit 1; }

# On upgrade (service already installed), stop it first so we can swap the
# binary cleanly and don't leave orphaned game subprocesses behind.
if [[ $DO_RESTART -eq 1 ]] && systemctl list-unit-files lobby.service >/dev/null 2>&1 \
   && systemctl is-active --quiet lobby.service 2>/dev/null; then
    echo "==> stopping lobby.service for clean binary swap"
    systemctl stop lobby.service
    sleep 1
fi
# Kill any orphaned game subprocesses (stale binaries from a previous lobby).
for p in /usr/local/bin/take_your_position /usr/local/bin/tictactoe; do
    if pgrep -f "^$p" >/dev/null 2>&1; then
        pkill -9 -f "^$p" 2>/dev/null || true
        echo "    killed orphaned: $p"
    fi
done

echo "==> creating system user 'lobby' (if missing)"
if ! id -u lobby >/dev/null 2>&1; then
    useradd --system --no-create-home --shell /usr/sbin/nologin lobby
fi

echo "==> creating runtime directories"
install -d -o lobby -g lobby -m 755 /var/lib/lobby
install -d -o lobby -g lobby -m 755 /var/log/lobby
install -d -m 755 /etc/lobby

echo "==> installing binaries to /usr/local/bin/"
install -m 755 "$BIN_SRC" /usr/local/bin/lobby

GAME_SRC="$(cd "$(dirname "$BIN_SRC")" && pwd)/tictactoe"
if [[ -x "$GAME_SRC" ]]; then
    install -m 755 "$GAME_SRC" /usr/local/bin/tictactoe
    echo "    installed /usr/local/bin/tictactoe"
else
    echo "WARN: tictactoe binary not found alongside lobby; skipping" >&2
fi

TYP_SRC="$(cd "$(dirname "$BIN_SRC")" && pwd)/take_your_position"
if [[ -x "$TYP_SRC" ]]; then
    install -m 755 "$TYP_SRC" /usr/local/bin/take_your_position
    echo "    installed /usr/local/bin/take_your_position"
else
    echo "WARN: take_your_position binary not found alongside lobby; skipping" >&2
fi

# env file (only if missing)
if [[ ! -f /etc/lobby/lobby.env ]]; then
    echo "==> creating /etc/lobby/lobby.env from example"
    install -m 640 -o root -g lobby lobby.env.example /etc/lobby/lobby.env
    echo "    >>> review with: sudo systemctl edit lobby  or  sudo nano /etc/lobby/lobby.env"
else
    echo "==> /etc/lobby/lobby.env already exists, leaving untouched"
fi

# games.toml registry
if [[ -f games.toml ]]; then
    if [[ ! -f /etc/lobby/games.toml ]]; then
        echo "==> installing /etc/lobby/games.toml"
        install -m 644 -o root -g lobby games.toml /etc/lobby/games.toml
    elif [[ $FORCE_GAMES_TOML -eq 1 ]]; then
        echo "==> --force-games-toml: overwriting /etc/lobby/games.toml"
        install -m 644 -o root -g lobby games.toml /etc/lobby/games.toml
    else
        echo "==> /etc/lobby/games.toml already exists (use --force-games-toml to overwrite)"
    fi
fi

# nginx reverse proxy example
if [[ -f nginx.conf.example ]] && command -v nginx >/dev/null 2>&1; then
    if [[ ! -f /etc/nginx/sites-available/lobby.conf ]]; then
        echo "==> installing /etc/nginx/sites-available/lobby.conf (review + enable manually)"
        install -m 644 nginx.conf.example /etc/nginx/sites-available/lobby.conf
        echo "    >>> edit server_name, then:"
        echo "    >>>   sudo ln -s ../sites-available/lobby.conf /etc/nginx/sites-enabled/lobby"
        echo "    >>>   sudo systemctl reload nginx"
    fi
fi

# systemd unit
echo "==> installing systemd unit"
install -m 644 lobby.service /etc/systemd/system/lobby.service
systemctl daemon-reload
systemctl enable lobby.service

# Restart (or start) the service
if systemctl list-unit-files lobby.service >/dev/null 2>&1 && \
   systemctl is-active --quiet lobby.service; then
    if [[ $DO_RESTART -eq 1 ]]; then
        echo "==> restarting lobby.service"
        systemctl restart lobby.service
        # Wait briefly for it to bind
        for _ in $(seq 1 20); do
            if systemctl is-active --quiet lobby.service; then break; fi
            sleep 0.2
        done
    else
        echo "==> --no-restart: lobby.service left running with old binary"
        echo "    >>> restart manually: sudo systemctl restart lobby"
    fi
elif [[ $DO_RESTART -eq 1 ]]; then
    echo "==> starting lobby.service"
    systemctl start lobby.service
fi

# Post-install verification (file-level only, no port probing)
if [[ $DO_VERIFY -eq 1 ]]; then
    echo ""
    echo "==> post-install verification"
    ok=1
    for f in /usr/local/bin/lobby /usr/local/bin/tictactoe /usr/local/bin/take_your_position; do
        if [[ -x "$f" ]]; then
            echo "    [ok] $f"
        else
            echo "    [FAIL] $f missing or not executable"; ok=0
        fi
    done
    for f in /etc/lobby/games.toml /etc/lobby/lobby.env /etc/systemd/system/lobby.service; do
        if [[ -f "$f" ]]; then
            echo "    [ok] $f"
        else
            echo "    [FAIL] $f missing"; ok=0
        fi
    done
    if [[ $ok -eq 0 ]]; then
        echo ""
        echo "Some files are missing — install did not complete cleanly." >&2
        exit 1
    fi
fi

cat <<EOF

==> Done.

    Binary:    /usr/local/bin/lobby  ($(stat -c '%s' /usr/local/bin/lobby) bytes)
    Game 1:    /usr/local/bin/tictactoe
    Game 2:    /usr/local/bin/take_your_position
    Config:    /etc/lobby/games.toml  ($(grep -c '\[\[games\]\]' /etc/lobby/games.toml) games registered)
    Env:       /etc/lobby/lobby.env
    Unit:      /etc/systemd/system/lobby.service

Verify it's actually serving the new binary:

    systemctl status lobby              # service state + latest log lines
    curl -s http://127.0.0.1:8192/api/games    # should list take_your_position
    PORT=8192 bash tools/test.sh        # 25 smoke tests

For FUTURE upgrades (no manual delete needed, keeps DB + env):

    cd /opt/lobby-<newver>
    sudo ./upgrade.sh                   # stop → swap binaries → force games.toml → restart

Live log:

    journalctl -u lobby -f

EOF
