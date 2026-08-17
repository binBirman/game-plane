# TYP UI 集成

> 前端 UI 怎么把 `snapshot` 数据变成可交互的座位面板、按钮、计时显示、轮结算弹窗。
> 通用设计语言见 `/docs/frontend-design.md`（颜色 token、字体、键盘可达等）。本文件专注 TYP 增量。

---

## 1. 房间卡片

`POST /api/rooms` 创建房间后，前端跳到 `#room/<id>`。TYP 房间卡显示：

```
┌──────────────────────────────────────────┐
│  TYP · Take Your Position       [#25]    │
│  5 人预测出牌，每轮预测名次并出牌           │
│  ┌────────┐  ┌────────┐  ┌────────┐       │
│  │ 30+60 │  │ 40+120 │  │ 60+180 │       │
│  │  快速  │  │  标准  │  │  宽松  │       │
│  └────────┘  └────────┘  └────────┘       │
│  ┌────────┐                                │
│  │ 300+0  │                                │
│  │  超长  │                                │
│  └────────┘                                │
│  玩家: 3 / 5                              │
│  [开始游戏]                                │
└──────────────────────────────────────────┘
```

`timer_preset` 选项来自 `POST /api/rooms` 的 `timer_preset` 字段，UI 选 4 个 preset（`30+60` / `40+120` / `60+180` / `300+0`），钮的 label 写"快速/标准/宽松/超长"，括号 `（ 30+60 s ）` 之类后缀（详见前端 design doc）。

---

## 2. 游戏大厅布局（`#game/<instance_id>`）

整个屏幕分成 5 个座位 + 中央：

```
                  ┌─────────────┐
                  │  seat-tl    │  ← seat 2 (猜对面)
                  │ ┌─────────┐ │
                  │ │  玩家 C  │ │
                  │ │ 头像 + 名 │ │
                  │ │ prior /  │ │
                  │ │  time    │ │
                  │ └─────────┘ │
                  └─────────────┘

┌──────────┐                  ┌──────────┐
│ seat-l   │  ┌────────────┐  │ seat-r   │
│ 玩家 B   │  │ center-area│  │ 玩家 D   │
│          │  │ (Play 区：   │  │          │
│          │  │  5 张中央   │  │          │
│          │  │  出牌格)     │  │          │
│          │  │            │  │          │
│          │  │  Posterior    │  │          │
│          │  │  5×5 网格  │  │          │
│          │  └────────────┘  │          │
└──────────┘                  └──────────┘

                  ┌─────────────┐
                  │  seat-b     │  ← 自己座位（seat 0）
                  │  玩家 A (你) │
                  │             │
                  │  + 手牌区    │
                  │  + 操作面板  │
                  └─────────────┘
```

CSS Grid 用 3×3 布局（见 `crates/lobby/static/style.css` 的 `.card-stage`）。Self 永远在 `seat-b`（底部），其他玩家逆时针排列在 `seat-l / seat-tl / seat-tr / seat-r`。

---

## 3. seat-panel 详情

每个玩家的座位面板包含：

```
┌─────────────────────────────────────────┐
│  [头]  Alice          8 分                │  ← header: avatar + name + score
│       ─────────────                     │
│       prior: 第 3 名                    │  ← 先验预测
│       posterior: 第 5 名                 │  ← 后验预测（committed 后显示）
│       ─────────────                     │
│       [committed card slot]               │  ← Play 阶段揭示后这里显示该玩家的牌
│       24s + 60s   ← 自己的 A+B 时间显示    │
└─────────────────────────────────────────┘
```

每个字段从 snapshot 读：

| UI 元素 | 字段 | 渲染逻辑 |
|---|---|---|
| 头像 | `colorFromUid(uid)` | 哈希→色；active 加蓝色边框 |
| 昵称 | `getNickname(uid)` | localStorage 缓存 + 房间玩家列表 |
| 分数 | `s.scores` | `score = scores[seat].1` |
| 先验 | `s.predictions` | `has_predicted` → "第 N 名" / "放弃" / "—" |
| 后验 | `s.posterior` | 首玩家那条 list → 每人 idx 显示 |
| 时间 | `s.times` + `isActiveThinker` | 只对自己显示（详见 `05-timing.md`）|
| committed 槽 | `s.committed` | Play 阶段揭示后才有 |

---

## 4. 5×5 后验预测网格

PosteriorPrediction 阶段，start_player 自己在自己的座位面板下方看到一个 5×5 网格：

```
   名次  1  2  3  4  5
   ┌──┬──┬──┬──┬──┐
Alice │  │  │ ●│  │  │  → Alice 在第 3 名
   ├──┼──┼──┼──┼──┤
Bob   │  │  │  │  │  │  → Bob 未排
   ├──┼──┼──┼──┼──┤
Carol │  │  │  │  │  │
   ├──┼──┼──┼──┼──┤
Dave  │  │ ●│  │  │  │  → Dave 在第 2 名
   ├──┼──┼──┼──┼──┤
Eve   │  │  │  │  │  │
   └──┴──┴──┴──┴──┘
```

每行是玩家，每列是名次。点格子 = 把那个玩家放到那个名次。已分配的名次在其他玩家那行变灰。再次点击同一格 = 取消分配。

**draft 实时同步**：每次点击 → 发 `draft_posterior` → 后端写 `posterior_draft` → 下一帧 snapshot 包含 draft → 其他 4 个玩家看到 start_player 的选择。

**commit**：点「上传」→ 发 `posterior_predict` → 后端校验全 5 个不重复 → 锁定 → 阶段切换。

---

## 5. 后验 3 秒揭示（关键 UI 模式）

**问题**：commit 之后，5 个玩家都要看到"每个玩家各自的名次"。如果立即显示，用户的眼睛来不及看。

**实现**：seat panel 的「后验」栏在 commit 那一瞬间触发 `flash` 类（CSS 3 秒脉冲动画），文字显示 `第 N 名` 或 `未预测`。3 秒后 flash 淡出，但文字保留。

CSS：

```css
.seat-predictions .posterior.committed.flash {
    animation: posterior-flash 3s ease-out 1;
    background: rgba(250, 204, 21, 0.28);
    outline: 2px solid #facc15;
}
@keyframes posterior-flash {
    0%   { background: rgba(250, 204, 21, 0.65); transform: scale(1.04); }
    60%  { background: rgba(250, 204, 21, 0.22); transform: scale(1.0); }
    100% { background: rgba(250, 204, 21, 0.08); transform: scale(1.0); }
}
```

**触发逻辑**（`app.js` 的 `buildSeatPanel`）：

```javascript
const flashKey = s.round + ":" + uid;
if (!shownPosteriorFlash.has(flashKey)) {
    shownPosteriorFlash.add(flashKey);
    flashCls = "flash";
}
```

每轮每玩家只触发一次。`shownPosteriorFlash` Set 在 `renderCardGameStage` 时清空（下一局重置）。

---

## 6. 轮结算弹窗（3 秒延迟）

后端 commit 之后立刻把 `RoundResult` 塞进 `pending_events`，broadcast snapshot。

**前端收到 snapshot 后**：

1. `renderCardBoard` 的 `events.forEach` 找到 `kind === "RoundResult"` 的事件
2. 检查 `shownRounds.has(ev.round)` → 第一轮第一次见，加 `shownRounds.add`，准备延迟显示
3. 查 `revealActiveAt`（commit 那一瞬由后验触发器设置 = `Date.now() + 3000`）
4. `delay = max(0, revealActiveAt - Date.now())`
5. 如果 `delay > 0` → `setTimeout(() => showRoundSummary(ev), delay)` 否则立即

如果后验触发器没 fire（`revealActiveAt === 0`），`delay = 0` → **秒弹**。所以触发器必须先于 events loop 跑。

**触发器位置**（`renderCardBoard` 内，events loop 之前）：

```javascript
if (Array.isArray(s.posterior)) {
    const postEntry = s.posterior.find(([, , committed]) => committed);
    if (postEntry) {
        if (!shownPosteriorReveals.has(s.round)) {
            shownPosteriorReveals.add(s.round);
            revealActiveAt = Date.now() + 3000;
        }
    }
}
```

**关键**：按 `committed` 标志找（不是 `s.start_player`），因为 `advance_phase` 切到下一轮时 `s.start_player` 已经指向下一轮的首玩家了。

---

## 7. 轮结算弹窗内容

`showRoundSummary(ev)` 渲染：

```
┌─────────────────────────────────────────────┐
│  第 1 轮结算                            [×] │
│                                             │
│  名次  玩家   出牌    排序  先验  后验  本轮  │
│  1    Alice   5♠    +2    +2    —     +4   │
│  2    Bob     7♣    +1    -2    —     -1   │
│  3    Carol   10♥    0    -2    ±2     0   │
│  4    Dave    3♦    -1    +2    —     +1   │
│  5    Eve     K♠    -2    -2    —     -4   │
│                                             │
│  (Alice 整了后验 ±2 = 全部猜对 → +2)        │
│                                             │
│  [关闭]                                      │
└─────────────────────────────────────────────┘
```

`ev` 字段：
- `round` — 标题
- `cards` (按 ranking 排序的 (s, r) 数组)
- `ranking` — best→worst uid 列表
- `prediction` — 每座位先验（Option<u8>）
- `posterior_prediction` — 首玩家的完整排名
- `score_delta` / `rank_score` / `prediction_score` / `posterior_score` — 各项分

15 秒后自动消失，或用户点关闭。

---

## 8. 最终结算（5 轮结束）

**时序**（关键）：最后一轮（第 5 轮）结束**先弹轮结算，用户关闭后才弹游戏结束页**。

后端第 5 轮 commit 后 `advance_phase` 切到 `End`，`pending_events` 同时含 `RoundResult` 和 `GameEnded`。前端 `renderCardBoard` 收到 snapshot（`phase == "ended"` / `is_over == true`）：

1. 设置 `pendingGameOver = true`（**不**直接弹游戏结束页）
2. events loop 照常处理 `RoundResult` → 3 秒延迟后 `showRoundSummary` 弹轮结算
3. 用户点「关闭」（或 15 秒自动关）→ `finishIfPending()`：
   - 移除轮结算弹窗
   - 检查 `pendingGameOver`，为 true → 调 `showGameOverForCard(typSnapshot)` 弹最终结算
   - 清空 `pendingGameOver`

> 非终局轮：`phase` 永远不会到 `"ended"`（`End` 只在第 5 轮后进入），所以 `pendingGameOver` 只会在最后一轮置 true。

```
第 5 轮 commit
  │
  ▼
[轮结算弹窗] ──3 秒延迟──► showRoundSummary(第5轮结果)
  │
  ▼  用户点「关闭」或 15 秒自动关
[游戏结束页] ◄── showGameOverForCard(s)
  │
  ▼  5 秒后自动跳回房间页
```

`showGameOverForCard(s)` 渲染（按 `scores` 排序，`getNickname` 取昵称）：

```
┌─────────────────────────────────────────────┐
│  游戏结束                                  │
│  你的最终名次：第 1 / 5（+12 分）            │
│                                             │
│  1. Alice    +12 分                          │
│  2. Bob      +8  分                          │
│  3. Carol    +3  分                          │
│  4. Dave     -2  分                          │
│  5. Eve      -5  分                          │
│                                             │
│  (5 秒后自动跳到房间页)                       │
└─────────────────────────────────────────────┘
```

---

## 9. 离线 / 在线

后端 `inject_online` 在每个 snapshot 加 `online: [uid, ...]` 字段（连着的 WS uid）。

前端：
- `panel.classList.add("is-offline")` → 头像变灰 + 闪电图标
- 在线的不显示闪电

TYP 没有"离线踢出"逻辑——只标记。如果 5 轮里有玩家断线，实际还在 player 列表里（slot 给了别人），断线的玩家被视为"放弃"。

---

## 10. 计时显示（只对自己）

详见 [`05-timing.md`](05-timing.md)。

每个 1s 的 `updateTimeDisplays` 只刷新自己的 `.seat-time` 元素：
- `isActiveThinker` 判断（PriorPrediction 看 `current_player`、Play 看 `committed`、PosteriorPrediction 看 `start_player`）
- `decayMs = isActiveThinker ? (Date.now() - typSnapshotAt) : 0`
- `remaining = remInit - decayMs`
- 拆 A / B：`aRem = min(aFull, remaining)`、`bRem = remaining - aRem`

`Seat-time` 节点只在 `isSelf` 时被渲染（其他玩家不显示）。

---

## 11. 5×5 网格交互细节

每个玩家行 5 个按钮点 [1, 2, 3, 4, 5] 名次。点击：
- 如果该玩家没有分配 → 给玩家分配这个名次
- 如果该玩家已经分配这个名次 → 取消（toggle off）
- 如果该玩家分配了别的名次 → 移到新名次

实现（`buildPosteriorRankRow`）：

```javascript
btn.addEventListener("click", () => {
    if (btn.disabled) return;
    let next = { ...draft };
    if (myRank === r) {
        delete next[forUid];  // toggle off
    } else {
        next[forUid] = r;    // (re-)assign
    }
    sendCardAction({ action: "draft_posterior", assignments: next });
    renderCardBoard(typSnapshot);
});
```

每个钮在以下情况被禁用：
- 已分配给其他玩家（且不是 toggle off 状态）

`ready` 状态（绿色）= 上传的「上传」按钮：所有 5 名都分配且不重复时才启用。

---

## 12. hand 区的视觉

手牌排成 5 张，水平或弧形。每张牌：

```
┌─────┐
│     │   ← 牌面（s/r 用 card-render.js 渲染 SVG）
│  ♠  │
│  5  │
│     │
└─────┘
```

play 阶段：点选 + 高亮 + 点「确认」提交。`committed_card` 提交后从 hand 移除（snapshot.hand 中该玩家数组变短）。

---

## 13. 主要常量在 CSS 里的位置

| 颜色 | 用途 |
|---|---|
| `--accent #6ea8fe` | 主要操作、链接、active 边框 |
| `--ok #4ade80` | 在线、prior 提交、commit 成功 |
| `--warn #facc15` | posterior 主题色、当前 active 玩家高亮、3 秒揭示 |
| `--error #f87171` | 离线、错误 |

后验 flash 动画用 `facc15`（后验主题色）做 outline 和 background pulse。

---

## 14. 常见 UI 错误及处理

| 场景 | 现象 | 处理 |
|---|---|---|
| 浏览器刷新页面 | 进入游戏 view 但 WS 已断 | `onclose` 触发，"已断开"提示，重新进房间 |
| 5 轮中有人断线 | 该玩家座位变灰 | 其他 4 人继续，5 轮完成时 `online` 不包含他 |
| 玩家 5 轮中刷新 | 座位 panel 空白几秒后 rebuild | `renderCardBoard` 从 snapshot 重建，seat panel 重新出现 |
| 房间被关 | 跳到房间页（如果有 gameOverModal，显示回到房间页）| 5 秒后自动跳 |
| WS 重连 | 重新发 `login` → 收到 login_ok + snapshot | `validate_session` 接受当前 token |
| `is_over == true` 但还在 `#game/<id>` | 显示 `showGameOverForCard` 弹窗 | 点关闭或 5 秒后跳回房间 |
