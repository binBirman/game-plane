# TYP 变更历史

> 关键 bug 修复和功能变更。`git log` 看代码 diff；本文件给**为什么**和测试方法。

---

## 2026-08 累计变更

### 1. 牌型排序：黑桃 > 红桃 > 方块 → 黑桃 > 红桃 > 梅花

**用户反馈**：排序时数字相同时应该是「黑红梅方」（Spade > Heart > Club > Diamond），不是「黑红方梅」。

**`card.rs`**：
- 原始：Diamond=2, Club=1
- 修改后：Club=2, Diamond=1

**测试**：`card::tests::suit_ranking_is_spade_heart_club_diamond` + `cmp_table_same_rank_uses_suit_order`。

---

### 2. 计时 bug：等待时间被错算到下一玩家头上

**症状**：玩家 1 在 commit 时被扣 6s（包含 5s 的等待时间），但实际只思考 1s。

**根因**（`state.rs::start_thinking` 旧版）：

```rust
Phase::PriorPrediction => !p.has_predicted,  // 任何没 predict 的玩家
```

5 个玩家在 phase 开始时都被设 `thinking_since = Some(T0)`。

玩家 0 commit → `settle_action(0)` 扣 elapsed=5s，A 30→25，refill 30。

`advance_phase` 切到玩家 1，但**没调 `start_thinking`**。玩家 1 的 `thinking_since` 还是 T0。

玩家 1 commit → `settle_action(1)` 算 elapsed = `now - T0 = 6s`，扣 6s。

**修复**（`state.rs` + `logic.rs`）：

1. `start_thinking` 的 `PriorPrediction` 分支改为 `seat == current_player && !has_predicted`（只对当前思考者设）。
2. **`start_thinking` 总是 reset** `thinking_since`（删 `is_none()` 检查）。
3. `advance_phase` PriorPrediction 分支在 `current_player = Some(next)` 之后调 `start_thinking()` 重置下一个玩家的时钟。

**测试**：`state::tests::start_thinking_*`（4 个单元测试）。

**配套修改**：`s.posterior.find(([, , committed]) => committed)` 替代 `s.posterior.find(([u]) => u === s.start_player)`，因为 `advance_phase` 切到下一轮时 `s.start_player` 已经指向下一轮首玩家，`committed=true` 的 tuple 是上一轮首玩家的。

---

### 3. 后验预测 3 秒揭示 + 轮结算延迟

**用户反馈**：commit 之后轮结算页"秒弹"，没有 3 秒阅读时间。

**前端**（`app.js`）：

1. `renderCardBoard` 入口处加 `revealActiveAt` 触发器：用 `s.posterior.find(([, , committed]) => committed)` 找首玩家的已提交 tuple。`s.round` 进 `shownPosteriorReveals` Set 防重复。设置 `revealActiveAt = Date.now() + 3000`。
2. `events.forEach` 处理 `RoundResult`：`delay = max(0, revealActiveAt - Date.now())`，>0 用 `setTimeout`。
3. 触发器放在 events loop **之前**——否则触发器的写入没用。

**后端无变化**——`RoundResult` 一直在 snapshot 的 `pending_events` 里。

---

### 4. 后验预测字典协议

**之前**：后端用 `Vec<i64>`（best→worst），前端必须按顺序填 1,2,3,4,5。

**修改后**：后端用 `BTreeMap<i64, u8>`（uid → rank），前端 5×5 网格自由点。

- `apply_action` 的 `draft_posterior` 分支解析 `assignments: {uid: rank}` dict
- `apply_posterior_draft` 接受**部分** dict（不要求全部 5 个）
- `apply_posterior` 仍要求严格 5 个不重复（commit 时校验）
- snapshot 加 `posterior_draft: {uid: rank}` 字段，编辑时实时同步

**测试**：`rules.rs::apply_posterior_draft` 接受 `{uid: rank}` dict。

---

### 5. 后验排名显示在座位面板

**用户反馈**：commit 后看不到每个玩家的预测名次。

**前端**（`app.js` `buildSeatPanel`）：

```javascript
const postEntry = (s.posterior || []).find(([, , committed]) => committed);
if (postEntry) {
    const idx = committedList.indexOf(uid);
    return idx >= 0 ? `第 ${idx + 1} 名` : "—";
}
```

只显示前 5 个玩家（按 first_player 的 committed list 索引）；跳过则显示「未预测」。

**关键 bug**：原版用 `s.posterior.find(([u]) => u === s.start_player)` 找提交 tuple——但 `advance_phase` 切到下一轮时 `start_player` 已经轮转，`find` 找错了人 → 显示「—」。改用 `find(([, , committed]) => committed)` 按 committed 标志找。

**3 秒高亮**：CSS `.posterior.committed.flash` 动画 + `posterior-flash` 关键帧（0.18s 缩放 + 颜色脉冲）。

---

### 6. 出牌时钟在 commit 后停止

**用户反馈**：出牌时玩家思考时间在 commit 后还在掉。

**修复**（`app.js` `buildTimeEl`）：

```javascript
} else if (s.phase === "play") {
    const committedEntry = (s.committed || []).find(([u]) => u === uid);
    const hasCommitted = !!(committedEntry && committedEntry[1] != null);
    isActiveThinker = !hasCommitted;
}
```

Play 阶段：只有 `committed_card == null`（即还没出牌）的玩家才 `isActiveThinker = true`。已出牌的玩家时钟暂停。

---

### 7. 只显示自己座位的时间

**用户反馈**：其他玩家也显示时间，看着乱。

**修复**（`app.js` `buildSeatPanel`）：

```javascript
if (isSelf) {
    const timeEl = buildTimeEl(s, uid);
    if (timeEl) panel.appendChild(timeEl);
}
```

只对 `isSelf` 渲染 `.seat-time`。其他玩家面板没有时间元素。

---

### 8. 公告栏可滚动日志上限

**用户反馈**：append `textContent +=` 单文本节点导致卡顿。

**修复**（`app.js` `gameLog`）：

```javascript
function gameLog(line) {
    ...
    const entry = el("div", { class: "log-entry" }, `[${ts}] ${line}`);
    log.appendChild(entry);
    while (log.children.length > 300) {
        log.removeChild(log.firstChild);
    }
}
```

每条一个 `.log-entry` div，上限 300 条。

---

### 9. game_log! 协议 + 文档

**新增**：结构化游戏日志协议——游戏进程用 `game_sdk::game_log!(level, "msg", k=v, ...)` 向 stderr 写 JSON Lines，lobby 解析后用 `tracing` 重新发射（保留 level / target / fields）。

**文件**：`crates/game-sdk/src/log.rs`（新）+ `lib.rs`（宏）+ `lobby/src/instance/manager.rs`（JSON 解析 + 兼容 fallback）。

**文档**：`/docs/game_log_protocol.md`。

**配置**：`lobby.env` 的默认 `RUST_LOG` 加 `lobby::game_stderr=debug`，让 fallback 文本日志（`init_tracing` 之类）能显示。

---

### 10. 玩家登录后立刻显示昵称

**用户反馈**：刷新页面后昵称显示成 UID。

**修复**（`app.js`）：

1. `state.roomCache` 持久化到 `localStorage`（`lobby_room_cache` 键），用 `parseInt` 还原 number key。
2. `state.currentRoomId` 持久化到 `localStorage`（`lobby_current_room_id` 键）。
3. `loadRoomCacheFromStorage` 在 state 初始化时同步调用。
4. `saveRoomCacheToStorage` 在 `state.roomCache.set(roomId, r)` 后调。

`getNickname(uid)` 继续依赖 `state.roomCache.get(state.currentRoomId)`，但这两个值现在跨刷新持久化。

---

### 11. 桌位轮转 bug 试运行报告（无生产问题）

测试中观察到"同一房间第二次开启 TYP 时，轮结算页无法正常弹出，需要退出重进才行"。

**诊断**：是 `renderCardGameStage` 没把所有模块级状态重置——核心是 `previousRenderedPhase`、`cardSelectedIndex`、`typSnapshot`、`typCountdownTimer`、`gameOverNavTimer`、`pending_events` 队列、`posteriorReveals` / `posteriorFlash` 等 Set、`tab/card-state` 状态。

**修复**（`renderCardGameStage` 全部重置）：

```javascript
boardState = null;
typSnapshot = null;
cardSelectedIndex = -1;
prevRenderedPhase = null;
pendingPredictRank = undefined;
pendingPosteriorRanks = {};
shownRounds = new Set();
shownPosteriorReveals = new Set();
shownPosteriorFlash = new Set();
revealActiveAt = 0;
roundSummaryQueue.length = 0;
if (typCountdownTimer) { clearInterval(typCountdownTimer); typCountdownTimer = null; }
typSnapshotAt = Date.now();
if (gameOverNavTimer) { clearTimeout(gameOverNavTimer); gameOverNavTimer = null; }
const prevModal = document.getElementById("game-over-modal");
if (prevModal) prevModal.classList.add("hidden");
const prevBanner = document.getElementById("round-summary-banner");
if (prevBanner) prevBanner.remove();
```

---

### 12. RoundResult 触发器阶段判断：不要硬靠 `phase === "posterior_prediction"`

**症状**：上一轮首玩家提交后，snapshot 的 `s.phase` 已经是 `prior_prediction`（advance_phase 切到下一轮）。触发器判 `phase === "posterior_prediction"` 永远 false → 不 fire → 秒弹。

**修复**：触发器用 `find(([, , committed]) => committed)` 按 committed 标志找（不看 phase）；`revealActiveAt` 在 events loop 之前写。

---

### 13. `tictactoe` 名字歧义 / `RuleError::InvalidCard` 不存在

之前 `err_kind` 函数引用 `RuleError::InvalidCard`，但 enum 里没这个变体。删掉。

---

### 14. 终局时序：轮结算先弹，关闭后再弹游戏结束页

**用户反馈**：最后一轮结算，应该先显示轮结算，关闭轮结算后才显示整局结算页面。

**之前**：`renderCardBoard` 在 `s.is_over || s.phase === "ended"` 时直接 `showGameOverForCard(s)`，和 RoundResult 弹窗同时出现。

**修复**（`app.js`）：

1. 新增模块变量 `pendingGameOver = false`。
2. `renderCardBoard`：`s.is_over || s.phase === "ended"` 时只设 `pendingGameOver = true`，不再直接调 `showGameOverForCard`。
3. `showRoundSummary` 的关闭路径（点「关闭」按钮 / 15 秒自动关）抽成 `finishIfPending()`：
   - 移除轮结算弹窗
   - 若 `pendingGameOver` 为 true → 清空标志 + `showGameOverForCard(typSnapshot)`
4. `renderCardGameStage` 重置 `pendingGameOver = false`。

时序：第 5 轮 commit → snapshot(`ended`) → 设 `pendingGameOver` → events loop 3 秒后 `showRoundSummary` → 用户关闭 → `showGameOverForCard`。

---

## 15. 单元测试覆盖历史

| 阶段 | 测试 |
|---|---|
| 0.1 | 无 |
| 0.3 | `state::tests` 4 个：`start_thinking` 3 个 + `play_phase` 1 个 |
| 0.3 | `card::tests` 2 个：花色排序 + `cmp_table` 对称性 |
| 0.3 | `game-sdk::tests` 3 个：log 序列化 + Level 字符串 + 字段辅助 |
| 0.3 | `lobby::tests::game_log_parse` 6 个：JSON Lines 解析 roundtrip |

---

## 16. 部署历史

| 版本 | 日期 | 备注 |
|---|---|---|
| V1.0 (0.1.0) | 2026-08-11 | 单 tictactoe |
| V1.1 (0.2.0) | 2026-08-11 | 加入 take_your_position 骨架 |
| V1.1 (0.3.0) | 2026-08-17 | TYP 完成 + 后验 dict 协议 + game_log 协议 + 计时 bug 修复 |

部署目标：`8.148.5.15`（生产），`192.168.69.129`（开发）。
