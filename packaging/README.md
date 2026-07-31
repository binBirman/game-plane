# lobby-server install package

Linux x86_64 install of the Lobby server (static musl binary).

## Files

| File | Purpose |
|---|---|
| `lobby` | static binary (musl, no runtime deps) |
| `lobby.service` | systemd unit |
| `lobby.env.example` | config template |
| `install.sh` | privileged installer |
| `uninstall.sh` | privileged remover |

## Install

```bash
sudo ./install.sh
sudo systemctl edit lobby   # optional overrides
sudo systemctl start lobby
sudo systemctl status lobby
```

The installer:

- creates system user `lobby`
- installs binary to `/usr/local/bin/lobby`
- creates `/var/lib/lobby` (sqlite data) and `/var/log/lobby` (logs)
- writes `/etc/lobby/lobby.env` from the example (edit before starting)
- enables the systemd service (does **not** start it)

## Configure

Edit `/etc/lobby/lobby.env`:

| Var | Default | Notes |
|---|---|---|
| `LOBBY_BIND` | `0.0.0.0:8192` | listen address |
| `LOBBY_DATABASE_URL` | `sqlite:///var/lib/lobby/lobby.db?mode=rwc` | sqlite path |
| `LOBBY_SESSION_TTL_DAYS` | `7` | session expiry |
| `LOBBY_LOG_FORMAT` | `json` | `text` or `json` |
| `LOBBY_LOG_FILE_DIR` | `/var/log/lobby` | unset to log to stdout only |
| `LOBBY_LOG_KEEP_DAYS` | `14` | old files pruned on startup |
| `RUST_LOG` | `info,lobby::http=debug` | env-filter syntax |

Reload after editing: `sudo systemctl restart lobby`.

## Logs

- **journald (always on)**: `journalctl -u lobby -f`
- **rolling files** (when `LOBBY_LOG_FILE_DIR` set): `/var/log/lobby/lobby.YYYY-MM-DD.log`
- **HTTP correlation**: every response carries `x-request-id`; the same id appears in both stdout and file logs as the span field `request_id`.

## Smoke test

```bash
curl --noproxy '*' -X POST http://127.0.0.1:8192/api/register \
  -H 'Content-Type: application/json' \
  -d '{"username":"alice","password":"secret123","nickname":"Alice"}'

curl --noproxy '*' -X POST http://127.0.0.1:8192/api/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"alice","password":"secret123"}'
```

Or use the bundled `tools/test.sh` (10+ test cases with pass/fail summary).

Expected: `{"uid":1,...}` and `{"uid":1,"token":"<64hex>"}`.

## Uninstall

```bash
sudo ./uninstall.sh
```

Preserves `/var/lib/lobby` and `/var/log/lobby`; remove manually if desired.

## Firewall

V1 uses **reverse proxy**: the client connects to the Lobby's WS endpoint on the bind port only. Game instances bind `127.0.0.1` and are not reachable from outside — no extra ports to open.