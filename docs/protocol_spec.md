# 状态、接口与错误码规范（V1 Protocol Spec）

Version: 0.2

本文件是 `docs/design.md` 的细化，定义 V1 的：

1. 状态机（Room / GameInstance / Player-Connection / Game 进程）
2. 接口契约（HTTP / 进程线协议 / WebSocket）
3. 错误码

V1 范围：注册、登录/会话鉴权、房间管理、游戏启动、动态端口、WebSocket、生命周期管理、心跳检测、断线重连、WS 反向代理。

---

## 1. 状态机

### 1.1 Room 状态机

存于 `rooms.status`。

状态：

```
   (create)
      │
      ▼
   Waiting ──start──► Starting ──ready──► Running ──finished──► Finished
      │                  │                   │                     │
      │                  │                   │                     │
      │                  │                   │   restart           │
      │                  ▼                   ▼                     ▼
      └───────────────► Destroyed ◄──────────┴─────────────────────┘
```

补充说明：

- `Finished → Starting` 的 restart 由房主 POST start 触发（**同一房间多局游戏**：一局结束后房间仍在，可继续 start）。
- 进入 `Destroyed` 的唯一条件：**全员离开**。不另设"空房超时"。实例异常 / 心跳超 15s / 进程崩溃 → `Waiting`（可重新开局），不是 `Destroyed`。
- `Starting` 启动失败（spawn 失败 / 进程立即退出 / ready 超时 10s）回滚到 `Waiting`，可重试。

| 当前 | 触发 | 条件/事件 | 目标 |
| --- | --- | --- | --- |
| - | 创建房间 | POST /api/rooms | Waiting |
| Waiting | 房主 start | POST start，人数满足（≥ 2） | Starting |
| Starting | Game ready | 收到 `{"event":"ready"}` | Running |
| Starting | 启动失败 | spawn 失败 / 进程立即退出 / ready 超 10s | Waiting（回滚，可重试） |
| Running | Game 结束 | 收到 `{"event":"finished"}` | Finished |
| Finished | 重新开局 | 房主 POST start（**新 GameInstance**） | Starting |
| Waiting/Finished | 玩家 join | 人数未满 | （房间不变） |
| Running/Finished | 玩家 leave | （房间不变） | — |
| 任意非 Destroyed | **最后一人** leave | `room_players` 空 | Destroyed |
| 任意非 Destroyed | 实例异常 | watchdog 判定（心跳超 15s / 进程崩溃） | Waiting（可重新开局） |
| 任意非 Destroyed | 房主 leave | 提早加入者为新 host；空房 → Destroyed | （详见 `room_lifecycle.md` §6） |

不变量：

- **房间 ≠ 一局游戏**：房间可经历多轮 `Waiting → Starting → Running → Finished → Starting → …`。
- 只有 `Waiting` 状态允许 `POST /api/rooms/:id/join`（多局中途不接收新玩家）。
- `Waiting` 和 `Finished` 状态允许房主 start；`Running` 期间不可重复 start。
- `leave` 在任何非 `Destroyed` 状态都允许；只有"最后一人离开"才进 `Destroyed`。
- 任何非 `Destroyed` 房间的 `rooms.host_uid` 必指向 `room_players` 里一名在房玩家（leave handler 保证）。

完整设计见 `docs/room_lifecycle.md`（含 host 禅让策略、GameInstance 状态机、失败恢复、下一周期路线）。

### 1.2 GameInstance 状态机

存于 `game_instances.status`。

```
              ready
   starting ────────► running
       │                │  │
       │ 异常            │  │ finished
       ▼                ▼  ▼
    abnormal          finished ──► stopped
       ▲                 │
       └─────────────────┘  (shutdown / 进程退出)
```

| 当前 | 触发 | 目标 |
| --- | --- | --- |
| - | Lobby 分配端口 + 建实例记录 + spawn 进程 | starting |
| starting | 收到 `{"event":"ready"}` | ready |
| starting | spawn 失败 / 进程退出 / 启动超时(10s) | abnormal |
| ready | Lobby 发 `{"cmd":"start"}` 或收到 `{"event":"running"}` | running |
| ready/running | 收到 `{"event":"finished"}` | finished |
| ready/running | 心跳超 15s / 进程崩溃 | abnormal |
| finished | 收到 `{"event":"shutdown"}` 或进程退出 | stopped |
| 任意 | Lobby 主动 `{"cmd":"stop"}` 并 kill | stopped |

Lobby 清理流程（进入 stopped / abnormal / finished 后）：

```
标记 game_instances.status 终态（finished/abnormal/stopped）
更新 rooms.status（Finished/Destroyed/Waiting）
```

### 1.3 Player / Connection 状态（Game 内部）

原则：`Player != Connection`。

Player（持久，跨连接存在）：

```
   online ◄──── reconnect ────► disconnected ────► left
     │                                              ▲
     └────────────── leave ──────────────────────────┘
```

| 状态 | 说明 |
| --- | --- |
| online | 验证通过，已绑定活跃 Connection |
| disconnected | socket 关闭，Player 状态保留（含游戏状态） |
| left | 超时未重连(30s，可配置) 或 玩家主动离开 |

Connection（短暂，一次连接）：

```
   auth_pending ── login/reconnect 成功 ──► bound ──► closed
        │                                       ▲
        └── 验证失败 / 超时 ──► closed           │
                                                │
                          socket 关闭 ──────────┘
```

| 状态 | 说明 |
| --- | --- |
| auth_pending | socket 已建立，尚未 login/reconnect |
| bound | 已 login/reconnect，绑定到 Player（且替换该 Player 旧 Connection） |
| closed | socket 已关闭 |

不变量：

- 每个 Player 最多绑定一个活跃 Connection；新连接 `bound` 会替换旧 Connection（关闭旧的）。
- 断线只关 socket，不清 Player。
- 重连验证成功后，新 Connection `bound` 到原 Player，并推送完整 `snapshot`。
- **会话校验**：login/reconnect 提交的 `session` 必须与 Lobby 在 `LobbyInit.players[*].sessions` 中传入的某个 token 匹配（一个用户可能持有多个未过期 token —— 比如在另一个标签里重新登录过），否则 Game 返回 `error:INVALID_SESSION` 并关闭连接。Game 端按"任一匹配即可"校验。

### 1.4 Game 进程状态

```
initializing ──► ready ──► running ──► finished ──► shutdown ──► exit
```

| 状态 | 行为 |
| --- | --- |
| initializing | 读 stdin 首行 init JSON，绑定 WS 监听端口（127.0.0.1） |
| ready | 监听成功，发 `ready`，等待玩家连接 |
| running | 收到 `cmd:start` 或 Game 自主开局 |
| finished | 游戏结束，发 `finished` |
| shutdown | 发 `shutdown`，退出进程 |

---

## 2. 接口契约

### 2.1 HTTP（Client ↔ Lobby）

通用约定：

- Base：`http://<lobby-host>:<lobby-port>`
- 请求/响应体均为 JSON，`Content-Type: application/json`
- 鉴权：`Authorization: Bearer <token>`（除 register/login/captcha 外必须）
- 失败响应统一：

```json
{"error":{"code":"<字符串错误码>","message":"<人类可读信息>"}}
```

#### 2.1.1 注册

```
POST /api/register
Body: {"username":"alice","password":"secret123","nickname":"Alice","captcha":{"challenge":"...","nonce":"..."}}
200: {"uid":1}
```

密码策略（V1）：

- 长度 ≥ 9 字符
- 必须同时包含：数字、字母、非字母数字字符（特殊字符）
- 不满足 → `400 WEAK_PASSWORD`，message 指出原因

#### 2.1.2 登录

```
POST /api/login
Body: {"username":"alice","password":"secret123","captcha":{"challenge":"...","nonce":"..."}}
200: {"uid":1,"token":"<64hex>"}
```

#### 2.1.3 人机验证（PoW）

```
POST /api/captcha/challenge
Auth: 无
200: {"challenge":"<32hex>","difficulty":16,"ttl_seconds":300}
```

客户端需找到 nonce，使得 `SHA256(challenge + ":" + nonce)` 的十六进制表示前导零 bit 数 ≥ `difficulty`。

服务验证：无状态，对任意 `(challenge, nonce)` 对，前导零 ≥ 难度（默认 16，环境变量 `LOBBY_POW_DIFFICULTY` 可调）即接受。

#### 2.1.4 创建房间（需鉴权）

```
POST /api/rooms
Auth: Bearer <token>
Body: {"game_type":"tictactoe"}
201: {"room_id":1001,"status":"Waiting"}
```

`game_type` V1 仅支持 `tictactoe`。

#### 2.1.5 房间列表（需鉴权）

```
GET /api/rooms
Auth: Bearer <token>
200: {"rooms":[{room_id, game_type, host_uid, status, players:[...]}]}
```

仅返回状态为 `Waiting` 或 `Running` 的房间。

#### 2.1.6 查询房间（需鉴权）

```
GET /api/rooms/:room_id
Auth: Bearer <token>
200: RoomInfo
```

#### 2.1.7 加入房间（需鉴权）

```
POST /api/rooms/:room_id/join
Auth: Bearer <token>
200: RoomInfo
```

前置条件：房间状态 `Waiting`，未满（V1：2 人），未加入。

#### 2.1.8 离开房间（需鉴权）

```
POST /api/rooms/:room_id/leave
Auth: Bearer <token>
200: {"ok":true}
```

若离开的是**房主**且房间仍有其他玩家，Lobby 自动把 `joined_at` 最早的剩余玩家提为新房主（`UPDATE rooms SET host_uid = ...`），保证房间始终有房主。若房间变空，更新 `rooms.status='Destroyed'`。

#### 2.1.9 开始游戏（需鉴权，仅房主）

```
POST /api/rooms/:room_id/start
Auth: Bearer <token>
200: {"instance_id":101,"ws_url":"ws://<public_host>:8192/ws/101"}
```

前置条件：房主 + 房间状态 `Waiting` + ≥ 2 玩家。

副作用：

1. `rooms.status='Starting'`
2. 分配 127.0.0.1 端口
3. spawn `game` 子进程，stdin 首行写 `LobbyInit`
4. 插入 `game_instances` 行（status=starting）
5. 启动 stdout 事件循环（更新 `game_instances.status`）
6. 返回 `ws_url`（客户端单端口反代到 Game）

#### 2.1.10 RoomInfo

```json
{
  "room_id": 1001,
  "game_type": "tictactoe",
  "host_uid": 1,
  "status": "Waiting",
  "players": [
    {"uid":1,"nickname":"Alice","seat":0},
    {"uid":2,"nickname":"Bob","seat":1}
  ]
}
```

### 2.2 进程线协议（Lobby ↔ Game）

- 通道：stdin / stdout，UTF-8，每行一个 JSON 对象（`\n` 结尾）。
- stderr 仅供日志，不参与协议。

#### 2.2.1 初始化（stdin 首行）

```json
{"room_id":1001,"game_type":"tictactoe","listen":"127.0.0.1:41001","players":[{"uid":1,"sessions":["<token-a>","<token-b>"]},{"uid":2,"sessions":["<token>"]}]}
```

字段见 `docs/protocol_spec.md` §1.3（PlayerInit）；`listen` 固定为 `127.0.0.1:<port>`（仅回环，Game 不对外暴露）。

#### 2.2.2 Game → Lobby 事件（stdout）

```json
{"event":"ready","port":41001}
{"event":"running","ts":1700000000}
{"event":"finished","result":{"winner":1},"ts":1700000000}
{"event":"shutdown","ts":1700000000}
{"event":"heartbeat","ts":1700000000}
```

| 事件 | 触发时机 | 必选字段 | 可选字段 |
| --- | --- | --- | --- |
| ready | 监听成功后 | event | port, ts |
| running | 游戏开局 | event | ts |
| finished | 游戏结束 | event | result, ts |
| shutdown | 准备退出 | event | ts |
| heartbeat | 每 5s | event | ts |

#### 2.2.3 Lobby → Game 命令（stdin）

```json
{"cmd":"start","ts":1700000000}
{"cmd":"stop","reason":"room_destroyed","ts":1700000000}
```

| 命令 | 说明 |
| --- | --- |
| start | 通知正式开始（Game 进入 running 并可选广播开始消息） |
| stop | Lobby 主动要求退出（收到后应发 shutdown 并退出；Lobby 等 5s 未退则 kill） |

### 2.3 WebSocket（Client → Lobby → Game）

- 地址：`ws://<public_host>:<lobby_port>/ws/<instance_id>`
- 消息：JSON，首个字段固定为 `type`。
- 客户端先连 Lobby，再由 Lobby 反向代理到对应 Game（详见 §2.4）。

#### 2.3.1 Client → Game

```json
{"type":"login","uid":1,"session":"<token>"}
{"type":"reconnect","uid":1,"session":"<token>"}
{"type":"ping"}
{"type":"game","data":{...}}
```

| type | 说明 |
| --- | --- |
| login | 首次登录（连接后首条必须为 login 或 reconnect）；session 必须匹配 `LobbyInit.players[*].sessions` 中的任一 token |
| reconnect | 断线重连，需之前登录过该局；session 校验同 login |
| ping | 保活，Game 回 pong |
| game | 游戏操作，data 由具体游戏定义 |

#### 2.3.2 Game → Client

```json
{"type":"login_ok"}
{"type":"snapshot","state":{...}}
{"type":"pong"}
{"type":"game","data":{...}}
{"type":"game_error","code":"INVALID_MOVE","message":"..."}
{"type":"error","code":"INVALID_SESSION","message":"..."}
```

| type | 说明 |
| --- | --- |
| login_ok | 首次登录成功；随后按游戏流程推 snapshot 或开始消息 |
| snapshot | 重连/进入后完整状态，state 由具体游戏定义 |
| pong | 回应 ping |
| game | 游戏状态更新 / 回合推送 |
| game_error | 游戏规则层面的操作被拒 |
| error | 连接/认证层错误（如 INVALID_SESSION、ALREADY_LOGGED_IN） |

握手顺序：

```
首次连接: login ─► login_ok ─► snapshot ─► (恢复操作)
重连:     reconnect ─► login_ok ─► snapshot ─► (恢复操作)
```

### 2.4 WS 反向代理（Lobby → Game）

V1 客户端只连 Lobby 单一端口，由 Lobby 透明转发到 Game。

路由：

- 客户端请求：`ws://<public_host>:<lobby_port>/ws/<instance_id>`
- Lobby 按 `instance_id` 查找 `game_instances`：不存在 → HTTP 404（`INSTANCE_NOT_FOUND`）；实例未就绪（status ≠ ready/running）→ HTTP 409（`INSTANCE_NOT_READY`）。
- 找到后，Lobby 与 Game 在 `127.0.0.1:<port>` 建立 TCP 连接，做 WS 握手（lobby-as-client），双向透传 WS 帧。

转发行为：

- Lobby **不解析 game 信封**，仅做传输层字节转发。
- Session 校验由 Game 端进行（与 `LobbyInit.players[*].sessions` 列表**任一匹配**即可）；校验失败 → Game 发 `error:INVALID_SESSION` 并关闭连接。
- 断连：客户端 socket 关闭 → Lobby 关闭 Game 侧连接；Game 侧断开 → Lobby 关闭客户端连接。

拓扑：

```
Client ──ws──► Lobby (:8192, 0.0.0.0) ──tcp/ws──► Game (:41001, 127.0.0.1)
```

要点：

- Game 不再绑定公网地址，部署时无需在防火墙开放 Game 端口段。
- `start` 响应中 `ws_url` 已含 `instance_id`；客户端连接代码与 Game 实际端口解耦。
- 反代是传输层职责，不破坏"Lobby 不参与游戏逻辑"原则。

---

## 3. 错误码

所有错误码为字符串，在 HTTP 与 WS 中复用同一套编码。

### 3.1 通用

| code | HTTP | 说明 |
| --- | --- | --- |
| INVALID_PARAMS | 400 | 参数缺失或格式错误 |
| NOT_FOUND | 404 | 资源不存在 |
| INTERNAL_ERROR | 500 | 服务器内部错误 |
| METHOD_NOT_ALLOWED | 405 | 方法不允许 |

### 3.2 认证（AUTH_*）

| code | HTTP | 说明 |
| --- | --- | --- |
| USERNAME_TAKEN | 409 | 用户名已被注册 |
| USER_NOT_FOUND | 404 | 用户不存在 |
| INVALID_CREDENTIALS | 401 | 用户名或密码错误 / token 无效 / 过期 |
| UNAUTHORIZED | 401 | 缺少 token |
| SESSION_EXPIRED | 401 | token 已过期 |

### 3.3 密码与验证

| code | HTTP | 说明 |
| --- | --- | --- |
| WEAK_PASSWORD | 400 | 密码不满足强度（message 指出原因） |
| CAPTCHA_REQUIRED | 400 | 缺少 captcha 字段 |
| CAPTCHA_INVALID | 400 | PoW 校验失败 |

### 3.4 房间（ROOM_*）

| code | HTTP | 说明 |
| --- | --- | --- |
| ROOM_NOT_FOUND | 404 | 房间不存在 |
| ROOM_FULL | 409 | 房间已满 |
| ALREADY_IN_ROOM | 409 | 已在房间内 |
| NOT_IN_ROOM | 403 | 不在房间内 |
| NOT_HOST | 403 | 仅房主可执行此操作 |
| ROOM_NOT_WAITING | 409 | 房间不在 Waiting/Finished 状态 |
| NOT_ENOUGH_PLAYERS | 409 | 不满足开始条件（人数不足） |
| GAME_TYPE_UNSUPPORTED | 400 | 不支持的 game_type |
| ROOM_DESTROYED | 410 | 房间已销毁 |

### 3.5 实例（INSTANCE_*）

| code | HTTP | 说明 |
| --- | --- | --- |
| INSTANCE_START_FAILED | 500 | 实例启动失败（spawn 异常/超时等，message 含 cause） |
| GAME_BINARY_NOT_FOUND | 503 | `LOBBY_GAME_BIN` / `games.toml` 指定的 binary 不存在或无 PATH |
| INSTANCE_ABNORMAL | 500 | 实例异常（心跳超时/崩溃） |
| INSTANCE_NOT_READY | 409 | 实例未就绪（WS 代理拒绝转发） |
| INSTANCE_NOT_FOUND | 404 | instance_id 不存在（WS 代理路由失败） |

### 3.6 Game 侧（WS 专属）

| code | 类型 | 说明 |
| --- | --- | --- |
| INVALID_SESSION | error | session 与 LobbyInit 不匹配（Game 内验证失败） |
| ALREADY_LOGGED_IN | error | 已登录后又收到 login/reconnect |
| BAD_FRAME | error | 首帧不是 login/reconnect 或 JSON 解析失败 |
| PLAYER_NOT_IN_GAME | game_error | 玩家不在此局中（保留） |
| ACTION_INVALID | game_error | 游戏操作不合法（规则外） |
| INVALID_MOVE | game_error | 非当前回合或落子非法 |
| NOT_YOUR_TURN | game_error | 非当前回合 |
| GAME_NOT_RUNNING | game_error | 游戏未开始或已结束 |

### 3.7 响应示例

HTTP 失败：

```json
{"error":{"code":"NOT_HOST","message":"only host can start the game"}}
```

WS 失败：

```json
{"type":"error","code":"INVALID_SESSION","message":"session not valid"}
{"type":"game_error","code":"INVALID_MOVE","message":"not your turn / bad cell"}
```

---

## 4. 一致性约束

- 所有状态流转、事件顺序必须与 §1 一致；新增流转须同步更新本文件。
- 错误码先于新功能定义：新接口必须复用 §3 已有编码，确需新增时追加到对应分组并登记 HTTP 状态。
- WS `session` 字段必填；Game 校验失败统一返回 `error:INVALID_SESSION`。
- `ws_url` 必含 `instance_id`；客户端无须感知 Game 实际端口。