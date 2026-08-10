#!/usr/bin/env bash
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "ERROR: must run as root (sudo ./install.sh)" >&2
    exit 1
fi

BIN_SRC="$(cd "$(dirname "$0")" && pwd)/lobby"
[[ -x "$BIN_SRC" ]] || { echo "ERROR: lobby binary not found in $(dirname "$BIN_SRC")" >&2; exit 1; }

# user / group
if ! id -u lobby >/dev/null 2>&1; then
    echo "==> creating system user 'lobby'"
    useradd --system --no-create-home --shell /usr/sbin/nologin lobby
fi

# directories
echo "==> creating runtime directories"
install -d -o lobby -g lobby -m 755 /var/lib/lobby
install -d -o lobby -g lobby -m 755 /var/log/lobby
install -d -m 755 /etc/lobby

# binary
echo "==> installing binaries to /usr/local/bin/"
install -m 755 "$BIN_SRC" /usr/local/bin/lobby

GAME_SRC="$(cd "$(dirname "$BIN_SRC")" && pwd)/tictactoe"
if [[ -x "$GAME_SRC" ]]; then
    install -m 755 "$GAME_SRC" /usr/local/bin/tictactoe
    echo "==> installed /usr/local/bin/tictactoe"
else
    echo "WARN: tictactoe binary not found alongside lobby; install.sh will skip it"
fi

# env file (only if missing)
if [[ ! -f /etc/lobby/lobby.env ]]; then
    echo "==> creating /etc/lobby/lobby.env from example"
    install -m 640 -o root -g lobby lobby.env.example /etc/lobby/lobby.env
    echo "    >>> review with: sudo systemctl edit lobby  or  sudo nano /etc/lobby/lobby.env"
else
    echo "==> /etc/lobby/lobby.env already exists, leaving untouched"
fi

# games.toml registry (only if missing)
if [[ -f games.toml ]]; then
    if [[ ! -f /etc/lobby/games.toml ]]; then
        echo "==> installing /etc/lobby/games.toml"
        install -m 644 -o root -g lobby games.toml /etc/lobby/games.toml
    else
        echo "==> /etc/lobby/games.toml already exists, leaving untouched"
    fi
fi

# nginx reverse proxy example (optional, only if nginx is installed)
if [[ -f nginx.conf.example ]] && command -v nginx >/dev/null 2>&1; then
    if [[ ! -f /etc/nginx/sites-available/lobby.conf ]]; then
        echo "==> installing /etc/nginx/sites-available/lobby.conf (review + enable manually)"
        install -m 644 nginx.conf.example /etc/nginx/sites-available/lobby.conf
        echo "    >>> edit server_name, then: sudo ln -s ../sites-available/lobby.conf /etc/nginx/sites-enabled/lobby && sudo systemctl reload nginx"
    fi
fi

# systemd
echo "==> installing systemd unit"
install -m 644 lobby.service /etc/systemd/system/lobby.service
systemctl daemon-reload
systemctl enable lobby.service

cat <<EOF

Installed. Next steps:

    sudo systemctl edit lobby      # optional: drop-in overrides
    sudo systemctl start lobby
    sudo systemctl status lobby

Logs:
    journalctl -u lobby -f         # live
    tail -f /var/log/lobby/lobby.*.log   # rolling files

Smoke test:
    curl -s -X POST http://127.0.0.1:8080/api/register \\
        -H 'Content-Type: application/json' \\
        -d '{"username":"alice","password":"secret123","nickname":"Alice"}'

EOF