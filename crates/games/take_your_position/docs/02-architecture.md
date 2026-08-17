# TYP Crate 架构

> 整个项目分层见 `/docs/architecture.md` 和 `/docs/games_architecture.md`。本文件专注 TYP crate 内部。

---

## 1. crate 边界

TYP crate **只负责游戏规则**。它不：

- 不解析 session token（SDK 做）
- 不存密码，不依赖 JWT（约定）
- 不直接写 Lobby 数据库（SDK/lobby 做）
- 不主动 spawn 兄弟进程

它通过 `game_sdk::GameLogic` trait 与外界对接：

```
                                    ┌────────────────────────────────────┐
                                    │  game_sdk (binary `take_your_position`  │
                                    │  唯一的"出 crate"边界)                │
                                    └────────────┬───────────────────────┘
                                                 │
      ┌──────────────────────────────────────────┼──────────────────────────────────────┐
      │                                          │                                      │
┌─────▼──────┐                          ┌───────▼────────┐                      ┌────────▼────────┐
│  Lobby    │  ←  WS 反向代理   纯字节桥  │  TakeYourPosition│  ←  stdin/stdout  │   前端        │
│  (lobby)  │  (不解析 game 信封)   │  (本 crate)      │  线协议          │  (browser)    │
└────────────┘                          └────────────────┘                      └─────────────────┘
```

SDK 提供的钩子（trait）：

```rust
#[async_trait]
pub trait GameLogic: Send + 'static {
    type Config: Default + Serialize + DeserializeOwned + Clone + Send;

    fn new(players: &[PlayerInit], config: &Self::Config) -> Self;
    fn snapshot(&mut self, viewer: Option<i64>) -> Value;
    fn handle_action(&mut self, uid: i64, action: Value) -> ActionOutcome;
    fn result(&self) -> Value;
    fn is_over(&self) -> bool;
    fn phase(&self) -> PhaseInfo;
    fn min_players(&self) -> usize;
    fn max_players(&self) -> usize;
    fn game_name(&self) -> &'static str;
}
```

TYP 的 `TakeYourPosition` 实现了 `GameLogic<GameLogic = TYP, Config = TakeYourPositionConfig>`。

---

## 2. crate 内部模块

```
src/
├── card.rs        数据结构：Suit / Rank / Card + 大小比较
├── command.rs     客户端→服务端 action 的 Rust 枚举（pure data）
├── event.rs       服务端→前端事件 的 Rust 枚举（pure data）
├── state.rs       状态机：Phase / PlayerState / GameState / StepTimers
├── rules.rs       规则校验：apply_* 函数 + finish_round 计分
├── logic.rs       GameLogic 适配：snapshot / handle_action / advance_phase / tick
└── main.rs        入口：读 stdin → game_sdk::run
```

依赖关系：

```
main.rs
  └─ game_sdk::run
        └─ TakeYourPosition::new / snapshot / handle_action / ...
              ├─ state.rs 的 GameState 操作
              ├─ rules.rs 的 apply_* 校验
              └─ event.rs 的 Event 构造
```

`logic.rs` 是协调层，`state.rs` 是数据，`rules.rs` 是规则。`card.rs` 是纯数据/排序。

---

## 3. 关键函数地图

### 3.1 `state.rs` — 状态机

| 函数 | 行 | 职责 |
|---|---|---|
| `Phase::name()` | 28 | 把 enum 序列化为 wire 字符串（"waiting_all"/"prior_prediction"/"play"/"posterior_prediction"/"ended"）|
| `StepTimers::from_preset(preset)` | 83 | 解析 "A+B" 字符串（毫秒）|
| `GameState::new(players)` | 131 | 默认 phase=WaitingAll，start_player=0 |
| `GameState::all_joined()` | 156 | 所有人都进过 WS 的判断 |
| `GameState::apply_timer_config()` | 162 | 把所有玩家 A/B 池填满 |
| `GameState::start_thinking()` | 186 | **每个玩家独立 thinking_since**（修复后版本：PriorPrediction 只 current_player；Play 所有人；PosteriorPrediction 只 start_player）|
| `GameState::thinking_elapsed_ms(seat)` | 222 | 玩家在 thinking_since 之后过了多久 |
| `GameState::remaining_ms(seat)` | 231 | 剩余总时间 = A + B - elapsed |
| `GameState::settle_action(seat)` | 241 | 扣 A / B / refill A |
| `GameState::out_of_time(seat)` | 261 | A+B=0 |
| `GameState::deal()` | 267 | 发 5 张牌：2 小 + 2 大 + 1 ♠ |
| `GameState::next_unacted()` | 284 | 找下一个未行动的玩家 |
| `GameState::begin_next_round()` | 327 | 轮转 start_player、清 per-round 状态、`start_thinking()` |
| `GameState::reveal_plays()` | 377 | 5 张暗牌 → 桌上明牌 + history |
| `GameState::end_game()` | 389 | 生成 `GameEnded` 事件 |

### 3.2 `rules.rs` — 规则

| 函数 | 行 | 职责 |
|---|---|---|
| `apply_predict(uid, rank)` | 53 | 校验先验预测：phase、是否已 commit、rank 范围；写 `prediction` 和 `has_predicted` |
| `apply_play_card(uid, card_index)` | 74 | 校验出牌：phase、是否已 commit、card_index 范围；写 `committed_card` |
| `apply_posterior(uid, rank_list)` | 97 | 校验后验：phase、是否首玩家、是否已 commit、rank_list 完整性（5 个不重复）；写 `posterior_prediction` |
| `apply_posterior_draft(uid, assignments)` | (中间) | 实时编辑：dict `{uid: rank}`，允许部分填充（不全 5 个） |
| `finish_round()` | 198 | 排序 → 计算三项分 → 生成 `RoundResult` 事件 |

### 3.3 `logic.rs` — GameLogic 适配

| 函数 | 行 | 职责 |
|---|---|---|
| `TakeYourPosition::new(players, config)` | 35 | 洗牌 → 构造初始 `GameState`；shuffle seats |
| `TakeYourPosition::snapshot(viewer)` | 100 | **每个玩家可见的 view**（hand 只给 owner；committed_card owner 明面 / 他人背面）|
| `TakeYourPosition::handle_action(uid, action)` | 210 | 解析 `action.action` 字段 → 调用 `rules.rs` 校验 → 调 `state.settle_action` → `advance_phase` → 应用事件 |
| `TakeYourPosition::advance_phase(events)` | 436 | 阶段机：移动 current_player / 触发阶段切换 / 产生 `PhaseChanged` |
| `TakeYourPosition::tick()` | 298 | 每 1s：超时自动代理（predict 跳过 / play_card 0 / posterior 跳过）|
| `TakeYourPosition::on_player_login(uid)` | (中间) | 全部 join 完切到 `PriorPrediction` |
| `TakeYourPosition::is_over()` | (中间) | `phase == End` |
| `TakeYourPosition::phase()` | (中间) | 返回 `PhaseInfo`（SDK 用）|

### 3.4 `card.rs` — 牌

| 类型 | 用途 |
|---|---|
| `enum Suit { Spade=0, Heart=1, Diamond=2, Club=3 }` | wire 编码：序列化用 `as_code` (0-3) |
| `enum Rank { A=0, 2=1, ..., K=12 }` | 同上 |
| `Card::cmp_table(&self, other, table)` | 按 A/K 特殊规则 + rank + 花色排序 |

---

## 4. 与 Lobby 的接口

### 4.1 启动：LobbyInit（stdin 首行）

Lobby 在 `POST /api/rooms/:id/start` 时把 `rooms.timer_preset` 注入 `LobbyInit.config`：

```json
{
  "room_id": 42,
  "game_type": "take_your_position",
  "listen": "127.0.0.1:41023",
  "players": [
    {"uid": 1, "sessions": ["tok1", "tok2"]},
    ...
  ],
  "config": {"timer_preset": "30+60"}
}
```

TYP 读 `init.config` → `serde_json::from_value::<TakeYourPositionConfig>(...)` → `apply_timer_config()`。

### 4.2 心跳 / 周期事件

stdout：
- `{"event":"ready","port":N}` 启动完成
- `{"event":"running"}` 收到 `cmd:start` 后
- `{"event":"heartbeat"}` 每 5s
- `{"event":"finished","result":{"online":[uid,...]}}` 5 轮结束
- `{"event":"action"}` 每次 `handle_action` 完成（lobby 用它更新 `last_action_at`）

stdin：
- `{"event":"start"}` lobby 收到 ready 后转发的
- `{"event":"stop","reason":"..."}` lobby 关闭给游戏
- `{"event":"add_session","uid":N,"session":"..."}` lobby 每 5s 推送新登录的 session

### 4.3 WS 客户端连接

前端 → `/ws/<instance_id>` → lobby 反向代理 → 本游戏进程的 WS 端口。

---

## 5. 与前端的接口

所有状态都通过 `snapshot()` 喂给前端。看 [`03-state-machine.md`](03-state-machine.md) 完整字段。

**关键**：snapshot 是**带 viewer** 的（虽然当前 TYP 实现里 viewer 实际只影响 hand 和 committed_card 的可见性，不影响其他字段）。前端收 snapshot 后从 `state.phase` / `state.current_player` 决定渲染。

---

## 6. 错误处理

`rules.rs` 的 `RuleError` enum：

| 错误 | 触发 |
|---|---|
| `NotYourTurn { expected, got }` | PriorPrediction 不是 current_player 发 predict |
| `WrongPhase { expected, got }` | 阶段不对 |
| `OutOfRange` | rank 越界 / card_index 越界 / 重复 rank / 缺 rank |
| `NotFirstPlayer` | PosteriorPrediction 不是 start_player 发 |
| `AlreadyActed` | 已提交过，再次提交 |
| `Unknown` | uid 不在玩家列表里 |
| `NotEnoughPlayers` | 加入时人数不够 |

`handle_action` 收到 `Err` 时返回 `ActionOutcome::Reject(reason)`，SDK 发 `game_error` 给前端。前端 toast 错误。

---

## 7. 配置文件

`/etc/lobby/games.toml` 一行（由 deploy 写入）：

```toml
[[games]]
type = "take_your_position"
name = "TYP · Take Your Position"
description = "5人预测出牌，每轮预测名次并出牌"
binary = "/usr/local/bin/take_your_position"
min_players = 5
max_players = 5
enabled = true
variants = []
```

**新增玩家变体**（如未来做 3 人版）：在 `variants` 数组加项，配合 `GameState::new` 接受 `Config` 字段。本 crate 代码不用改。

---

## 8. SDK contract 对 TYP 的要求

`game_sdk::GameLogic` trait 约束：

| 约束 | TYP 的实现 |
|---|---|
| `Config: Default + Serialize + DeserializeOwned + Clone + Send` | `TakeYourPositionConfig`（带 `timer_preset: Option<String>`）|
| `handle_action` 返回 `ActionOutcome` | `Ok` / `Reject(reason)` / `GameOver` |
| `snapshot(viewer)` 返回 `serde_json::Value` | per-viewer 字段（hand 只给 owner）|
| `phase()` 返回 `PhaseInfo { name, active_player, awaiting, time_limit_ms }` | 返回 `PhaseInfo`（SDK 用，不进 snapshot）|
| `validate_session(uid, session)` | 因为 `PlayerInit.sessions` 多个 token 都能匹配，TYP 暂不用，SDK 直接 accepts |

不依赖任何 Lobby 内部 API。
