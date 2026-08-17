# Take Your Position — 项目交接文档

> 接手本项目的快速通道：架构、游戏规则、开发注意、本地测试、部署、代码地图、常见坑。
> 状态：V1 已上线，多游戏架构（V1.1）已就绪。

---

## 1. 项目概览

**game-plane** 是一个 Rust workspace 的多人游戏平台：

```
Client ── HTTP/WS ──► Lobby Server ──(spawn 子进程)──► Game Process
                        │                                 │
                        │   auth / rooms / 生命周期          │  游戏规则 / 状态 / 回合
                        └── WS 反向代理 (Lobby 单端口) ─────┘
```

- **Lobby**（`crates/lobby`）：注册/登录/房间 CRUD/游戏进程生命周期/动态端口/WS 反向代理/心跳 watch/僵尸清理。
- **Game**（`crates/games/<name>`）：游戏规则、状态、回合、重连。
- **game-sdk**（`crates/game-sdk`）：游戏与 Lobby 之间的通信骨架（stdin/stdout 线协议、WS 服务、session 校验、心跳、`GameLogic` trait）。
- **protocol**（`crates/protocol`）：跨进程/跨网络消息类型。

已上线的游戏：
| 游戏 | crate | type | 玩家数 |
|---|---|---|---|
| 井字棋 | `crates/games/tictactoe` | `tictactoe` | 2 |
| **Take Your Position** | `crates/games/take_your_position` | `take_your_position` | 5 |

权威规范（先看这几个）：
- `docs/architecture.md` — 整体架构、职责边界
- `docs/protocol_spec.md` — 状态机、接口、错误码（改协议先改这里）
- `docs/games_architecture.md` — 多游戏架构、新游戏接入步骤
- `docs/design.md` — 设计概览、数据库表
- `docs/frontend-design.md` — 前端约定（无 emoji、零资源渲染）

---

## 2. Take Your Position 游戏规则

### 2.1 流程

5 人 × 5 轮。每轮：
```
PriorPrediction (先验预测) → Play (同时出牌) → PosteriorPrediction (后验) → 结算
```

- **WaitingAll**：开局前等待全部 5 个玩家 WS 连接成功（`on_player_login` 逐个加入），齐了才进 `PriorPrediction`。**计时在此之前不启动**。
- **先验预测**：依次（逆时针）每位玩家选 1–5 名次或「放弃」，需点「确认」提交，提交后锁定不可改。
- **出牌**：所有玩家**同时**出牌，点牌选中 + 点「确认」提交；全部提交后才揭示。出牌期间牌显示在**自己角色卡旁**（自己正面、他人背面）。
- **后验预测**：仅首位玩家（`start_player`）提交完整 5 人排名或跳过。编辑时实时同步（`draft_posterior`），点「确认」/「跳过」才正式提交。
- 结算后 `start_player` 逆时针轮转，进入下一轮。5 轮后 `End`。

### 2.2 计分

| 分项 | 规则 |
|---|---|
| 排序分 | 按真实名次 +2/+1/0/-1/-2 |
| 先验预测分 | 每人预测自己名次：猜对 +2，猜错 -2，跳过 0 |
| 后验预测分 | 首位玩家按完整排名逐位对比（`accurate` = 猜对位数）：**5→+2, 4→+1, 3→0, 2→-1, 1→-2, 0→-2**（对 4 不可能出现，有 `assert` 兜底） |

总分 = 三者和，累加到 `player.score`。

### 2.3 A+B 双池计时（核心，易混淆）

`timer_preset = "A+B"`（秒）：
- **A（刷新池）**：每次行动后**重置回满**为 A 秒。
- **B（保留池）**：整局不重置，扣多少剩多少。
- **每个玩家独立**两池。
- **行动结算**：该玩家累计思考时间 → 先扣 A，A 归零再扣 B → 行动完 A 回满、B 保留。
- **超时**：A+B 都归零 → 自动代理（预测=放弃，出牌=第一张，后验=跳过）。
- **0 = 该池不限时**（如 `300+0` = 预测 5 分钟、出牌不限时）。

预设档（前端可选）：`30+60` / `40+120` / `60+180` / `300+0`。任意 `N+M`（可含 0）后端都接受。

**UI 显示**：玩家角色卡旁显示 `A秒 + B秒`；A 白色、`+` 和 B 橙色；某池为 0 时隐藏该池并去掉加号。前端 1s 本地递减（先 A 后 B）。

---

## 3. 架构与职责边界（红线）

- **Lobby 不解析 game 信封**：WS 反向代理是纯字节透传，`snapshot`/`game` 内容由前端解析。
- **Game 不存密码、不依赖 JWT**：Lobby 发 session，Game 用 `LobbyInit.players[*].sessions` 校验（任一匹配即可）。
- **Room ≠ 一局游戏**：Room 可经历多轮 `Waiting→Starting→Running→Finished→Starting…`，GameInstance 每局一个。

---

## 4. 开发注意事项（踩过的坑，务必知道）

### 4.1 schema 迁移顺序
`schema.sql` 用 `CREATE TABLE IF NOT EXISTS`；新列靠 `migrations.rs` 的 `ALTER TABLE` 补。**索引若引用了新列，必须放在 ALTER 之后创建**——否则旧库上 `CREATE INDEX IF NOT EXISTS ... ON room_players(online)` 会因列不存在而 `no such column`，整个 lobby 启动失败。
- 新列：`rooms.timer_preset`、`rooms.last_active_at`、`room_players.online`、`game_instances.last_action_at`。
- 对应索引在 `migrations.rs` 末尾（ALTER 之后）创建。

### 4.2 deadline 起点（曾导致"一进去全放弃"）
计时**不能从游戏进程 spawn 就开始**——玩家还在登录/连接。现在用 `WaitingAll` 阶段 + `start_thinking()`：全部玩家认证后才开始计 A+B。

### 4.3 session 同步有 5s 窗口
游戏启动后新登录的 token 由 Lobby 每 5s 推给运行中的 game（stdin `add_session`）。玩家重新登录后**等 ~5s 再进**，否则 `INVALID_SESSION`。

### 4.4 `pending_events` 累积会卡
每轮结算后 `begin_next_round` 会 `pending_events.clear()`。不要往该 vec 无限 append，否则每次 snapshot 都重放大量事件、前端日志膨胀。

### 4.5 前端日志别用 `textContent +=`
`gameLog` 已改为每条一个 `.log-entry` div + 上限 300 条，避免单文本节点无限增长导致卡顿。别改回字符串拼接。

### 4.6 玩家断开感知
前端/测试**必须发 WS close frame（opcode 0x8）**，服务端 `receiver.next()` 才会返回 None 触发 `cleanup`。Python `socket.close()` 只发 TCP FIN，服务端不感知。前端浏览器正常关页会发 close frame，无需担心。

### 4.7 僵尸房间清理
`cleanup.rs` 每 10s：
- `Running/Starting` 房间：`COALESCE(last_action_at, start_time)` 距今超 300s 且无玩家动作 → 销毁（**新游戏有 300s 宽限**，用 COALESCE 而非直接判 last_action_at，否则刚开的房间会被误杀）。
- `Waiting` 房间：`last_active_at`（房间页心跳，每次 GET /api/rooms/:id 更新）超 300s → 销毁。
- 可配 `LOBBY_CLEANUP_*` 环境变量。

### 4.8 对局结束离线玩家自动退房
game 结束发 `{"event":"finished","result":{"online":[...]}}`，Lobby 把不在 `online` 里的玩家从 `room_players` 删除。

### 4.9 前端 CSS 命名冲突
前端用了 `.card`（房间面板）和 `.play-card`（纸牌）两套。**新增纸牌样式必须用 `.play-*` 前缀**，否则覆盖房间面板布局。

---

## 5. 本地测试

### 5.1 快速冒烟（本机，需 sudo 因为写到 /var/lib）

```bash
cd ~/dev/game-plane-main

# 编译全部
cargo check --workspace
cargo build --release --target x86_64-unknown-linux-musl \
    -p lobby -p tictactoe -p take_your_position

# 覆盖安装 + 重启（lobby 内嵌前端，改 JS/CSS 必须重编 lobby）
sudo install -m 755 target/x86_64-unknown-linux-musl/release/{lobby,tictactoe,take_your_position} /usr/local/bin/
sudo systemctl restart lobby
curl -s http://127.0.0.1:8192/api/games | python3 -m json.tool
# 应看到 tictactoe + take_your_position
```

### 5.2 跑官方烟测（25 用例）

```bash
PORT=8192 bash tools/test.sh
```

### 5.3 端到端 WS 测试（TYP 完整流程）

> **注意**：本机 shell 有 `http_proxy`，Python 必须 `os.environ["no_proxy"]="*"`，curl 用 `--noproxy '*'`。

测试要点（脚本参考 `/tmp/typ-*.py`，重写思路）：
1. 注册 5 个用户 + 登录（初始 token）
2. 创建房间 `POST /api/rooms {"game_type":"take_your_position","timer_preset":"30+60"}`
3. 4 人 join，host start
4. **重新登录 5 人拿新 token，等 6s**（session sync）
5. 5 个 WS 连 `/ws/<instance_id>`，发 `login`，收 snapshot（应 `phase=waiting_all`）
6. 全部 login 后 → `prior_prediction`，`times` 字段带 A/B 池
7. 依次 predict（当前玩家）→ play（全选+确认）→ posterior（首位玩家）
8. 验证 `online`、`times`、`posterior_draft`、RoundResult 计分

**WS 客户端**：`tools/ws_client.py`（Python，无依赖）。

### 5.4 只测 game 逻辑（不连 Lobby）

game-sdk 是独立的，但 spawn 需要 LobbyInit。最简单还是走 Lobby spawn。要纯逻辑测试可给 `GameState` 写单测（目前无，建议后续补 `#[cfg(test)]`）。

---

## 6. 部署说明

### 6.1 打产包

```bash
cd ~/dev/game-plane-main
bash build.sh                 # 默认版本取自 Cargo.toml (workspace.package.version)
# 产出 dist/lobby-<version>.tar.gz（musl 静态，含 3 个二进制 + games.toml + install.sh + upgrade.sh + DEPLOY.md + static）
md5sum dist/lobby-0.4.0.tar.gz
```

### 6.2 上传 + 安装（目标服务器）

```bash
scp dist/lobby-0.4.0.tar.gz root@SERVER:/opt/
ssh root@SERVER
cd /opt
md5sum lobby-0.4.0.tar.gz      # 核对（传错=旧包）
tar xzf lobby-0.4.0.tar.gz
cd lobby-0.4.0
./install.sh --force-games-toml   # 必须 --force，否则旧 games.toml 不覆盖
```

### 6.3 升级（推荐，不用手动删）

包内含 `upgrade.sh`，一键完成：**停服务 → 杀孤儿 game 进程 → 换二进制 → 强制覆盖 games.toml → 重启 → 校验**，并**保留 DB 与 env**：

```bash
cd /opt/lobby-0.4.0
sudo ./upgrade.sh
```

对比手动流程，`upgrade.sh` 省掉：`systemctl stop`、`pkill -f take_your_position`、`rm -rf 旧目录`、`md5sum` 核对（脚本自带校验 + 打印新 md5）。`install.sh` 也已内置孤儿进程清理。

### 6.3 配置 `/etc/lobby/lobby.env`

```bash
# 确保这行是取消注释的（默认是注释，lobby 会 fallback 到单 tictactoe）
LOBBY_GAMES_TOML=/etc/lobby/games.toml
```

```bash
systemctl start lobby
systemctl is-active lobby
md5sum /usr/local/bin/lobby    # 与本地 build 一致
curl -s http://127.0.0.1:8192/api/games   # 两个游戏
```

### 6.4 判断装的是不是新包

`install.sh` 输出：
- **旧包**：只有 `installed /usr/local/bin/tictactoe`
- **新包**：`installed /usr/local/bin/tictactoe` + `installed /usr/local/bin/take_your_position`

### 6.5 环境变量一览

| 变量 | 默认 | 说明 |
|---|---|---|
| `LOBBY_BIND` | `0.0.0.0:8192` | 监听地址 |
| `LOBBY_GAMES_TOML` | 无 | 多游戏注册表；**必须设**否则单 tictactoe |
| `LOBBY_PUBLIC_HOST/PORT` | bind | `ws_url` 用；反代时设对外域名 |
| `LOBBY_POW_DIFFICULTY` | 16 | PoW 难度 |
| `LOBBY_CLEANUP_ACTION_SECS` | 300 | Running 无动作清理阈值 |
| `LOBBY_CLEANUP_WAITING_SECS` | 300 | Waiting 无人看清理阈值 |
| `LOBBY_CLEANUP_INTERVAL_SECS` | 10 | 清理扫描间隔 |
| `RUST_LOG` | `info,lobby::http=debug,...` | 日志级别 |

### 6.6 已知服务器

| 机器 | IP | 说明 |
|---|---|---|
| 开发/本机 | `192.168.69.129` | 代码 + 本地 lobby（systemd） |
| 生产 | `8.148.5.15` | 部署目标 |

---

## 7. 代码地图

### 游戏 crate `crates/games/take_your_position/`

| 文件 | 职责 |
|---|---|
| `card.rs` | Suit/Rank/Card + 大小比较（A/K 特殊规则） |
| `command.rs` | action 枚举（`Action`，前端同构） |
| `event.rs` | 事件枚举（`Event`，进 `pending_events`） |
| `state.rs` | `PlayerState` / `GameState` / `Phase` / `StepTimers`；A+B 池、thinking、结算 |
| `rules.rs` | `apply_predict/play_card/posterior/draft`、`finish_round` 计分 |
| `logic.rs` | `GameLogic` 实现：snapshot / handle_action / advance_phase / tick / on_player_login |
| `main.rs` | 读 stdin `LobbyInit` → `game_sdk::run` |

### SDK `crates/game-sdk/src/lib.rs`

- `GameLogic` trait（`new/snapshot/handle_action/is_over/phase/validate_session/tick/on_player_login`）
- `run()`：读 stdin init → WS 服务 → 心跳(5s) → tick(1s) → stdin 命令(start/stop/add_session)
- snapshot 注入 `online`（活跃 WS uid）
- 事件：`ready/running/finished/shutdown/heartbeat/action`；`finished` 带 `result.online`

### Lobby 关键路径

| 文件 | 职责 |
|---|---|
| `src/http/room.rs` | create/join/leave/start/list/games；timer_preset 校验与注入 |
| `src/instance/manager.rs` | spawn/stop/lookup/watchdog/GameEvent 处理（含 Action→last_action_at、Finished→退房） |
| `src/cleanup.rs` | 僵尸房间清理 |
| `src/ws_proxy/handler.rs` | WS 反向代理（纯透传） |
| `src/main.rs` | 启动、后台任务（watchdog/cleanup/session-sync） |

### 前端 `crates/lobby/static/`

| 文件 | 职责 |
|---|---|
| `app.js` | 全部前端逻辑（登录/房间/游戏 UI/WS） |
| `card-render.js` | 零资源纸牌渲染（`.play-card`） |
| `card-preview.html` | 纸牌渲染预览页 |
| `style.css` | 样式（`.play-*` 前缀纸牌） |

---

## 8. 建议下一步

- 给 `GameState` 补 `#[cfg(test)]` 单测（计时、计分、阶段推进目前靠端到端验证）。
- `tools/test.sh` 增加 TYP 用例（当前只覆盖 tictactoe）。
- 前端 `app.js` 的 `renderCardBoard` 每秒重渲染较重，可优化为只刷 `.seat-time` 节点。
- 明确生产 `8.148.5.15` 的自动部署流程（CI 或脚本）。
