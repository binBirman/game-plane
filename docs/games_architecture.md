# Games Architecture (V1.1 Plan)

Version: 0.1
Status: Planned, not yet implemented

本文件描述 V1.1 阶段的多游戏 / DLC 架构。后续 V1.1 完成后，本文件中的"计划"项会改为"已实现"。

## 1. 目标

- 同一套运行时支持多款游戏：井字棋（V1 已实现）+ 两款待开发游戏（一款 5 人、一款 5-8 人，规则待用户提供）。
- 现有游戏可通过 DLC 扩展（具体形态由各游戏规则决定），**不污染** SDK 或 Lobby。
- Lobby 按配置选择并 spawn 正确的 game binary。
- 前端提供游戏目录，让用户在创建房间前选游戏。

> 本文件是架构骨架，**不预设任何具体游戏机制**。各游戏的玩法、阶段、动作、胜负条件由各自游戏文档定义。

## 2. Workspace 布局

替换 V1 的单 `crates/game/`：

```
crates/
├── game-sdk/             # 新：通信骨架，无游戏逻辑
│   └── src/lib.rs
├── games/
│   ├── tictactoe/        # 迁移自 crates/game/
│   │   └── src/main.rs
│   ├── <game-A>/         # 待实现（用户给规则）
│   │   └── src/main.rs
│   └── <game-B>/         # 待实现（用户给规则）
│       └── src/main.rs
└── lobby/                # 不变
```

`game-sdk` 是**库**，被 `games/*` 二进制引用。每个游戏 crate 独立可执行。

## 3. SDK 设计（`game-sdk`）

### 3.1 `GameLogic` trait

```rust
#[async_trait::async_trait]
pub trait GameLogic: Send + 'static {
    type Config: Default + Serialize + DeserializeOwned + Clone + Send;

    /// 用玩家列表 + 配置构造
    fn new(players: &[PlayerInit], config: &Self::Config) -> Self;

    /// 某玩家的快照视图（None = 全公开，旁观者/结算用）
    fn snapshot(&self, viewer: Option<i64>) -> serde_json::Value;

    /// 处理一个 action
    fn handle_action(&mut self, uid: i64, action: serde_json::Value) -> ActionOutcome;

    /// 全局结果（写 DB、UI 用）
    fn result(&self) -> serde_json::Value;

    fn is_over(&self) -> bool;
    fn phase(&self) -> PhaseInfo;

    fn min_players(&self) -> usize;
    fn max_players(&self) -> usize;
    fn game_name(&self) -> &'static str;
}

pub struct PhaseInfo {
    pub name: String,                  // "pre-flop" / "day_vote" / "night"
    pub active_player: Option<i64>,    // 谁回合
    pub awaiting: Vec<i64>,            // 等待行动的玩家
    pub time_limit_ms: Option<u64>,    // 可选超时
}

pub enum ActionOutcome {
    Ok,
    Reject(String),
    GameOver,
}
```

### 3.2 SDK 负责的固定部分（`run()`）

- 读 stdin 首行 `LobbyInit`（含新 `config` 字段）
- 绑定 WS 监听（`listen`，固定 `127.0.0.1`）
- 发 `ready` 事件
- 周期发 `heartbeat`（5s）
- 接受 WS 连接；首帧必须是 `login` 或 `reconnect`
- 校验 session 对照 `LobbyInit.players[*].session`
- 路由 `ping → pong`、`game → handle_action`
- 处理 stdin `cmd:start`、`cmd:stop`
- 阶段切换、发 `finished` 等事件给 Lobby

### 3.3 游戏负责的部分

- 实现 `GameLogic` trait
- 声明自己的 `Config` 类型（具体形态由游戏决定）
- 游戏规则、阶段切换、胜负判定（具体规则由用户提供）
- **每个玩家的视角可能不同**（手牌/身份/私有信息），由游戏在 `snapshot(viewer)` 中决定

## 4. `LobbyInit` 扩展

`protocol::LobbyInit` 增加可选字段：

```rust
pub struct LobbyInit {
    pub room_id: i64,
    pub game_type: String,
    pub listen: String,
    pub players: Vec<PlayerInit>,
    #[serde(default)]
    pub config: Option<serde_json::Value>,  // NEW
}
```

Lobby 在 spawn game 时把配置塞进 init：

```json
{
  "room_id": 42,
  "game_type": "<game-A-type>",
  "listen": "127.0.0.1:41001",
  "players": [{"uid":1,"session":"..."}],
  "config": { /* 由游戏决定；Lobby 透传 */ }
}
```

Game 把 `config` 反序列化为自己的 `Config` 类型。

## 5. Lobby 注册表

配置文件 `/etc/lobby/games.toml`（也支持 env `LOBBY_GAMES_TOML` 指向其他路径）：

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

[[games]]
type = "<game-A-type>"
name = "<game-A 名称>"
description = "..."
binary = "/usr/local/bin/<game-A>"
min_players = 5
max_players = 5
enabled = true
variants = [
  # 等用户给规则后填
]

[[games]]
type = "<game-B-type>"
name = "<game-B 名称>"
description = "..."
binary = "/usr/local/bin/<game-B>"
min_players = 5
max_players = 8
enabled = true
variants = [
  # 等用户给规则后填
]
```

Lobby 启动时加载到内存注册表。`POST /api/rooms` 校验 `game_type` 在表内，`start` 时按表查 binary 路径 spawn。

## 6. 新增 API

### 6.1 `GET /api/games`

返回注册表中 `enabled=true` 的游戏：

```json
{
  "games": [
    {
      "type": "tictactoe",
      "name": "井字棋",
      "description": "三连一线",
      "min_players": 2,
      "max_players": 2,
      "variants": []
    },
    {
      "type": "<game-A-type>",
      "name": "<game-A 名称>",
      "description": "...",
      "min_players": 5,
      "max_players": 5,
      "variants": []
    },
    {
      "type": "<game-B-type>",
      "name": "<game-B 名称>",
      "description": "...",
      "min_players": 5,
      "max_players": 8,
      "variants": []
    }
  ]
}
```

### 6.2 `POST /api/rooms` 扩展

```json
{"game_type":"<game-A-type>","variant":"<a>"}
{"game_type":"<game-B-type>","variant":"<b>"}
{"game_type":"tictactoe"}                       // 无 variant 走默认
```

校验：
- `game_type` 在注册表且 `enabled=true`
- `min_players ≤ 当前人数 ≤ max_players`
- `variant`（若指定）在游戏的 `variants` 列表中

### 6.3 DB 扩展

`rooms` 表加 `variant TEXT NULL`（或整个 `config TEXT` JSON 列）。`start` 时把 `variant` 与 `config` 一并写入 init。

## 7. 游戏特定 Config：约定

> **本节不预设具体游戏规则。** 用户后续提供每款游戏的玩法文档，游戏 crate 按各自规则实现 `GameLogic` 并定义自己的 `Config` 类型。SDK 与 Lobby 对 Config 内容**一无所知**，只做透传（JSON in → 游戏反序列化 → JSON out）。

`Config` trait 约束（SDK 侧）：

```rust
type Config: Default + Serialize + DeserializeOwned + Clone + Send;
```

每款游戏的 `Config` 形态由该游戏 crate 自己决定，可能包括但不限于：

- variant 枚举（单选 DLC）
- 数据驱动的角色/牌型/事件表
- 由 Lobby 从 `POST /api/rooms` 的 `config` 字段透传

Lobby 透传路径：

```
POST /api/rooms {game_type, variant?, config?}
  → 校验 game_type 与 variant
  → 写入 rooms.variant, rooms.config（JSON 列）
  → start 时 lobby 把 rooms.config 合并进 LobbyInit.config → spawn 游戏 binary → 游戏反序列化
```

## 8. DLC 兼容路径（通用）

| DLC 类型 | 做法 | 影响面 |
|---|---|---|
| 现有游戏新增玩法选项 | 在该游戏 `Config` 类型加分支或数据 | 仅该游戏 crate 重编 |
| 现有游戏新增可选规则集合 | 编辑 `rooms.config` JSON（无需改代码） | 无需重编 |
| 全新游戏 | 新建 `games/<type>` crate + 在 `games.toml` 注册 | Lobby / SDK / 其他游戏不动 |

SDK / Lobby **不变**。游戏之间不互相影响。

## 9. 实施阶段

| # | 内容 | 状态 |
|---|---|---|
| 1 | `game-sdk` 骨架 + `run()` + `GameLogic` trait | ✓ 已完成 |
| 2 | tictactoe 迁移到 SDK（行为不变） | ✓ 已完成 |
| 3 | `games/<game-A>` 用户给规则后实现 | 等规则 |
| 4 | `games/<game-B>` 用户给规则后实现 | 等规则 |
| 5 | Lobby `games.toml` 注册表 + `GET /api/games` + 前端多游戏卡片 | 待做 |
| 6 | 单元测试 + protocol_spec 增订 + test.sh 加新游戏烟测 | 部分（room+WS 烟测已加在 test.sh） |

**已完成**：
- `crates/game-sdk/` 库：`GameLogic` trait + `run()` + WS/session/heartbeat 全套通信骨架
- `crates/games/tictactoe/`：仅含游戏规则 + `GameLogic` 实现，binary 名 `tictactoe`
- `protocol::LobbyInit` 加 `config: Option<Value>` 字段
- `tools/ws_client.py`：Python 实现的 RFC 6455 WS 客户端（无外部依赖）
- `tools/test.sh` 加 8 个新用例：注册 B、login B、create room、join、start、WS roundtrip、leave x2

**未实现**：
- games.toml 注册表（lobby 仍用单一 `LOBBY_GAME_BIN` 环境变量，默认指向 `tictactoe`）
- `GET /api/games` 端点
- `POST /api/rooms` 接受 `variant` / `config` 字段
- 前端多游戏卡片

**最小可用闭环** = 阶段 1+2+5（SDK + tictactoe 验证 + 注册表/UI）。扩展点已就绪：新增游戏只需 `crates/games/<name>` + 在 `games.toml` 注册。

## 10. 下次实现清单

按 §9 阶段顺序：

1. 新建 `crates/game-sdk/`，定义 `GameLogic` trait + `PhaseInfo` + `ActionOutcome` + `run()` 入口。
2. 把 `crates/game/` 整体迁移到 `crates/games/tictactoe/`，重写为只实现 `GameLogic`。
3. 更新根 `Cargo.toml` workspace members。
4. `protocol::LobbyInit` 加 `config: Option<Value>` 字段。
5. Lobby：加载 `games.toml` 到内存注册表（`Arc<GameRegistry>`），写入 `AppState`。
6. Lobby：新增 `GET /api/games` 路由。
7. Lobby：扩展 `POST /api/rooms` 接受 `variant`，`rooms` 表加 `variant` 列，`start` 时按注册表 spawn 对应 binary 并把 `config` 塞进 init。
8. 前端 Lobby 页改成多游戏卡片网格 + variant 选择器；Room 页增加游戏名/variant/人数显示。
9. **等用户给规则** → 实现 `games/<game-A>` 完整逻辑。
10. **等用户给规则** → 实现 `games/<game-B>` 完整逻辑。
11. (阶段 6) 单元测试覆盖新游戏 `Config` 分支；`tools/test.sh` 加新游戏烟测；`docs/protocol_spec.md` 把"计划"项标为"已实现"。

## 11. 不做什么

- 不做匹配 / 撮合系统（spec 出范围）
- 不做好友、邀请、分享链接（spec 出范围）
- 不做游戏回放 / 录像（spec 出范围）
- 不做插件动态加载（cargo 不友好，超出 V1.1 范围）—— 新游戏通过加 crate 完成
- 不做 AI 对手