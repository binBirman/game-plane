# TYP 游戏规则

> 5 人 × 5 轮。每轮：**先验预测 → 5 人同时出牌 → 后验预测 → 结算**。5 轮后游戏结束。

---

## 1. 牌组

一副去掉大小王的扑克牌共52张：

| 花色 | 张数 | 范围 |
|---|---|---|
| ♠ Spade（黑桃） | 13 | A, 2, ..., K |
| ♥ Heart（红桃） | 13 | A, 2, ..., K |
| ♦ Diamond（方块） | 13 | A, 2, ..., K |
| ♣ Club（梅花） | 13 | A, 2, ..., K |

**发牌**：每位玩家 5 张 = `2 小 (A-7 ♥/♣/♦) + 2 大 (8-K ♥/♣/♦) + 1 ♠`。每人**有且仅有 1 张 ♠**。

---

## 2. 5 轮流程

每轮都是这 4 个 phase（`state.phase` 字段）：

```
WaitingAll → PriorPrediction → Play → PosteriorPrediction → (score) → next round
                                                               ↓
                                                            End (round 5)
```

### 2.1 WaitingAll
- 5 个玩家通过 WS `login`/`reconnect` 加入 `state.joined`
- 全部到齐之前，**不计时**（避免"一进去全放弃"）
- 全部到齐 → `on_player_login` 触发：`phase = PriorPrediction`, `current_player = start_player`, `start_thinking()`

### 2.2 PriorPrediction（先验预测）
- **轮流**（不是同时）：`current_player` 依次轮到每位玩家
- 操作：选 1~5 名次自己最终会得，或者「放弃」
- 提交后**锁定**不可改（`has_predicted = true`）
- 全部提交完 → 阶段切换到 `Play`

### 2.3 Play（出牌）
- **5 人同时**：所有未 commit 的玩家都需要行动
- 操作：选手里第 `<card_index>` 张（0-based）→ 提交
- 提交后该玩家**不能再改**（`committed_card = Some(card)`，`hand.remove(idx)`）
- 牌面**不会立刻揭示**——揭晓在 PosteriorPrediction 完成后（`reveal_plays`）
- 自己看自己的牌明面，其他玩家看你的牌背面
- 全部 5 个玩家都提交 → 阶段切到 `PosteriorPrediction`

### 2.4 PosteriorPrediction（后验预测）
- **只有 `start_player`（首位玩家）**可以提交
- 操作：5 人 `best→worst` 完整排名，或「跳过」（空列表）
- 提交后**锁定**（`posterior_prediction = Some(list)`）
- 编辑时实时同步 draft（`actions.draft_posterior.assignment` = `{uid: rank}` dict）
- 提交后：
  - `reveal_plays()`：把 5 张暗牌翻到桌上（`state.table`），append to `played_history`
  - `finish_round()`：`RoundResult` 事件入 `pending_events`
  - `round += 1`
  - 如果 `round == 5` → `phase = End`
  - 否则 → `begin_next_round()`：`start_player` 逆时针轮转一个座位，进入下一轮

### 2.5 End
- 5 轮结束
- 客户端 `phase == "ended"` 或 `is_over == true` → 弹最终排名（按 `phase_after_advance` 之后的所有 `score_delta` 累加）

---

## 3. 排名规则（关键：A/K 特殊裁定）

`card.rs::Card::cmp_table(table)`：

```rust
let has_a = table.iter().any(|c| c.rank == Rank::A);
let has_k = table.iter().any(|c| c.rank == Rank::K);
if has_a && has_k {
    // A 是王牌（同 A 比花色）；K 二牌
    match (self.rank, other.rank) {
        (Rank::A, Rank::A) | (Rank::K, Rank::K) => {}
        (Rank::A, _) => Greater,  // A 打什么都是最大
        (_, Rank::A) => Less,
        (Rank::K, _) => Greater,  // K 仅次于 A
        (_, Rank::K) => Less,
        _ => {}
    }
}
// 其余按 rank，rank 相同按花色：黑桃 > 红桃 > 梅花 > 方块
match self.rank.cmp(&other.rank) {
    Equal => suit_value(self.suit).cmp(&suit_value(other.suit)),
    other => other,
}
```

**含义**：当且仅当本轮桌上**同时有 A 和 K**时：
- A > 任何牌（同 A 比花色）
- K > 除 A 外的所有牌（同 K 比花色）
- 其余按 rank

**A 全打 / K 全打不可能**（5 张牌都 A 或都 K 一手完全不可能），故 `finish_round` 用 `assert!(matches!(accurate, 5 | 3 | 2 | 1 | 0))` 兜底。

---

## 4. 计分

5 轮每轮结束后算一遍总分，**累加到 `player.score`**。每轮三个分项：

| 分项 | 规则 | 范围 |
|---|---|---|
| **排序分** `rank_score` | 真实名次第 1/2/3/4/5 → +2/+1/0/-1/-2 | -2 ~ +2 |
| **先验分** `prediction_score` | 每人猜自己名次：猜对 +2，猜错 -2，跳过 0 | -2 ~ +2 |
| **后验分** `posterior_score` | 首位玩家按完整排名逐位对比，统计 `accurate`（猜对位数）：`5→+2, 3→0, 2→-1, 1→-2, 0→-2`（4 不可能） | -2 ~ +2 |

**总分** = 三项之和（`finish_round` 里累加到 `state.players[seat].score`）。

---

## 5. A+B 双池计时

详见 [`05-timing.md`](05-timing.md)。

---

## 6. 5 局场景举例

以 5 个玩家 Alice/Bob/Carol/Dave/Eve 为例，1 轮为例：

| 步骤 | 动作 | 状态 |
|---|---|---|
| 0 | 5 个玩家登录，game 进入 `PriorPrediction` | current_player = Alice（首玩家） |
| 1 | Alice 提交"我猜第 3 名" | has_predicted[Alice] = true |
| 2 | current_player 翻到 Bob | current_player = Bob |
| 3 | Bob 提交"我猜第 1 名" | has_predicted[Bob] = true |
| 4 | ...（Player Carol/Dave/Eve 都提交） | 全员 has_predicted = true |
| 5 | 阶段切到 `Play` | 5 人同时 thinking |
| 6 | 5 人各自从手牌选一张提交 | committed_card 全员 Some |
| 7 | 阶段切到 `PosteriorPrediction` | current_player = start_player = Alice |
| 8 | Alice 在 5x5 网格里选 5 名次，点「上传」 | posterior_prediction = [uid, ...] |
| 9 | reveal + finish_round | pending_events += [RoundResult{round:0,...}] |
| 10 | 阶段切到下一轮 `PriorPrediction`（start_player = Eve 逆时针） | 回到 0 |

---

## 7. 客户端 UI 流程（粗略）

详见 [`04-ui-integration.md`](04-ui-integration.md)。

粗略：
- 大厅 → 选 `take_your_position` → 创建房间 → 进房间页（5 人齐）
- 房主点「开始」→ 后端 `POST /api/rooms/:id/start` → 后端 spawn game 进程 → 客户端跳到 `#game/<instance_id>`
- 5 个浏览器 WS 连上 `/ws/<instance_id>` → 发 `login` → 收 `login_ok` + `snapshot`
- 5 人齐 → `WaitingAll → PriorPrediction` → 5 个座位面板按 `current_player` 轮流高亮
- 每轮结束 → 收 `pending_events` 含 `RoundResult` → 前后端用 `revealActiveAt` 排队 3 秒再弹 `showRoundSummary`（详见 04-ui §6）
- 5 轮完 → `End` → 弹最终排名（按总分倒序）

---

## 8. 配置文件

`/etc/lobby/games.toml` 注册：

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

`timer_preset` 选项（在 `POST /api/rooms` 时传）：

| 预设 | 每人每次思考 | 每人整局备用 |
|---|---|---|
| `30+60` | 30s | 60s |
| `40+120` | 40s | 120s |
| `60+180` | 60s | 180s |
| `300+0` | 300s | 0（不限时） |

任意 `N+M` 后端都接受，0 表示对应池不限时。
