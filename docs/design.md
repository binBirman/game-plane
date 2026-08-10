# Lobby & Game 系统设计

Version: 0.2

基于 `docs/architecture.md` 与 `docs/game_integration_spec.md.md`。

- V1（已实现）：register/login、房间 CRUD、动态端口、WS 反向代理、心跳/生命周期、断线重连。详见 `docs/protocol_spec.md`。
- V1.1（计划中）：拆 `game` 为 `game-sdk` + 多游戏 crate；详见 `docs/games_architecture.md`。

本期范围：V1 + V1.1 计划。

分布式部署、Gateway、插件、Docker 调度、匹配、好友、排行榜、录像、多服务器同步不在范围。

## 1. 技术栈

### 1.1 语言与运行时

- Rust（edition 2021），Cargo workspace，Tokio 异步运行时。
- Lobby 与 Game 使用同一套 Rust 技术栈，跨进程线协议类型放共享 crate，保证两端一致。

### 1.2 依赖选型

Lobby：

| 依赖 | 用途 |
| --- | --- |
| axum | HTTP 路由 + WebSocket |
| sqlx (sqlite) | 异步数据库访问 |
| serde / serde_json | 序列化 |
| tokio | 运行时、`process` 进程管理 |
| argon2 | 密码哈希 |
| rand | Session token 生成 |
| tracing | 日志 |
| tokio-util | stdout 按行读取 |

Game：

| 依赖 | 用途 |
| --- | --- |
| axum 或 tokio-tungstenite | WebSocket 服务端 |
| serde_json | 消息解析 |
| rand | 游戏随机性 |
| tokio-util | stdin 按行读取 |

### 1.3 仓库结构（Cargo Workspace）

V1（当前）：

```
game-plane/
├── Cargo.toml            # workspace
├── crates/
│   ├── protocol/         # 共享类型：LobbyInit、GameEvent、LobbyCommand、WS 信封
│   ├── lobby/            # Lobby Server
│   └── game/             # V1 单文件 tictactoe
└── docs/
```

V1.1（计划，详见 `docs/games_architecture.md`）：

```
game-plane/
├── Cargo.toml
├── crates/
│   ├── protocol/
│   ├── lobby/
│   ├── game-sdk/         # ✓ 已完成：通信骨架 + GameLogic trait
│   └── games/
│       ├── tictactoe/    # ✓ 已完成：迁移自 crates/game/，binary 名 `tictactoe`
│       ├── <game-A>/     # 待用户给规则
│       └── <game-B>/     # 待用户给规则
└── docs/
```

### 1.4 配置

- Lobby：环境变量 + 配置文件（监听端口、DB 路径、Game 二进制注册表、`public_host`、Game 端口范围）。
- Game：初始化数据由 Lobby 在启动时以 stdin 首行传入，不依赖 argv 长参数。

## 2. 数据库表（SQLite）

启动时自动执行迁移（`CREATE TABLE IF NOT EXISTS`）。

```sql
CREATE TABLE users (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,             -- argon2
    nickname      TEXT NOT NULL,
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE sessions (
    token      TEXT PRIMARY KEY,             -- 随机 64 hex
    user_id    INTEGER NOT NULL REFERENCES users(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at TEXT NOT NULL
);

CREATE TABLE rooms (
    room_id    INTEGER PRIMARY KEY AUTOINCREMENT,
    game_type  TEXT NOT NULL,
    host_uid   INTEGER NOT NULL,
    status     TEXT NOT NULL DEFAULT 'Waiting',  -- Waiting/Starting/Running/Finished/Destroyed
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE room_players (
    room_id   INTEGER NOT NULL REFERENCES rooms(room_id),
    uid       INTEGER NOT NULL REFERENCES users(id),
    seat      INTEGER NOT NULL,
    joined_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (room_id, uid)
);

CREATE TABLE game_instances (
    instance_id INTEGER PRIMARY KEY AUTOINCREMENT,
    room_id     INTEGER NOT NULL REFERENCES rooms(room_id),
    pid         INTEGER,
    port        INTEGER,
    status      TEXT NOT NULL,               -- starting/ready/running/finished/abnormal/stopped
    start_time  TEXT,
    end_time    TEXT
);

CREATE INDEX idx_sessions_expires ON sessions(expires_at);
CREATE INDEX idx_room_players_uid ON room_players(uid);
CREATE INDEX idx_instances_room ON game_instances(room_id);
```

要点：

- Room 与 GameInstance 分离：一次开局建一行 `game_instances`，房间结束后可重新开局，旧实例作为历史记录保留。
- Room 状态存 `rooms.status`；`game_instances.status` 记录实例自身生命周期。
- 不存任何游戏状态；游戏状态由 Game 进程自主管理。

## 3. 模块划分

### 3.1 Crate: protocol（共享）

定义全部跨进程 / 跨网络消息的 Rust 类型（serde），Lobby 与 Game 共同依赖：

- `LobbyInit`：初始化数据（room_id / game_type / listen / players）
- `GameEvent`：ready / running / finished / shutdown / heartbeat
- `LobbyCommand`：start / stop
- WS 消息信封 `WsMsg { type, ... }`

### 3.2 Crate: lobby

```
src/
├── main.rs
├── config.rs          # 配置加载
├── state.rs           # AppState：DbPool + InstanceManager
├── db/
│   ├── mod.rs
│   └── migrations.rs  # 启动迁移
├── auth/
│   ├── password.rs    # argon2 哈希 / 校验
│   ├── session.rs     # token 生成 / 校验
│   └── extractor.rs   # axum 鉴权 extractor
├── http/
│   ├── router.rs
│   ├── user.rs        # register / login
│   ├── room.rs        # create / join / query / start
│   └── error.rs       # 统一错误响应
├── ws_proxy/          # WS 反向代理（Lobby -> Game，见 protocol_spec §2.4）
│   ├── mod.rs
│   ├── router.rs      # /ws/:instance_id 入口
│   ├── upstream.rs    # 与 Game 的 TCP 连接
│   └── pipe.rs        # 双向帧透传
├── service/
│   ├── user.rs
│   ├── room.rs
│   └── game.rs        # 开始流程：分配端口 -> 建实例 -> spawn
└── instance/
    ├── manager.rs     # 实例注册表 + 端口分配器
    ├── process.rs     # tokio::Command 启动、stdin/stdout 读写
    ├── lifecycle.rs   # stdout 事件分发、状态机
    └── watchdog.rs    # 心跳超时(15s)检测与异常回收
```

职责要点：

- `http/` 薄层：只做参数解析与错误映射，业务在 `service/`。
- `ws_proxy/`：按 `instance_id` 路由 WS 到 Game；仅做传输层字节透传，不解析 game 信封。
- `instance/manager`：持有 `HashMap<instance_id, GameProcess>`；端口分配用"临时 bind `127.0.0.1:0` 获取空闲端口再释放"或范围扫描，分配后写入 init JSON。
- `instance/watchdog`：每 5s 扫描一次心跳时间戳，超 15s 无心跳 -> 标记 abnormal -> kill -> 清理实例、释放端口、更新 Room 状态。

### 3.3 Crate: game（V1 单文件实现）/ V1.1 多 crate

**V1（当前）**：`crates/game/` 是单 binary，骨架与井字棋耦合。模块分工：

```
crates/game/src/main.rs
├── init 解析（stdin 首行 LobbyInit）
├── WS 服务（axum）
├── session 校验（player_sessions 白名单）
├── heartbeat 循环
├── stdin 命令处理（start/stop）
└── 井字棋规则（GameState、move、win 检测）
```

**V1.1（进行中，详见 `docs/games_architecture.md`）**：

- ✓ `crates/game-sdk/` 已抽出通信骨架，提供 `GameLogic` trait 与 `run()` 入口。
- ✓ `crates/games/tictactoe`（已迁移）以及待实现的 `<game-A>`、`<game-B>` 各自只实现 `GameLogic`。
- ✗ 新游戏的 `games.toml` 注册表、`GET /api/games`、前端多游戏卡片尚未做。
- 新游戏接入路径明确：新建 `crates/games/<name>` + 在 `games.toml` 注册，Lobby / SDK 不动。

## 4. 接口协议

> 状态机、接口字段、错误码的权威定义见 `docs/protocol_spec.md`；本章为概览。

### 4.1 HTTP（Client <-> Lobby）

统一约定：

- JSON body，`Content-Type: application/json`。
- 需鉴权接口带 `Authorization: Bearer <token>`。
- 错误统一返回 `{"error":{"code":"<字符串错误码>","message":"..."}}`（编码表见 `docs/protocol_spec.md` §3）。

| 方法 | 路径 | 鉴权 | 请求 | 响应 |
| --- | --- | --- | --- | --- |
| POST | /api/register | 无 | `{username,password,nickname}` | `{uid}` |
| POST | /api/login | 无 | `{username,password}` | `{token,uid}` |
| POST | /api/rooms | 需 | `{game_type}` | `{room_id,status}` |
| GET | /api/rooms/:id | 需 | - | 房间信息 |
| POST | /api/rooms/:id/join | 需 | - | 房间信息 |
| POST | /api/rooms/:id/leave | 需 | - | `{ok:true}` |
| POST | /api/rooms/:id/start | 需(host) | - | `{instance_id,port,ws_url}` |

房间信息结构：

```json
{
  "room_id": 1001,
  "game_type": "tictactoe",
  "host_uid": 1,
  "status": "Waiting",
  "players": [{"uid": 1, "nickname": "alice", "seat": 0}]
}
```

`start` 响应中 `ws_url` 形如 `ws://<public_host>:8080/ws/<instance_id>`，客户端只连 Lobby 单端口，由 Lobby 反向代理到 Game（详见 protocol_spec §2.4）。`start` 同时驱动 Lobby 内部：分配端口（127.0.0.1） -> 建 GameInstance -> spawn 进程。

### 4.2 进程线协议（Lobby <-> Game）

stdin / stdout，每行一个 JSON 对象（`\n` 分隔），UTF-8。

初始化：Lobby 启动 Game 进程时，将初始化 JSON 作为 stdin 首行写入：

```json
{"room_id":1001,"game_type":"tictactoe","listen":"127.0.0.1:41001","players":[{"uid":1,"session":"xxxx"}]}
```

`listen` 固定为 `127.0.0.1:<port>`，Game 不暴露公网。

Game -> Lobby 事件（stdout）：

| 事件 | 说明 | 附加字段 |
| --- | --- | --- |
| ready | 初始化完成，开始监听 | port |
| running | 游戏开始 | - |
| finished | 游戏结束 | result（可选） |
| shutdown | 准备退出 | - |
| heartbeat | 心跳 | ts |

示例：

```json
{"event":"ready","port":41001}
{"event":"running"}
{"event":"finished","result":{"winner":1}}
{"event":"shutdown"}
{"event":"heartbeat","ts":1700000000}
```

Lobby -> Game 命令（stdin）：

```json
{"cmd":"start","ts":1700000000}
{"cmd":"stop","reason":"room_destroyed","ts":1700000000}
```

- `start`：Lobby 通知正式开始（用于等待全部玩家连接后再开局的场景）。
- `stop`：Lobby 主动要求退出（房间销毁、房主踢人等回收场景）。

心跳约定：Game 每 5s 发一次 `heartbeat`；Lobby 超过 15s 未收到即判定实例异常。

### 4.3 WebSocket（Client <-> Game）

客户端连接 `ws://<host>:<port>/ws`，消息为 JSON，含 `type` 字段。

连接与恢复：

```json
{"type":"login","uid":1,"session":"xxxx"}
{"type":"reconnect","uid":1,"session":"xxxx"}
```

Game 响应：

```json
{"type":"login_ok"}
{"type":"snapshot","state":{}}
{"type":"error","code":"INVALID_SESSION","message":"invalid session"}
```

- 首次登录成功：先回 `login_ok`，随后视游戏流程推送快照或开始消息。
- 重连成功：绑定新 Connection，推送完整 `snapshot`，客户端据此恢复。

游戏消息信封（具体游戏自定义 `data`）：

```json
{"type":"game","data":{...}}
{"type":"game_error","code":...,"message":"..."}
```

保活：客户端每 15s 发 `{"type":"ping"}`，Game 回 `{"type":"pong"}`。

断线重连：

- 断线：socket 关闭，Player 保留（含游戏状态）。
- 重连：验证 session -> 查找 Player -> 绑定新 Connection -> 推送 snapshot。
- 超时未重连（可配置，默认 30s）：可选标记玩家离开。

## 5. 与现有文档的一致性

- 沿用 Lobby / Game 职责边界：Lobby 不参与任何游戏逻辑，不保存游戏状态。
- 沿用 Room 与 GameInstance 分离、动态端口、15s 心跳、Player != Connection 原则。
- 认证模型不变：Lobby 签发 Session，启动 Game 时传入，Game 验证但不存密码、不依赖 JWT。
- WS 反向代理：客户端只连 Lobby 单一公网端口，Game 仅绑 127.0.0.1；Lobby 不解析 game 信封，不破坏"Lobby 不参与游戏逻辑"原则。
- 多游戏扩展：V1.1 起，`GameLogic` trait + `games/` 子目录 + `games.toml` 注册表是新增游戏的唯一入口，详见 `docs/games_architecture.md`。
