# 房间生命周期（V1）

> 适用版本：`lobby` v0.1.x（`crates/lobby/`）。状态机定义见 `crates/lobby/src/http/room.rs` 与 `crates/lobby/src/instance/manager.rs`；错误码表见 `docs/protocol_spec.md` §3。

## 1. 设计原则（已采纳）

| # | 原则 | 含义 |
|---|---|---|
| 1 | 房间持续存在直至无人 | 没有"空房超时"。房间有人就一直存活；最后一个人离开才进 `Destroyed`。 |
| 2 | 房间 ≠ 一局游戏 | 房间是 **容器**，可连续多局；游戏结束回到房间界面，host 可在同一房间内反复 start。 |
| 3 | host 禅让走心跳 | host 禅让 **不另开超时**；下一周期通过 WS 心跳检测 host 在线状态后做禅让。V1 仅"显式 leave 时禅让"。 |
| 4 | 其他弱点（见 §9）下一周期补 | `current_instance_id` 清理、孤儿 `game_instances` 清理等。 |

## 2. 两层状态机

房间是长寿命的容器；游戏实例是短命的事件。

```
        ┌──── games: N 个 ────┐
        │                     │
        ▼                     │
  ┌───────────┐  start      ┌──────────┐  ready   ┌──────────┐
  │ Waiting   │ ──────────► │ Starting │ ───────► │ Running  │
  │ (host=创建) │           │          │          │(WS 活跃) │
  └─────┬─────┘           └────┬─────┘          └────┬─────┘
        │                       │ spawn 失败回滚     │ finished
        │                       └──────────────┐    │
        │ join / leave（始终允许）             ▼    ▼
        │                                  Waiting ┌─────────┐
        ▼                                       │Finished  │
  ┌─────────────┐  全员 leave                  │(等待再开)│
  │ Destroyed   │ ◄────────────────────────────┴────┬────┘
  │  (终态)     │                                   │ start
  └─────────────┘                                   └────┘
        ▲
        │  注意：Waiting|Finished 不删房间，仅 host leave 且房间为空 → Destroyed
        └────────────────────────────────────────────────────────┘
```

`game_instances` 是子表：同一 `room_id` 可有 N 行（一局一行）；`instance_id` 自增。

## 3. Room 状态机（`rooms.status`）

| 当前 | 触发 | 条件 | 目标 | 代码 |
|---|---|---|---|---|
| — | `POST /api/rooms` | 创建 | `Waiting` | `room.rs:86` |
| `Waiting`/`Finished` | `POST /api/rooms/:id/start`（host） | 人数 `≥ min_players`；binary 预检通过 | `Starting` | `room.rs:316` |
| `Starting` | 实例收到 `{"event":"ready"}` | spawn 成功 + 收到 ready | `Running` | `instance/manager.rs:178` |
| `Starting` | spawn 失败 / 进程立即崩 / ready 超时（10s） | — | `Waiting`（回滚，可重试） | `room.rs:455`, `instance/manager.rs:273` |
| `Running` | 实例收到 `{"event":"finished"}` | 游戏结束 | `Finished` | `instance/manager.rs:200` |
| `Running` | 实例异常（心跳 >15s / 进程崩 / stdout EOF） | watchdog 判定 | `Waiting`（可重新开局） | `instance/manager.rs:236,353` |
| `Finished` | host 再 start | 满足 start 前置 | `Starting`（**新 `instance_id`**） | `room.rs:316` |
| `Running`/`Finished` | 玩家 leave | 玩家自己 POST | （房间不变） | `room.rs:262` |
| `任意非 Destroyed` | **最后一人** leave | `room_players` 空 | `Destroyed` | `room.rs:299` |
| `Waiting`/`Finished`/`Running` | 玩家 join | 未满 | （房间不变，玩家加入） | `room.rs:181` |

**不变量**：

- 房间状态机只允许 `Waiting ↔ Starting → Running → Finished`；不存在 `Destroyed → 任何状态`。
- 同一房间可经历多次 `Waiting → Starting → Running → Finished` 循环（即同一房间多局）。
- 只有 `Waiting` 接受 `POST /api/rooms/:id/join`；其他状态 join 返 `409 ROOM_NOT_WAITING`。
- 只有 `Waiting` 和 `Finished` 接受 `POST /api/rooms/:id/start`；其他状态 start 返 `409 ROOM_NOT_WAITING`。
- 任何非 `Destroyed` 状态下 `rooms.host_uid` 必指向 `room_players` 里一名在房玩家。

## 4. GameInstance 状态机（`game_instances.status`）

| 当前 | 触发 | 目标 | 代码 |
|---|---|---|---|
| — | spawn() | `starting` | `instance/manager.rs:127` |
| `starting` | 收到 `{"event":"ready"}` | `ready` | `instance/manager.rs:176` |
| `ready` | 发 `cmd:start` 后收到 `{"event":"running"}` | `running` | `instance/manager.rs:190` |
| `running` | 收到 `{"event":"finished"}` | `finished` + kill 子进程 + 内存移除 | `instance/manager.rs:198` |
| `starting`/`ready`/`running` | stdout EOF（进程崩）/ 心跳 > 15s / ready > 10s | `abnormal` + 回滚 rooms=`Waiting` | `instance/manager.rs:236, 273, 353` |
| 任意非终态 | Lobby 发 `cmd:stop` + 等 5s | `stopped`（graceful）→ 否则 SIGKILL | `instance/manager.rs:316` |
| 任意非终态 | Lobby SIGTERM | `stopped`（graceful shutdown_all） | `instance/manager.rs:362` |

**不变量**：

- 一条 `game_instances` 行对应一局游戏的完整生命周期（从 spawn 到 finished/abnormal）。
- 同一 `room_id` 可有任意多行；`current_instance_id` 字段（`RoomInfo`）保留最近一次活跃实例的引用。
- 子进程 stdout 必须每 5s 发 `{"event":"heartbeat"}`，否则 15s 后被 watchdog 判 abnormal。

## 5. 玩家生命周期

### 5.1 加入（`POST /api/rooms/:id/join`，`room.rs:181`）

事务内顺序：

1. `SELECT game_type, host_uid, status, variant FROM rooms WHERE room_id = ?`
   - 行不存在 → `ROOM_NOT_FOUND`
   - `status != "Waiting"` → `ROOM_NOT_WAITING`
2. `SELECT COUNT(*) FROM room_players WHERE room_id = ?`
   - `count >= max_p` → `ROOM_FULL`
3. `SELECT uid FROM room_players WHERE room_id = ? AND uid = ?`
   - 已存在 → `ALREADY_IN_ROOM`
4. 计算 `seat`：`seat = (0..max_p).find(|i| !taken.contains(i)).unwrap_or(0)`
5. `INSERT INTO room_players (room_id, uid, seat)`

> **设计**：join 只在 `Waiting` 接受。多局游戏中途 `Running`/`Finished` 不允许新人加入 —— 这一约束使棋盘上始终是最初入局的玩家，避免游戏重新开局时被踢出。如果未来要支持"观战 / 临时加入"，需要新增 `phase` 与权限模型，本周期不做。

### 5.2 离开（`POST /api/rooms/:id/leave`，`room.rs:262`）

```
SELECT host_uid FROM rooms WHERE room_id = ?      -- 取旧 host

DELETE FROM room_players WHERE room_id = ? AND uid = ?
  affected == 0 → NOT_IN_ROOM

SELECT uid FROM room_players WHERE room_id = ?
  ORDER BY joined_at ASC, seat ASC LIMIT 1        -- 最早加入的存活者

IF 空:
  UPDATE rooms SET status='Destroyed' WHERE room_id = ?
ELSE IF 离开的是 host:
  UPDATE rooms SET host_uid = new_host
  log "host promoted"
```

> **设计**：房间在 `Running`/`Finished` 期间也允许 leave —— 玩家可中途退出（违反规则但服务端不阻止，房间仍可结束）。最后一人 leave 才进 `Destroyed`。**不留"空房超时"**。

> **不变量（leave 保证）**：任何非 `Destroyed` 房间的 `host_uid` 始终指向 `room_players` 里一名在房玩家。

## 6. host 禅让

| 场景 | V1 行为 | 下一周期 |
|---|---|---|
| host 显式 leave | 提早加入的存活者为新 host | 同 V1 |
| host 关浏览器/网络断 | **不禅让**，等 host 显式 leave（或 V1 通过下次 leave 触发） | 检测 host WS 断开 → 自动禅让 |
| host session 过期 | 同上 | 同上 |

> **原则（意见 3）**：host 在线状态检测走 WS 心跳，**不另开 timeout**。下一周期 lobby 加"WS 代理侧"的连接追踪（目前 WS 代理是纯转发，不感知用户），用"host WS 断开后 N 秒仍无 reconnect → 禅让"取代 idle timeout。

## 7. Start → Spawn → Run → Finish 全流程

```
host POST /api/rooms/:id/start
  │
  ├─ host_uid == caller.uid 否则 NOT_HOST
  ├─ status ∈ {Waiting, Finished} 否则 ROOM_NOT_WAITING
  ├─ entry.binary 预检：绝对路径 is_file() + 可执行；裸名走 $PATH
  │     失败 → 503 GAME_BINARY_NOT_FOUND
  ├─ SELECT rp.uid, s.token
  │   FROM room_players rp JOIN sessions s
  │   WHERE rp.room_id = ? AND s.expires_at >= datetime('now')
  │   ORDER BY rp.uid, s.created_at
  │     按 uid 聚合 → Vec<PlayerInit>{ uid, sessions: [..] }
  ├─ UPDATE rooms SET status='Starting'    -- 进临界区
  │
  └─ InstanceManager::spawn() {
        allocate_port() → 127.0.0.1:<rand>
        Command::new(bin).spawn()
        INSERT INTO game_instances (room_id, pid, port, status='starting')
        stdin 首行写 LobbyInit JSON
        启后台 task A: 转发 stdin (cmd:start/stop)
        启后台 task B: stderr → tracing
        启后台 task C: stdout → GameEvent 解析
        启 watchdog: 10s 没 ready → 杀进程, rooms='Waiting'
        插入 ActiveInstance{ port, status=Starting, ... }
     }
     返 instance_id

HTTP 返 { instance_id, ws_url: "ws://<public>:<port>/ws/<id>" }
房间仍 Waiting；进入下一阶段靠 stdout 事件推进

[c2] Game stdout: {"event":"ready","port":...}
        instance.status=Ready
        game_instances.status='ready'
        rooms.status='Running'                              ← 房间正式进入 Running
        写 cmd:start 到 game stdin

[c3] Game stdout: {"event":"running"}                       (可选)
        instance.status=Running
        game_instances.status='running'

[c4] 客户端 GET /ws/<instance_id>
        ws_handler: instances.lookup(id) → port
        ws.on_upgrade → bridge(socket, id, port)
          ↔ 127.0.0.1:<port> WS 透传，纯字节，不解析

[c5] 客户端发 {"type":"login","uid","session"}
        game-sdk 调 L::validate_session(uid, session)
        命中任一 token → 发 {login_ok, snapshot}
        不命中 → INVALID_SESSION + 关连接

[c6] 客户端发 {"type":"game","data":{...}}
        game-sdk 调 L::handle_action → Ok/Reject/GameOver
        Ok/Reject → broadcast_snapshot 或 game_error
        GameOver → broadcast_snapshot(phase=finished) → drop tx/cleanup/await pump → 关连接

[c7] Game stdout: {"event":"finished"}
        instance.status=Finished
        game_instances.status='finished', end_time=now()
        rooms.status='Finished'                              ← 房间回到 Finished
        SIGKILL 子进程；从内存 map 移除 ActiveInstance

WS 代理桥 TCP 半边关，自动退出
客户端 gameWs.onclose 触发 → 弹出结算画面（modal）→ "返回房间" 跳 #room/<id>
```

> **设计要点**：客户端 `gameWs` 断开后，**前端**负责跳回 `#room/<id>`（基于 `state.currentRoomId`）。房间本身仍在 `Finished`，等 host 再点 "再来一局"。

## 8. 失败与恢复

| 故障 | 检测 | 恢复 |
|---|---|---|
| `entry.binary` 不存在 / 无 exec | start handler 预检 | `503 GAME_BINARY_NOT_FOUND`，rooms 不变 |
| spawn 子进程失败（port 抢不到） | start handler | `500 INSTANCE_START_FAILED`，rooms=`Waiting` 回滚 |
| spawn 后 10s 内未 `ready` | watchdog | `500 INSTANCE_START_FAILED`，rooms=`Waiting` 回滚 |
| 子进程 stdout EOF | stdout 解析 task | `instance=Abnormal`，rooms=`Waiting` 回滚 |
| 子进程心跳 > 15s | watchdog | 同上 |
| 房主离开 | leave handler | 提早加入者为新 host（**不依赖房主在线**） |
| **房主离线（关浏览器）** | — | V1：不处理，等显式 leave；下周期：WS 心跳 |
| Lobby SIGTERM | main | `shutdown_all()`：发 `cmd:stop`，5s grace，force-kill；DB 状态保留 |

## 9. 已知弱点与下一周期

| # | 弱点 | 计划 |
|---|---|---|
| W1 | 重启 lobby 后 DB 里 `starting`/`ready` 的 `game_instances` 行变孤儿 | 启动时扫描并标 `abnormal`，或加 lazy GC |
| W2 | `RoomInfo.current_instance_id` 在 instance finished 后还指过去那条记录 | `finished` 路径清零（或前端看 `phase` 而非 `current_instance_id`） |
| W3 | 无 WS 侧用户在线追踪（WS 代理纯转发） | 在 `bridge` 里维护 `(uid → instance_id)` 集合；断开时回调 lobby |
| W4 | host 离线不能自动禅让 | W3 实现后用"WS 断开 + N 秒无 reconnect → 禅让"实现 |
| W5 | `Running`/`Finished` 期间外人不能加入 | 下一周期加 `observers` / `late_join` 角色（需要先设计 phase 权限模型） |
| W6 | `Finished` 状态会无限期存在（房主不重启也不走） | 与 W4 协同：如果 host 离线后整个房间只剩 N-1 个心跳全断的玩家 → 禅让失败 → 房间继续；W3 上线后可判"全房无活跃 WS"→ Destroyed |
| W7 | session 过期但 player_sessions 仍含其 token | spawn 时已经按 `expires_at >= now` 过滤；无需修复 |

## 10. 关键不变量（code review checklist）

- `rooms.host_uid ∈ room_players.uid`（leave handler 保证）
- `min_players ≤ room_players.uid 数量 ≤ max_players`（join handler 保证）
- 一局游戏中 `PlayerInit.sessions[i]` ⊆ 该 uid 当前未过期 session（spawn 时过滤）
- 房间 `Finished` 至少存在一名玩家（否则本应 `Destroyed`）
- 同一 `room_id` 的 `current_instance_id` 一定指 `game_instances` 里 status `≠ {stopped}` 的最新一条（待 W2 修）
- 子进程 `cmd:stop` 后 5s 仍未退出 → SIGKILL（graceful shutdown）