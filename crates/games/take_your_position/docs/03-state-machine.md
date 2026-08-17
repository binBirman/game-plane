# TYP 状态机 + Snapshot Schema

> 这是**前端**视角的权威文档。前端的所有渲染逻辑都基于 snapshot 字段。
> 字段格式由 `TakeYourPosition::snapshot(viewer)` 决定（见 `logic.rs:100`）。

---

## 1. Phase 状态机

```
                  ┌──────────────┐
                  │  WaitingAll  │  5 人 WS 都没 login
                  └──────┬───────┘
                         │  5 个都 login
                         ▼
              ┌──────────────────────┐
              │  PriorPrediction    │  current_player 依次轮到
              └──────┬───────────────┘
                     │  全员 has_predicted
                     ▼
              ┌──────────────────────┐
              │  Play                │  5 人同时 thinking
              └──────┬───────────────┘
                     │  全员 committed_card
                     ▼
              ┌────────────────────────────┐
              │  PosteriorPrediction      │  只 start_player 思考
              └──────┬─────────────────────┘
                     │  start_player 提交
                     ▼
              ┌──────────────────────────────┐
              │  reveal + finish_round          │
              │  round += 1                     │
              │  round == 5 ?  → End : loop     │
              └──────────────────────────────┘
```

5 轮结束 → `End`，前端按 `result` 显示最终排名。

---

## 2. Snapshot Schema（前端看到的 JSON）

**调用方**：`broadcast_snapshot` 在每次 snapshot 刷新时给每个连上的客户端发一次。

**接收方**：前端 `handleGameMessage` 收到 `{"type":"snapshot", "state": {...}}` 后调用 `renderCardBoard(s)`。

```json
{
  "phase": "prior_prediction",        // 5 个值之一
  "round": 0,                          // 0..=4
  "current_player": 1,                 // uid of the active player; null in Play
  "start_player": 1,                   // uid of the first player (rotates each round)
  "players": [4, 1, 5, 2, 3],          // uids in seat order (seat 0, 1, 2, 3, 4)
  "seats": [0, 1, 2, 3, 4],            // redundant index list — same length as players
  "scores": [[4, 8], [1, 6], ...],     // [(uid, score), ...] 按 seat 顺序
  "predictions": [[4, 3, true], [1, null, true], ...],  // [(uid, rank_or_null, has_predicted), ...]
  "committed": [[4, null], [1, {"s":1,"r":2,"hidden":false}], ...],  // Play 阶段；null = 没出牌
  "posterior": [[4, null, false], [1, null, false], ...],  // 第一玩家的 tuple 是 (uid, list, committed)
  "posterior_draft": {"1": 1, "3": 2},  // PosteriorPrediction 编辑过程中的 dict {uid: rank}
  "table": [[4, {"s":1,"r":3}], ...],  // 揭示后的明牌（只在 PosteriorPrediction 后才填）
  "hand": [{"s":1,"r":3}, ...],         // 当前用户的手牌（其他玩家为 null）
  "times": [
    // 每玩家：[(uid, a_ms, b_ms, remaining_ms), ...]
    [4, 30000, 60000, 89000],
    [1, 30000, 60000, 85000],
    ...
  ],
  "history": [
    // 每玩家 [(uid, [card, ...])]
    [4, [{"s":2,"r":5}, {"s":0,"r":12}]],
    ...
  ],
  "pending_events": [
    // 这轮产生的服务端事件（每轮 settle 后清空）
    {"kind":"PredictionAccepted","uid":1},
    {"kind":"CardPlayed","uid":1},
    {"kind":"PosteriorPredictionAccepted","uid":1},
    {"kind":"RoundResult","round":0,"cards":[...],"ranking":[...],"prediction":[...],"posterior_prediction":[...],"score_delta":[...],"rank_score":[...],"prediction_score":[...],"posterior_score":[...]},
    {"kind":"PhaseChanged","phase":"prior_prediction"}
  ],
  "is_over": false,
  "online": [1, 2, 3, 4, 5]            // 当前在线的 WS uid 列表（inject_online 加进去的）
}
```

### 字段说明

| 字段 | 类型 | 说明 |
|---|---|---|
| `phase` | string | `"waiting_all"` / `"prior_prediction"` / `"play"` / `"posterior_prediction"` / `"ended"` |
| `round` | int | 0..=4 |
| `current_player` | uid? | 当前轮到思考的玩家；Play 阶段为 `null` |
| `start_player` | uid | 首位玩家（每轮逆时针轮转）。首玩家在 PosteriorPrediction 提交完整排名 |
| `players` | [uid] | 座位 0..4 上的 uid 顺序 |
| `seats` | [int] | 冗余（seat 0..4）|
| `scores` | [(uid, score_int)] | 当前总分，按座位顺序 |
| `predictions` | [(uid, rank\|null, has_predicted)] | 每人先验预测状态 |
| `committed` | [(uid, card\|null)] | 每人出牌状态，Play 阶段填；null = 未出牌 |
| `posterior` | [(uid, list\|null, committed_bool)] | 每人后验状态；只有首位玩家那条 committed=true 且 list 非空 |
| `posterior_draft` | {uid: rank} | 实时编辑中的草稿，PosteriorPrediction 阶段才有 |
| `table` | [(uid, card)] | 已揭示的明牌（PosteriorPrediction 完成后才有）|
| `hand` | [card] | 当前用户的手牌（按 player_specific 切换；他人为 null）|
| `times` | [(uid, a_ms, b_ms, remaining_ms)] | 时间池（见 05-timing.md）|
| `history` | [(uid, [card])] | 每人累计打过的牌（reveal 时 append）|
| `pending_events` | [Event] | 这轮产生的服务端事件；frontend 用来显示 round summary |
| `is_over` | bool | 5 轮结束 |
| `online` | [uid] | 当前连上的 WS uid（SDK 的 `inject_online` 加的）|

### Card 编码

`{s: 0..3, r: 0..12}` —— 直接用 `Suit::as_code()` 和 `Rank::as_code()`：

| s | 花色 |
|---|---|
| 0 | ♠ Spade（黑桃）|
| 1 | ♥ Heart（红桃）|
| 2 | ♦ Diamond（方块）|
| 3 | ♣ Club（梅花）|

| r | 牌面 |
|---|---|
| 0 | A |
| 1-9 | 2-10 |
| 10 | J |
| 11 | Q |
| 12 | K |

committed_card 字段还额外带 `hidden: bool`：
- `owner`（viewer == card.uid）→ `hidden: false`，正面朝上
- 其他人 → `hidden: true`，背面

---

## 3. Action Schema（前端 → 服务端）

前端通过 `game_ws.send(JSON.stringify({type:"game", data: {action: ..., ...}}))` 发。

### 3.1 `predict` — 先验预测

```json
{"action": "predict", "rank": 3}      // 猜第 3 名
{"action": "predict", "rank": null}   // 放弃
```

校验（`rules.rs::apply_predict`）：
- 相必须是 `PriorPrediction`
- `current_player` 必须是自己
- `has_predicted` 必须为 false（不能再改）
- `rank` 必须在 1..=5 内（null 也合法）

### 3.2 `play_card` — 出牌

```json
{"action": "play_card", "card_index": 0}   // 出手牌里第 1 张（0-based）
```

校验：
- 相必须是 `Play`
- `committed_card` 必须为 None（不能改）
- `card_index` 必须 < `hand.len()`

### 3.3 `draft_posterior` — 后验预测实时编辑（5×5 网格）

```json
{"action": "draft_posterior", "assignments": {"1": 1, "3": 2, "5": 3, "2": 4, "4": 5}}
```

- `assignments` 是 `{uid: rank}` dict：每个玩家分配到 1..5 名次
- 允许**部分**（不放满 5 个也算合法，draft 状态）
- 校验：每个 uid 存在、每个 rank 在 1..=5、rank 不重复
- 由 `apply_posterior_draft` 处理，**不**影响 `posterior_prediction`（那是 commit 的）

### 3.4 `posterior_predict` — 后验预测提交（commit）

```json
{"action": "posterior_predict", "rank_list": [1, 3, 5, 2, 4]}    // best → worst
{"action": "posterior_predict", "rank_list": []}                  // 跳过
```

- 完整 5 个（best → worst）的排名
- 空 = 跳过
- 校验：长度 == 5 且不重复；只能 1 次（commit 后锁）
- 由 `apply_posterior` 处理

### 3.5 `restart_vote` — 5 轮后投票再来一局

```json
{"action": "restart_vote", "yes": true}
{"action": "restart_vote", "yes": false}
```

- 只有当 `is_over`（5 轮结束）且全 `yes` 才真正重启
- 由前端 `showGameOver` 弹窗触发

---

## 4. 错误响应

服务端在 `handle_action` 失败时返回 `ActionOutcome::Reject(reason)`，SDK 发 `game_error` 给前端：

```json
{"type":"game_error","code":"INVALID_MOVE","message":"not your turn (expected uid=1, got 2)"}
```

`code` 来自 `ActionOutcome::Reject` 包装（SDK 层面），`message` 是后端的 `Display` 输出。

---

## 5. 事件（`pending_events` 的每个 Event）

每次 `handle_action` 调 `rules.rs::apply_*` 后，事件累积到 `state.pending_events`。下一个 snapshot 把它发给所有客户端。

| Event | 序列化 | 何时产生 |
|---|---|---|
| `PredictionAccepted { uid }` | `{"kind":"PredictionAccepted","uid":N}` | `apply_predict` Ok |
| `CardPlayed { uid }` | `{"kind":"CardPlayed","uid":N}` | `apply_play_card` Ok |
| `PosteriorPredictionAccepted { uid }` | `{"kind":"PosteriorPredictionAccepted","uid":N}` | `apply_posterior` Ok |
| `RoundResult { round, cards, ranking, ... }` | `{"kind":"RoundResult","round":N,...}` | `finish_round` |
| `PhaseChanged { phase }` | `{"kind":"PhaseChanged","phase":"play"}` | `advance_phase` 切换阶段 |
| `GameEnded { final_scores }` | `{"kind":"GameEnded","final_scores":[[uid,score],...]}` | 5 轮结束 |

`pending_events` 在 `begin_next_round` 里**清空**，避免跨轮累积。

---

## 6. 完整状态转换图

```
[T0] 玩家 0..4 全部 login
     → phase: WaitingAll → PriorPrediction
     → current_player = start_player
     → start_thinking()  // 只有 current_player 的 thinking_since = now

[T1] 玩家 i 提交 predict
     → apply_predict(i, rank) Ok
     → pending_events += [PredictionAccepted { uid: i }]
     → settle_action(i)  // 扣 A/B, refill A
     → advance_phase:
         → next_unacted() 找下一个
         → current_player = next
         → start_thinking()  // 下一个玩家的时钟重置
         → return
     → broadcast snapshot

[T2] 5 人 predict 完
     → advance_phase: next_unacted() -> None
     → phase = Play
     → current_player = None
     → start_thinking()  // 5 人同时 thinking_since = now
     → pending_events += [PhaseChanged { phase: "play" }]
     → broadcast snapshot

[T3] 玩家 i 提交 play_card
     → apply_play_card(i, card_index) Ok
     → pending_events += [CardPlayed { uid: i }]
     → settle_action(i)  // 扣 A/B, refill A
     → advance_phase: 任何 committed_card.is_none() → return
     → broadcast snapshot

[T4] 5 人 play_card 完
     → advance_phase: 全员 committed_card.is_some()
     → phase = PosteriorPrediction
     → current_player = Some(start_player)
     → start_thinking()  // 只 start_player 的 thinking_since = now
     → pending_events += [PhaseChanged { phase: "posterior_prediction" }]
     → broadcast snapshot

[T5] start_player 提交 draft_posterior
     → apply_posterior_draft → posterior_draft = assignments
     → 不进 pending_events（编辑不产生事件）
     → broadcast snapshot

[T6] start_player 提交 posterior_predict
     → apply_posterior
     → pending_events += [PosteriorPredictionAccepted { uid: start_player }]
     → settle_action(start_player)
     → advance_phase: Phase::PosteriorPrediction 分支
         → reveal_plays()
         → pending_events += [RoundResult]
         → round += 1
         → if round == 5: phase = End, pending_events += [GameEnded]
         → else: begin_next_round()
             → pending_events += [PhaseChanged { phase: "prior_prediction" }]
             → start_thinking()
     → broadcast snapshot

[T7] 客户端收到 RoundResult
     → 前端 `showRoundSummary(ev)`（实际有 3 秒 reveal buffer，详见 04-ui §6）
     → 列表展示：本轮排名 + 出牌 + 预测命中情况 + 三项分

[T8] End 时
     → pending_events 含 GameEnded
     → 前端 `showGameOverForCard(s)` 计算总分，显示最终排名
```

---

## 7. 客户端代码定位

| 字段 | frontend 用法 |
|---|---|
| `phase` | `renderCardBoard` 入口分支 |
| `current_player` | `panel.classList.add("is-active")` 谁的座位高亮 |
| `start_player` | `panel.classList.add("is-first-player")` 谁的"首位"标签 |
| `players` / `seats` | 座位布局（按 self 旋转）|
| `scores` | `seat-score` 文本 |
| `predictions` | `prior` 栏（`第 N 名` / `放弃` / `—`）|
| `committed` | `committed-slot`（自己明面 / 他人背面）|
| `posterior` | `posterior` 栏（后验显示）|
| `posterior_draft` | `buildPosteriorRankRow` 5×5 网格 |
| `table` | 前端中央桌面的明牌 |
| `hand` | 自己手牌（手牌区）|
| `times` | `buildTimeEl` 只对自己显示 |
| `pending_events` | `renderCardBoard` 末尾 fire round summary |
| `online` | `is-offline` 类标灰 |
