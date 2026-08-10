# Lobby 运维手册（Linux / systemd）

## 1. 安装

```bash
# 解包并安装
tar xzf lobby-0.1.0.tar.gz
cd lobby-0.1.0
sudo ./install.sh

# 安装做了什么：
#   /usr/local/bin/lobby          static musl 二进制
#   /usr/local/bin/tictactoe      static musl 二进制
#   /etc/lobby/lobby.env          环境变量（从 lobby.env.example 拷贝；之后改这里）
#   /etc/lobby/games.toml         游戏注册表（多游戏）
#   /etc/systemd/system/lobby.service
#   /var/lib/lobby                SQLite DB 目录
#   /var/log/lobby                滚动日志目录
#   user/group `lobby`（system）
#   如果 nginx 已装，会同时丢一份 /etc/nginx/sites-available/lobby.conf（不自动启用）
```

## 2. 配置

`/etc/lobby/lobby.env`：

| 变量 | 默认值 | 含义 |
|---|---|---|
| `LOBBY_BIND` | `0.0.0.0:8192` | 监听地址（**裸 HTTP/WS**，必须放 nginx 后） |
| `LOBBY_DATABASE_URL` | `sqlite:///var/lib/lobby/lobby.db?mode=rwc` | SQLite URL |
| `LOBBY_SESSION_TTL_DAYS` | `7` | 会话有效期 |
| `LOBBY_PUBLIC_HOST` | `127.0.0.1` | 客户端用于拼 `ws_url` 的主机名（**对外域名**） |
| `LOBBY_PUBLIC_PORT` | `LOBBY_BIND` 的端口 | 客户端用于拼 `ws_url` 的端口（**对外端口**） |
| `LOBBY_GAMES_TOML` | `<unset>` | 注册表路径；未设则用单 binary 兜底 |
| `LOBBY_GAME_BIN` | `tictactoe` | 默认 game binary |
| `LOBBY_LOG_FORMAT` | `json` | `text` 或 `json` |
| `LOBBY_LOG_FILE_DIR` | `/var/log/lobby` | 滚动日志目录；设为空 → 仅 stdout |
| `LOBBY_LOG_KEEP_DAYS` | `14` | 旧日志自动删除 |
| `LOBBY_POW_DIFFICULTY` | `16` | 注册/登录 PoW 难度（bit） |
| `LOBBY_RL_REGISTER_PER_MIN` | `10` | 注册限流（每 IP，60s 滑动窗口） |
| `LOBBY_RL_LOGIN_PER_MIN` | `20` | 登录限流 |
| `LOBBY_RL_CAPTCHA_PER_MIN` | `60` | Captcha 挑战限流 |
| `RUST_LOG` | `info,lobby::http=debug` | tracing-subscriber EnvFilter |

应用：`sudo systemctl restart lobby`。

## 3. 反向代理 + TLS

**必须**：Lobby 自身只接受 HTTP/WS；裸跑会泄露 session token。
`packaging/nginx.conf` 是参考配置：

```bash
sudo ln -s /etc/nginx/sites-available/lobby.conf /etc/nginx/sites-enabled/lobby
sudo nginx -t && sudo systemctl reload nginx

# Let's Encrypt
sudo apt install certbot python3-certbot-nginx
sudo certbot --nginx -d lobby.example.com
```

`certbot` 会自动填 `ssl_certificate` / `ssl_certificate_key`，续期走 systemd timer。

## 4. systemd 单元

`/etc/systemd/system/lobby.service`：

```ini
[Service]
Type=simple
User=lobby
Group=lobby
EnvironmentFile=/etc/lobby/lobby.env
ExecStart=/usr/local/bin/lobby
WorkingDirectory=/var/lib/lobby
Restart=on-failure
RestartSec=5
LimitNOFILE=65536
```

常用：

```bash
sudo systemctl status lobby
sudo journalctl -u lobby -f             # 实时 stdout/stderr
tail -f /var/log/lobby/lobby.*.log      # 滚动日志（JSON 或 text）
sudo systemctl restart lobby
sudo systemctl stop lobby               # SIGTERM，触发 5s 优雅关 game 实例
```

## 5. 监控与故障排查

| 现象 | 查 |
|---|---|
| HTTP 500 INTERNAL_ERROR | `journalctl -u lobby -n 100`；找 `request_id` |
| WS 客户端连不上 | 先确认 `/ws/<instance_id>` 路由 → nginx → Lobby 9192 单端口；用 `curl --noproxy '*' http://127.0.0.1:8192/api/games` 验 Lobby 存活 |
| Game 一直 Starting | Lobby 日志里查 `instance_id=...`；可能 ready 超时（10s）或 spawn 失败 |
| Heartbeat 异常回收 | 日志里 `heartbeat timeout`；Game 进程卡死或 stdout 缓冲 |
| 注册/登录 429 | `LOBBY_RL_*_PER_MIN` 调高；或封禁 IP |

`x-request-id` 关联：客户端发请求带 `X-Request-Id: xxx`，Lobby 写到日志 span，journalctl 和滚动日志都能 grep 出整条链路。

## 6. 备份与升级

**备份**：单文件 SQLite，定期拷走即可：

```bash
sqlite3 /var/lib/lobby/lobby.db ".backup '/var/backups/lobby-$(date +%F).db'"
```

**升级**：替换 `/usr/local/bin/lobby` 与 `/usr/local/bin/tictactoe`，重启。
Schema 变更通过 `db/migrations.rs` 的幂等 `ALTER TABLE` 自动应用；新会话 GC 后台任务每小时跑一次。

## 7. 添加新游戏

1. `crates/games/<name>/`：实现 `game-sdk::GameLogic`，main 里 `game_sdk::init_tracing()` + 解析 stdin init + `game_sdk::run::<MyGame>(init)`。
2. 在 `/etc/lobby/games.toml` 加 `[[games]]` 段：

```toml
[[games]]
type = "mygame"
name = "我的游戏"
description = "..."
binary = "/usr/local/bin/mygame"
min_players = 2
max_players = 4
enabled = true
variants = ["classic", "blitz"]
```

3. `sudo systemctl restart lobby`。客户端 `GET /api/games` 立即可见，`POST /api/rooms {"game_type":"mygame","variant":"blitz"}` 即可建房。

Lobby / SDK **无需改动**。

## 8. 限速与防爆破

- Lobby 内置每 IP 滑动 60s 窗口限流（register/login/captcha），由 `LOBBY_RL_*` 控制。
- nginx 层 `limit_req_zone` 在 `packaging/nginx.conf` 也加了二次防护。
- 若遇到 CC，建议在 nginx 前再加 Cloudflare / 阿里云 WAF。

## 9. 已知边界

- **进程内限流**：重启清零；不集群同步。多实例部署需前置共享存储（Redis token bucket）。
- **SQLite 单机**：高并发写场景需迁 PostgreSQL（超出 V1 范围）。
- **无 metrics endpoint**：Prometheus 抓取未做；先看 journal + 日志。
- **前端的 `app.js` 仍硬编码 `tictactoe` option**：多游戏前端 UI 待做（仅前端工作，不影响后端）。