# TYP 计时系统（A+B 双池）

> 详细架构背景见 `/docs/architecture.md`。本文件专注 TYP 的 A+B 双池计时实现 + 修复历史。

---

## 1. 概念

每个玩家有**两个独立的时间池**：

| 池 | 含义 | 何时变化 |
|---|---|---|
| **A**（refresh pool） | 每次行动思考时间上限 | 每次 `settle_action` 后**自动回满** |
| **B**（reserve pool） | 整局备用时间 | **永不重置**；A 耗尽后才用 |

**总预算 = A + B**。两个池都没时间 → `out_of_time` → 自动代理（predict 跳过 / play_card 0 / posterior 跳过）。

**0 = 不限时**（如 `300+0` 含义：预测 5 分钟、出牌不限时）。

---

## 2. 后端实现

### 2.1 `StepTimers` (`state.rs:73-93`)

```rust
pub struct StepTimers {
    pub refresh_ms: u64,  // A 池满池值
    pub reserve_ms: u64,  // B 池满池值
}

impl StepTimers {
    pub fn from_preset(preset: Option<&str>) -> Self {
        let (a, b) = match preset.unwrap_or("30+60").split('+').collect::<Vec<_>>()[..] {
            [a, b] => {
                let a: u64 = a.trim().parse().unwrap_or(30);
                let b: u64 = b.trim().parse().unwrap_or(60);
                (a, b)
            }
            _ => (30, 60),
        };
        Self { refresh_ms: a * 1000, reserve_ms: b * 1000 }
    }
}
```

### 2.2 `PlayerState` 时间字段

```rust
pub struct PlayerState {
    pub time_a_ms: u64,                  // A 池剩余
    pub time_b_ms: u64,                  // B 池剩余
    pub thinking_since: Option<Instant>,  // 当前思考起点（None = 没在思考）
}
```

### 2.3 思考时机模型（**关键、已修复**）

每个玩家有 `thinking_since`，`Some(t0)` 表示"正在思考中，起点是 t0"。

**每个阶段哪些玩家是"当前思考者"**（`state.rs::start_thinking`）：

| 阶段 | 思考者 |
|---|---|
| `WaitingAll` / `End` | **无**（全部 `thinking_since = None`） |
| `PriorPrediction` | **仅 `current_player`**（其余 4 人等）|
| `Play` | **所有人**（未 `committed_card` 的 5 人同时思考）|
| `PosteriorPrediction` | **仅 `start_player`**（其余 4 人只读）|

**`start_thinking` 干三件事**：
1. 对 `needs_act` 的玩家：`thinking_since = Some(now)`
2. 对其他玩家：`thinking_since = None`
3. 写一条 `game_log!(debug, "start_thinking", phase, round, active_players)` 方便排查

### 2.4 times 字段（snapshot 里的）

```rust
"times": [
    [uid, a_ms, b_ms, remaining_ms],
    ...
],
```

- `a_ms` / `b_ms` — 当前 A 池 / B 池剩余
- `remaining_ms` — `_rem = aMs + bMs - elapsed_at_snapshot`（在快照时刻已经扣过该玩家当前思考的 `elapsed`）

### 2.5 `settle_action` 流程

```rust
pub fn settle_action(&mut self, seat: usize) {
    let elapsed = self.thinking_elapsed_ms(seat);
    if elapsed > 0 {
        let a = &mut self.players[seat].time_a_ms;
        let take_from_a = (*a).min(elapsed);
        *a -= take_from_a;
        let rest = elapsed - take_from_a;
        if rest > 0 {
            let b = &mut self.players[seat].time_b_ms;
            *b = b.saturating_sub(rest);
        }
    }
    self.players[seat].thinking_since = None;
    // Refill A after the action.
    if let Some(t) = self.timers {
        self.players[seat].time_a_ms = t.refresh_ms;
    }
}
```

**A 优先扣，扣完了才扣 B**。再 refill A 到 `refresh_ms`。B 永远是"还剩多少"。

### 2.6 思考者切换：`advance_phase` 调 `start_thinking`

```rust
Phase::PriorPrediction => {
    if let Some(next) = self.state.next_unacted() {
        self.state.current_player = Some(next);
        self.state.start_thinking();  // ← 关键：下一玩家时钟重置
        return;
    }
    ...
}
```

`start_thinking` 在每次切换时调一次，**确保新玩家的 `thinking_since = now`**。

### 2.7 自动代理（`tick()`，1s 一次）

```rust
fn tick(&mut self) {
    match self.state.phase {
        Phase::PriorPrediction => {
            for seat in seats where !has_predicted && out_of_time(seat) {
                if let Ok(ev) = self.state.apply_predict(uid, None) {
                    self.state.settle_action(seat);
                    acted = true;
                }
            }
        }
        Phase::Play => {
            for seat in seats where !committed_card && out_of_time(seat) {
                if let Ok(ev) = self.state.apply_play_card(uid, 0) {
                    self.state.settle_action(seat);
                    acted = true;
                }
            }
        }
        Phase::PosteriorPrediction => {
            if self.state.out_of_time(self.state.start_player) {
                if let Ok(ev) = self.state.apply_posterior(uid, vec![]) {
                    self.state.settle_action(self.state.start_player);
                    acted = true;
                }
            }
        }
        _ => {}
    }
}
```

策略：跳过（rank=None）/ 出第一张牌 / 后验设为空。

---

## 3. 前端实现

### 3.1 只对自己显示时间

`renderSeatPanel` 里：

```javascript
if (isSelf) {
    const timeEl = buildTimeEl(s, uid);
    panel.appendChild(timeEl);
}
```

其他玩家不渲染 `.seat-time`，所以不会有"别人在掉时间"的错觉。

### 3.2 `buildTimeEl` 拆 A/B

```javascript
function buildTimeEl(s, uid) {
    const t = (s.times || []).find(([u]) => u === uid);
    if (!t) return null;
    let [, aFull, bFull, remInit] = t;
    const inPhase = s.phase === "prior_prediction" || s.phase === "play" || s.phase === "posterior_prediction";
    let aRem, bRem;
    if (inPhase) {
        // 1. 是否当前思考者？
        let isActiveThinker;
        if (s.phase === "prior_prediction") {
            isActiveThinker = s.current_player === uid;
        } else if (s.phase === "play") {
            const committedEntry = (s.committed || []).find(([u]) => u === uid);
            const hasCommitted = !!(committedEntry && committedEntry[1] != null);
            isActiveThinker = !hasCommitted;
        } else {
            isActiveThinker = s.start_player === uid;
        }
        // 2. 按当前时钟衰减
        const decayMs = isActiveThinker ? Math.max(0, Date.now() - typSnapshotAt) : 0;
        const remaining = Math.max(0, remInit - decayMs);
        // 3. A 先扣，A 满了的都给 A
        const totalElapsed = (aFull + bFull) - remaining;
        aRem = Math.max(0, aFull - totalElapsed);
        bRem = Math.max(0, remaining - aRem);
    } else {
        aRem = aFull; bRem = bFull;
    }
    // 渲染 "30s + 60s" 或 "0s" 等
    ...
}
```

### 3.3 1s 局部刷新

```javascript
function updateTimeDisplays(s) {
    const positions = ["seat-b", "seat-l", "seat-tl", "seat-tr", "seat-r"];
    for (const id of positions) {
        const seatEl = $("#" + id);
        if (!seatEl) continue;
        const uidAttr = seatEl.dataset.uid;
        if (!uidAttr) continue;
        const uid = parseInt(uidAttr, 10);
        const oldTime = seatEl.querySelector(".seat-time");
        if (!oldTime) continue;  // 只更新有 .seat-time 的 seat（自己的）
        const newTime = buildTimeEl(s, uid);
        if (newTime) oldTime.replaceWith(newTime);
    }
}
```

每秒 1 次，只 `replaceWith` 自己的 `.seat-time` 节点，**不**重渲染整个 seat panel（避免按钮等焦点丢失）。

---

## 4. Bug 修复历史

### 4.1 ✅ 修复 #1：`start_thinking` 把所有玩家都设了 `thinking_since`

**症状**：玩家 1 在 commit 时被扣 6s（包含 5s 的等待时间），但实际只思考 1s。

**根因**（`state.rs::start_thinking` 旧版）：

```rust
Phase::PriorPrediction => !p.has_predicted,  // 任何没 predict 的玩家
```

游戏一开始，5 个玩家都没 predict → 5 人都 `needs_act=true` → `thinking_since = Some(now)` 都设。

玩家 0 在 T0+5s commit，`settle_action(0)` 扣 5s ✓。

玩家 1 在 T0+6s commit。**问题**：`advance_phase` 切到 player 1 时**没调 `start_thinking`**，所以 player 1 的 `thinking_since` 还是 T0。`settle_action(1)` 算 `elapsed = 6s` 扣 6s。**玩家 1 被多扣 5s**。

**修复**：

1. `start_thinking` 的 `PriorPrediction` 分支改为 `seat == current_player && !has_predicted`（只一把当前玩家的时钟）。
2. **总是** reset `thinking_since`（删除 `is_none()` 检查）。
3. `advance_phase` PriorPrediction 分支在 `current_player = Some(next)` 之后调 `start_thinking()` 重置下一个玩家的时钟。

**修复后**：

- 玩家 1 在 commit 时 `elapsed = 1s`（从 advance_phase 到 commit），只扣 1s ✓
- 等待时间归到玩家 0 自己（玩家 0 实际就想了 5s）

### 4.2 ✅ 修复 #2：`advance_phase` 切下一玩家时没调 `start_thinking`

**症状**：和上一条叠加。即使 `start_thinking` 只对 current_player 标记，没有 advance_phase 的再次调用，next_player 永远没有 `thinking_since`。

**修复同上**：在 `advance_phase` 设置 `current_player` 之后调 `start_thinking()`。

### 4.3 ✅ 修复 #3：tick 1Hz 自动代理

之前 tick 没改过，但同样严格：apply_predict / apply_play_card / apply_posterior 调用后**也要** `settle_action`。否则超时玩家被代理后 A 不扣。

---

## 5. 单元测试

`state.rs::tests` 覆盖：

```rust
#[test] fn start_thinking_only_marks_current_player() { ... }
#[test] fn start_thinking_resets_stale_clock_on_next_player() { ... }
#[test] fn play_phase_marks_all_uncommitted_players_thinking() { ... }
#[test] fn start_thinking_clears_inactive_players() { ... }
```

跑：

```bash
cargo test -p take_your_position state::tests
```

---

## 6. 端到端验证

1. `tools/test.sh` 跑 lobby 烟测
2. Python raw-WS 跑一次完整 TYP：每个玩家 predict 等 1s，看 `start_thinking` 日志的 `active_players` 字段，应该**只有 current_player** 在里面
3. snapshot 的 `times` 字段：等待玩家的 `remaining_ms` 应该**满池**（90s），不应该被扣过

```bash
grep "start_thinking" /tmp/typ_stderr.log | python3 -c "
import json, sys
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try:
        d = json.loads(line)
        f = d.get('fields', {})
        print(f'  phase={f.get(\"phase\"):20s}  active_players={f.get(\"active_players\")}')
    except: pass
"
```

期望输出：

```
  phase=prior_prediction     active_players=[3]
  phase=prior_prediction     active_players=[1]
  phase=prior_prediction     active_players=[5]
  ...
```

**每一步只 1 个 active_player**。
