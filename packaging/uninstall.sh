#!/usr/bin/env bash
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "ERROR: must run as root" >&2
    exit 1
fi

echo "==> stopping and disabling service"
systemctl stop lobby.service 2>/dev/null || true
systemctl disable lobby.service 2>/dev/null || true

echo "==> removing files"
rm -f /etc/systemd/system/lobby.service
rm -f /usr/local/bin/lobby
rm -f /usr/local/bin/tictactoe
rm -rf /etc/lobby

systemctl daemon-reload

cat <<EOF
Uninstalled.

Data and logs were preserved (remove manually if desired):
    rm -rf /var/lib/lobby    # sqlite DB
    rm -rf /var/log/lobby    # rolling logs
    userdel lobby            # remove system user

EOF