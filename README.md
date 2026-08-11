# game-plane / Lobby Server

一个轻量级多人游戏大厅服务器。Lobby 管理认证、房间与游戏进程生命周期；Game 进程（独立的 Rust 二进制）实现游戏规则、状态同步、断线重连。Lobby 与 Game 之间通过 stdin/stdout 行协议通信；客户端只连 Lobby 的单一公网端口，WS 帧由 Lobby 透明反代到 Game。

设计文档：

- [`docs/architecture.md`](docs/architecture.md) — 总体架构
- [`docs/design.md`](docs/design.md) — 工作区与协议概览
- [`docs/protocol_spec.md`](docs/protocol_spec.md) — 状态机 + 接口 + 错误码（权威）
- [`docs/games_architecture.md`](docs/games_architecture.md) — V1.1 多游戏骨架

---

## 目录

1. [技术栈](#技术栈)
2. [仓库结构](#仓库结构)
3. [快速开始](#快速开始)
4. [运行需求](#运行需求)
5. [配置](#配置)
6. [构建与发布](#构建与发布)
7. [部署](#部署)
8. [测试](#测试)
9. [添加新游戏](#添加新游戏)
10. [HTTP 接口摘要](#http-接口摘要)
11. [WebSocket / 进程协议摘要](#websocket--进程协议摘要)
12. [错误码](#错误码)
13. [运维与故障排查](#运维与故障排查)

---

## 技术栈

### 语言与运行时

| 项 | 选择 | 版本 |
|---|---|---|
| 语言 | Rust | edition 2021，MSRV 1.75 |
| 异步运行时 | Tokio | 1.39（full feature） |
| HTTP/WS 框架 | axum | 0.7（`macros`, `ws`） |
| 数据库 | SQLite（WAL 模式） | sqlx 0.8 |
| WS 客户端（lobby → game） | tokio-tungstenite | 0.24 |
| 密码哈希 | argon2 | 0.5 |
| 序列化 | serde / serde_json | 1.x |
| 日志 | tracing + tracing-subscriber + tracing-appender | 0.1 / 0.3 / 0.2 |
| 配置 | 环境变量 + TOML（`games.toml`） | toml 0.8 |

### 静态资源

- HTML / CSS / JS 通过 [`rust-embed`](https://docs.rs/rust-embed) 编译进二进制，无需运行时拷贝
- WS 客户端握手测试用纯 Python 实现（`tools/ws_client.py`，无外部依赖）

### Workspace 布局

```
game-plane/
├── Cargo.toml              workspace 根
├── crates/
│   ├── protocol/           共享消息类型（LobbyInit / PlayerInit）
│   ├── lobby/              Lobby 二进制（HTTP + WS 反代 + 实例管理）
│   ├── game-sdk/           游戏通信骨架库（GameLogic trait + run()）
│   └── games/
│       └── tictactoe/      第一款游戏：井字棋（二进制名 tictactoe）
├── docs/                   设计文档
├── packaging/              systemd unit、env 模板、games.toml、nginx 配置、install/uninstall 脚本
├── tools/
│   ├── test.sh             端到端烟测（25 用例）
│   └── ws_client.py        纯 Python WS 客户端
├── build.sh                musl 静态构建 + 打包
├── Dockerfile              Alpine + musl + tini
└── .github/workflows/ci.yml
```

---

## 快速开始

```bash
# 构建（默认 musl 静态二进制；需要 rustup target x86_64-unknown-linux-musl + musl-tools）
bash build.sh
# 产物：dist/lobby-<version>.tar.gz

# 本地直接运行（开发模式）
cargo run -p lobby
# 默认监听 127.0.0.1:8192；DB 写到 ./data/lobby.db

# 烟测
PORT=8192 bash tools/test.sh
# 期望：All 25 tests passed
```

---

## 运行需求

### 二进制端点

| 文件 | 用途 |
|---|---|
| `lobby` | Lobby 主进程（HTTP + WS） |
| `tictactoe` | 默认游戏进程，由 Lobby 按需 spawn |

构建产物为**静态 musl 二进制**，无动态依赖；可放到任何 glibc / musl Linux 上跑。

### 运行时端点

| 资源 | 必需 | 默认 |
|---|---|---|
| 监听端口（HTTP + WS） | ✅ | `0.0.0.0:8192`（TCP） |
| SQLite 文件 | ✅ | `data/lobby.db` |
| 可写的临时目录 | ✅ | 启动时 `127.0.0.1:0` 端口分配 |
| `/usr/local/bin`（生产安装） | 生产可选 | install.sh 自动放 |
| game binary 目录 | ✅ | `LOBBY_GAME_BIN` 指定路径 |
| game registry TOML | 多游戏必需 | `LOBBY_GAMES_TOML` 指定路径；未设走兜底（单 tictactoe） |

### 生产环境额外端点（推荐）

| 资源 | 用途 |
|---|---|
| nginx / caddy | TLS 终止 + WS 反代（**必需**——Lobby 只接受裸 HTTP/WS） |
| certbot | Let's Encrypt 证书自动签发与续期 |
| systemd | 进程托管 + journald 日志 + 自动重启 |
| `/var/lib/lobby` | SQLite 数据目录（system 用户 `lobby` 可写） |
| `/var/log/lobby` | 滚动日志目录 |
| `/etc/lobby/lobby.env` | 环境变量配置 |
| `/etc/lobby/games.toml` | 游戏注册表 |
| 防火墙 | **只放 Lobby 端口**；game 进程绑 127.0.0.1，无需对外 |

### 系统能力

- 不需要 root（容器/普通用户可跑），生产 install 脚本会创建系统用户
- `tokio::process::Command` spawn 子进程（已在 `target/release/lobby` 中验证）
- 文件系统：`/usr/local/bin` 写入（仅生产安装）

### 端口

- **单一公开端口**（默认 8192）：HTTP + WS 复用
- game 进程端口：动态分配，监听 `127.0.0.1`，对外不可见
- nginx 监听 80/443；后端 127.0.0.1:8192

---

## 配置

Lobby 接收两类配置：环境变量（部署层）+ TOML（数据层）。

### 环境变量（`/etc/lobby/lobby.env`）

| 变量 | 默认 | 必填 | 含义 |
|---|---|---|---|
| `LOBBY_BIND` | `0.0.0.0:8192` |  | 监听地址。**裸 HTTP/WS**，必须前置 nginx |
| `LOBBY_DATABASE_URL` | `sqlite://data/lobby.db?mode=rwc` |  | SQLite URL；`?mode=rwc` 自动建库 |
| `LOBBY_SESSION_TTL_DAYS` | `7` |  | session 有效期 |
| `LOBBY_PUBLIC_HOST` | `127.0.0.1` |  | `start` 返回的 `ws_url` 中使用的主机名（**对外域名**） |
| `LOBBY_PUBLIC_PORT` | =`LOBBY_BIND` 的端口 |  | `ws_url` 中的端口（**对外端口**） |
| `LOBBY_GAMES_TOML` | `<unset>` | 多游戏 | 注册表路径；未设走兜底（单 binary tictactoe） |
| `LOBBY_GAME_BIN` | `tictactoe` |  | 默认 game 二进制路径 |
| `LOBBY_LOG_FORMAT` | `json` |  | `text` 或 `json` |
| `LOBBY_LOG_FILE_DIR` | `/var/log/lobby` |  | 滚动日志目录；设为空 → 仅 stdout |
| `LOBBY_LOG_KEEP_DAYS` | `14` |  | 旧日志自动删除阈值 |
| `LOBBY_POW_DIFFICULTY` | `16` |  | 注册/登录 PoW 难度（bit） |
| `LOBBY_RL_REGISTER_PER_MIN` | `10` |  | 注册限流（每 IP，60s 滑动窗口） |
| `LOBBY_RL_LOGIN_PER_MIN` | `20` |  | 登录限流 |
| `LOBBY_RL_CAPTCHA_PER_MIN` | `60` |  | Captcha 挑战限流 |
| `RUST_LOG` | `info,lobby::http=debug` |  | `tracing-subscriber` EnvFilter |

### 游戏注册表（`/etc/lobby/games.toml`）

```toml
[[games]]
type = "tictactoe"
name = "井字棋"
description = "三连一线"
binary = "/usr/local/bin/tictactoe"
min_players = 2
max_players = 2
enabled = true
variants = []
```

每条记录一个游戏；`enabled=false` 跳过；`type` 唯一。

### 配置方法

#### 方法 1：编辑 `packaging/lobby.env.example` 然后 install

```bash
# 编辑
nano packaging/lobby.env.example

# 打包时会被复制成 dist/lobby-<ver>/lobby.env.example
bash build.sh

# 上线时 install.sh 会把它装到 /etc/lobby/lobby.env
sudo ./install.sh
```

#### 方法 2：systemd drop-in

```bash
sudo systemctl edit lobby
# 写入：
# [Service]
# Environment="LOBBY_POW_DIFFICULTY=20"
# Environment="LOBBY_BIND=127.0.0.1:8192"
sudo systemctl restart lobby
```

#### 方法 3：直接 env 启动（开发模式）

```bash
LOBBY_BIND=127.0.0.1:8192 \
LOBBY_DATABASE_URL=sqlite:///tmp/lobby.db?mode=rwc \
LOBBY_GAME_BIN=$(pwd)/target/debug/tictactoe \
RUST_LOG=debug \
cargo run -p lobby
```

#### 方法 4：games.toml 即时生效

```bash
# 编辑后重启即可（无热加载）
sudo nano /etc/lobby/games.toml
sudo systemctl restart lobby
```

---

## 构建与发布

### 开发

```bash
cargo check --workspace            # 类型检查
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace             # 8 单元测试
cargo build                        # 调试构建
cargo build --release              # 优化构建（非 musl）
```

### 生产 musl 静态构建

```bash
bash build.sh
# 等价于：
#   rustup target add x86_64-unknown-linux-musl
#   sudo apt install musl-tools            # Debian/Ubuntu
#   apk add musl-dev gcc                   # Alpine
#   cargo build --release --target x86_64-unknown-linux-musl
#
# 产物：dist/lobby-<ver>.tar.gz，含 lobby / tictactoe / install.sh / uninstall.sh /
#       lobby.service / lobby.env.example / games.toml / nginx.conf.example / RUNBOOK.md / static/
```

### Docker

```bash
docker build -t lobby:0.1.0 .
docker run -d --name lobby \
    -p 8192:8192 \
    -v lobby-data:/var/lib/lobby \
    -v lobby-log:/var/log/lobby \
    -e LOBBY_LOG_FORMAT=json \
    lobby:0.1.0
```

镜像基于 Alpine + musl + tini，自带 HEALTHCHECK（`curl /` 每 15s 一次）。

---

## 部署

### 手动（无 systemd）

```bash
mkdir -p /opt/lobby /var/lib/lobby /var/log/lobby /etc/lobby
cp dist/lobby-0.1.0/{lobby,tictactoe} /usr/local/bin/
cp dist/lobby-0.1.0/games.toml /etc/lobby/
cp dist/lobby-0.1.0/lobby.env.example /etc/lobby/lobby.env
# 编辑 /etc/lobby/lobby.env 把 LOBBY_PUBLIC_HOST 改成对外域名
nohup /usr/local/bin/lobby &
```

### systemd（推荐）

```bash
sudo ./install.sh
sudo systemctl edit lobby         # 可选 drop-in
sudo systemctl start lobby
sudo systemctl status lobby
```

### nginx + Let's Encrypt

```bash
sudo apt install nginx certbot python3-certbot-nginx
sudo cp dist/lobby-0.1.0/nginx.conf.example /etc/nginx/sites-available/lobby
sudo ln -s /etc/nginx/sites-available/lobby /etc/nginx/sites-enabled/lobby
# 编辑 server_name lobby.example.com
sudo nginx -t && sudo systemctl reload nginx
sudo certbot --nginx -d lobby.example.com
```

`packaging/nginx.conf` 含：

- 80 → 443 重定向（保留 ACME 路径）
- WS 转发：`Upgrade`/`Connection` 头 + 长 timeout（3600s）
- `proxy_set_header X-Real-IP` 让 lobby 的 IP 限流生效
- 二次 `limit_req_zone`（login 10r/s、register 5r/s）

---

## 测试

### 单元测试

```bash
cargo test --workspace
# 8 passed: 密码策略 (3) + PoW (3) + 限流 (2)
```

### 端到端烟测（`tools/test.sh`）

```bash
# 启动 lobby
LOBBY_GAME_BIN=$(pwd)/target/debug/tictactoe cargo run -p lobby &

# 另一终端
PORT=8192 bash tools/test.sh
# 25 用例全部通过
```

覆盖：可达性、PoW、注册（缺/弱/重/正常密码）、登录（缺/对/错/无用户）、注册用户 B、登录 B、`GET /api/games`、建房间、加入、离开再加入、start、start 返回 ws_url、**WS 握手 + login + snapshot + move**、清理。

### 手动 WS 调试

```bash
# 用工具 ws_client.py
python3 tools/ws_client.py &
python3 -c "
import socket, sys
sys.path.insert(0, 'tools')
from ws_client import handshake, send_text, recv_text
sock = handshake('127.0.0.1', 8192, '/ws/1')
send_text(sock, '{\"type\":\"login\",\"uid\":1,\"session\":\"<token>\"}')
print(recv_text(sock))
"
```

---

## 添加新游戏

零侵入：Lobby / game-sdk **无需改动**。

```bash
# 1. 新建 crate
mkdir -p crates/games/<name>/src
cat > crates/games/<name>/Cargo.toml <<'EOF'
[package]
name = "<name>"
version.workspace = true
edition.workspace = true

[[bin]]
name = "<name>"
path = "src/main.rs"

[dependencies]
game-sdk = { path = "../../game-sdk" }
protocol = { path = "../../protocol" }
tokio = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
async-trait = "0.1"
tracing = { workspace = true }
anyhow = { workspace = true }
EOF

cat > crates/games/<name>/src/main.rs <<'EOF'
use async_trait::async_trait;
use game_sdk::{ActionOutcome, GameLogic, PhaseInfo};
use protocol::PlayerInit;
use serde_json::Value;

#[derive(Default, serde::Serialize, serde::Deserialize, Clone)]
pub struct MyConfig;

struct MyGame { /* state */ }

#[async_trait]
impl GameLogic for MyGame {
    type Config = MyConfig;
    fn new(_players: &[PlayerInit], _cfg: &MyConfig) -> Self { MyGame {} }
    fn snapshot(&self, _viewer: Option<i64>) -> Value { serde_json::json!({}) }
    fn handle_action(&mut self, _uid: i64, _action: Value) -> ActionOutcome {
        ActionOutcome::Reject("not implemented".into())
    }
    fn is_over(&self) -> bool { false }
    fn phase(&self) -> PhaseInfo { PhaseInfo { name: "playing".into(), active_player: None, awaiting: vec![], time_limit_ms: None } }
    fn validate_session(&self, _uid: i64, _session: &str) -> bool { true }
    // Hint: each PlayerInit carries `sessions: Vec<String>` — all non-expired
    // tokens Lobby has for that uid. Accept any of them on login/reconnect.
    fn min_players(&self) -> usize { 2 }
    fn max_players(&self) -> usize { 4 }
    fn game_name(&self) -> &'static str { "My Game" }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    game_sdk::init_tracing();
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut reader = BufReader::new(tokio::io::stdin()).lines();
    let init: protocol::LobbyInit = serde_json::from_str(&reader.next_line().await?.unwrap())?;
    game_sdk::run::<MyGame>(init).await
}
EOF

# 2. 加入 workspace
# 编辑根 Cargo.toml，在 members 加上 "crates/games/<name>"

# 3. 在 games.toml 注册
cat >> packaging/games.toml <<EOF

[[games]]
type = "<name>"
name = "..."
binary = "/usr/local/bin/<name>"
min_players = 2
max_players = 4
enabled = true
variants = ["classic", "blitz"]
EOF

# 4. 编译 & 部署
cargo build --release -p <name>
# 重启 lobby
```

游戏通过 `variants` 暴露 DLC；`POST /api/rooms {"game_type":"<name>","variant":"blitz"}` 即可建房。

---

## HTTP 接口摘要

| 方法 | 路径 | 鉴权 | 说明 |
|---|---|---|---|
| POST | `/api/register` | 无 | 注册；需 captcha + 强密码 |
| POST | `/api/login` | 无 | 登录；需 captcha，返回 session token |
| POST | `/api/captcha/challenge` | 无 | 获取 PoW challenge |
| GET | `/api/games` | 无 | 列出注册表中 enabled 的游戏 |
| GET | `/api/rooms` | 需 | 列出 Waiting/Running 房间 |
| POST | `/api/rooms` | 需 | 建房间；body 含 `game_type`、`variant?`、`config?` |
| GET | `/api/rooms/:id` | 需 | 房间详情 |
| POST | `/api/rooms/:id/join` | 需 | 加入 |
| POST | `/api/rooms/:id/leave` | 需 | 离开 |
| POST | `/api/rooms/:id/start` | 需 + host | 启动游戏，返回 `{instance_id, ws_url}` |
| GET | `/ws/:instance_id` | WS | 反代到对应 game |

错误统一：

```json
{"error":{"code":"<STRING_CODE>","message":"..."}}
```

完整字段与错误码见 `docs/protocol_spec.md`。

---

## WebSocket / 进程协议摘要

- 客户端 `ws://<public_host>:<port>/ws/<instance_id>`，Lobby 透明反代到 game 进程的 `127.0.0.1:<game_port>`
- Lobby **不解析** game 信封（保留职责边界）
- 详细帧格式与状态机见 `docs/protocol_spec.md §2`

---

## 错误码

完整列表见 `docs/protocol_spec.md §3`。常用：

| HTTP | code | 含义 |
|---|---|---|
| 400 | `INVALID_PARAMS` / `WEAK_PASSWORD` / `CAPTCHA_REQUIRED` / `CAPTCHA_INVALID` / `GAME_TYPE_UNSUPPORTED` | 客户端参数问题 |
| 401 | `INVALID_CREDENTIALS` / `UNAUTHORIZED` / `SESSION_EXPIRED` | 鉴权失败 |
| 404 | `USER_NOT_FOUND` / `ROOM_NOT_FOUND` / `INSTANCE_NOT_FOUND` / `NOT_FOUND` | 资源缺失 |
| 409 | `USERNAME_TAKEN` / `ROOM_FULL` / `ALREADY_IN_ROOM` / `ROOM_NOT_WAITING` / `NOT_ENOUGH_PLAYERS` / `INSTANCE_NOT_READY` | 状态冲突 |
| 429 | `RATE_LIMITED` | 触发限流 |
| 500 | `INSTANCE_START_FAILED` / `INSTANCE_ABNORMAL` / `INTERNAL_ERROR` | 服务端问题 |

WS 层错误用 `{"type":"error",...}` 或 `{"type":"game_error",...}`。

---

## 运维与故障排查

详见 [`packaging/RUNBOOK.md`](packaging/RUNBOOK.md)，要点：

```bash
# 实时日志
journalctl -u lobby -f
tail -f /var/log/lobby/lobby.YYYY-MM-DD.log

# 健康检查
curl --noproxy '*' http://127.0.0.1:8192/

# 升级（无停机：先拉流量再换）
sudo systemctl stop lobby           # SIGTERM 触发 5s 优雅关 game
sudo cp dist/lobby-<new> /usr/local/bin/lobby
sudo systemctl start lobby

# 备份 SQLite
sqlite3 /var/lib/lobby/lobby.db ".backup '/var/backups/lobby-$(date +%F).db'"

# 追踪某次请求
# 客户端带 X-Request-Id: <id>，再 grep journalctl：
journalctl -u lobby -o cat | grep "<id>"
```

---

## 已知边界（V1 范围内）

- 进程内限流（重启清零）；多实例需共享存储
- SQLite 单机；高并发写需迁 PostgreSQL
- 无 `/metrics` endpoint（先 journal + 日志）
- 前端 `static/app.js` 仍硬编码 `<option>tictactoe</option>`（纯前端工作）
- 分布式部署 / 网关 / 插件 / 匹配 / 好友 / 排行 / 录像不在 V1 范围（见 `docs/architecture.md §9`）