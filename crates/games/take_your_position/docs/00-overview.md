# Take Your Position (TYP) — 文档索引

> 5 人预测出牌游戏。本目录是 TYP 的**完整规约**——游戏规则、架构、状态机、UI 接入、计时、测试都在这里。
>
> 整个项目的总览见 `/docs/TYP_HANDOVER.md`（架构 + 部署 + 9 条踩过的坑）。本目录专注于 TYP 游戏本身。

---

## 1. 一句话

5 人 × 5 轮的卡牌游戏。每轮：**先猜名次 → 5 人同时出牌 → 首位玩家补一轮排名 → 结算**。按真实名次 + 先验/后验预测的命中度 + 一次"全中"或"全空"的特殊裁定算分。

---

## 2. 文件地图

| 文档 | 解决的问题 |
|---|---|
| `01-rules.md` | 玩家怎么玩：5 轮流程、计分细则、A+B 双池计时 |
| `02-architecture.md` | crate 内部结构、SDK 接口、关键函数 |
| `03-state-machine.md` | 阶段机 + snapshot schema + action schema + 事件（前端看的） |
| `04-ui-integration.md` | 前端怎么用 snapshot 数据（座位布局、后验 flash 3 秒、轮结算弹窗） |
| `05-timing.md` | A+B 双池：玩家几时开始计时、settle 怎么扣、上次 bug 修复历史 |
| `06-testing.md` | 单元测试怎么跑 + 端到端怎么测 |
| `07-changelog.md` | 关键变更历史（计时 bug / 3 秒揭示 / 字典协议 等） |

---

## 3. 源码地图

`crates/games/take_your_position/src/`：

| 文件 | 行数 | 职责 |
|---|---|---|
| `card.rs` | 174 | `Suit`/`Rank`/`Card`，大小比较（A/K 特殊规则）|
| `command.rs` | 17 | `Action` enum（前端发往游戏进程的 4 种动作） |
| `event.rs` | 40 | `Event` enum（进 `pending_events` 的事件类型） |
| `state.rs` | 489 | `Phase` / `PlayerState` / `GameState` / `StepTimers` |
| `rules.rs` | 425 | `apply_predict` / `apply_play_card` / `apply_posterior` / `apply_posterior_draft` / `finish_round` 计分 |
| `logic.rs` | 533 | `GameLogic` 实现：`snapshot` / `handle_action` / `advance_phase` / `tick` / `on_player_login` |
| `main.rs` | 31 | 读 stdin `LobbyInit` → `game_sdk::run` |

`src/main.rs` 没有任何游戏逻辑——所有规则都在 `logic.rs` 和 `rules.rs`。

---

## 4. 双向链接

- 上游：`/docs/games_architecture.md`（V1.1 多游戏架构，本游戏是其中之一）
- 上游：`/docs/protocol_spec.md`（与 Lobby 的 WS / stdin / stdout 协议）
- 上游：`/docs/game_log_protocol.md`（`game_sdk::game_log!` 的 JSON Lines 协议）
- 上游：`/docs/frontend-design.md`（前端设计语言，本游戏 UI 遵循）
- 下游：前端 `crates/lobby/static/app.js` 的 `renderCardBoard` / `buildPosteriorRankRow` / `showRoundSummary` 是 TYP 的镜像
