# 前端设计规范（V1 Frontend Design Spec）

> 本文档定义 Lobby 前端的设计语言、组件库与交互模式。新增页面、组件、状态时，必须先看本文档、再写代码。
>
> 版本：0.1 ｜ 范围：`crates/lobby/static/{index.html, app.js, style.css}` 的所有用户可见部分。

---

## 0. 设计原则

| 原则 | 说明 |
|---|---|
| **语义优先** | 颜色、图标、文案必须传达语义，不要装饰 |
| **状态可见** | 任何异步/错误/成功都要有可见反馈，不靠用户"猜" |
| **键盘可达** | 一切交互元素 Tab 可达，Enter/Space 触发 |
| **留白即信息** | 用空间而不是边框/分隔线来分组 |
| **最小惊讶** | 同类操作在不同页面行为一致（如 join → toast → 跳转） |

> **不**用 emoji 当 UI 元素：装饰性 emoji 可作占位，但**功能性**UI（按钮、状态徽章、icon）必须用文字或 CSS/SVG。Emoji 渲染依赖操作系统/字体，在不同设备上的视觉表现不一致，会破坏品牌一致性（见 §5）。

---

## 1. 设计令牌（Design Tokens）

所有值用 CSS 变量定义在 `:root`。**禁止在组件中硬编码颜色 / 间距 / 字号**。

### 1.1 颜色

```css
:root {
    /* 中性 */
    --bg:        #14141a;   /* 页面底色 */
    --card:      #1f1f27;   /* 卡片 / 表面 */
    --card-2:    #171720;   /* 次级表面（输入框、嵌套块） */
    --border:    #2e2e38;   /* 默认描边 */
    --text:      #ececef;   /* 主文本 */
    --muted:     #8b8b95;   /* 次文本、占位、说明 */

    /* 语义 */
    --accent:    #6ea8fe;   /* 主操作 / 链接 / 焦点 */
    --ok:        #4ade80;   /* 成功 / 在线 / 进度 */
    --warn:      #facc15;   /* 警告 / 进行中 / 中间状态 */
    --error:     #f87171;   /* 错误 / 危险操作 / 离线 */
}
```

**对比度要求**（WCAG AA）：正文 ≥ 4.5:1，大字（≥ 18px / 14px bold）≥ 3:1。

| 用途 | 背景 | 前景 | 实测 |
|---|---|---|---|
| 正文 | `--bg` | `--text` | ≥ 12:1 ✓ |
| 次文本 | `--bg` | `--muted` | ≥ 4.7:1 ✓ |
| 错误提示 | `--card` | `--error` | ≥ 4.5:1 ✓ |

### 1.2 字体

```系统- CSS
font-family: -apple-system, BlinkMacSystemFont, "Segoe UI",
             "PingFang SC", "Microsoft YaHei", sans-serif;
```

系统字体优先：中文走苹方/雅黑，英文走 San Francisco/Segoe UI，**不**引入 Web Font（首屏 + 网络都贵）。

**字号梯度**（rem，基准 16px）：

| Token | rem | px | 用法 |
|---|---|---|---|
| `--fs-xs` | 0.75 | 12 | 辅助信息、标签、UID |
| `--fs-sm` | 0.875 | 14 | 次要正文、列表项 |
| `--fs-md` | 1 | 16 | **正文基准** |
| `--fs-lg` | 1.125 | 18 | 标题、按钮 |
| `--fs-xl` | 1.375 | 22 | H1 |

**字重**：常规 500（按钮、强调），正文 400。仅在必要时 600/700。

### 1.3 间距

**4px 基线网格**：

```css
--space-1: 4px;   --space-2: 8px;   --space-3: 12px;
--space-4: 16px;  --space-5: 24px;   --space-6: 32px;
--space-8: 48px;
```

| 用途 | token |
|---|---|
| 紧贴（如按钮内 padding） | `--space-2` / `--space-3` |
| 控件之间（按钮组） | `--space-3` |
| 卡片内边距 | `--space-4` / `--space-5` |
| 卡片之间 | `--space-4` |
| 区块之间 | `--space-5` / `--space-6` |

### 1.4 圆角

```css
--radius-sm: 6px;   /* 小标签、状态徽章 */
--radius-md: 8px;   /* 输入框、小按钮 */
--radius-lg: 12px;  /* 卡片、模态 */
--radius-pill: 999px; /* 头像、tag */
```

### 1.5 阴影 / 高度

V1 用边框为主，几乎不用阴影：

```css
--shadow-1: 0 1px 2px rgba(0,0,0,0.2);   /* 悬浮提示 */
--shadow-2: 0 4px 12px rgba(0,0,0,0.3);  /* 模态、Toast */
```

---

## 2. 文字语气

**原则**：动词在前、对象在后；不卖萌；不写"哦"、"呢"。

| 场景 | ❌ 反例 | ✅ 推荐 |
|---|---|---|
| 注册成功 | "🎉 注册成功啦！" | "注册成功" |
| 错误 | "出了点小问题哦～" | "密码强度不足：需 ≥ 9 位" |
| 确认 | "确认要离开吗？" | "离开房间？" |
| 操作中 | "正在努力加载中…" | "加载中…" |
| 空状态 | "(空空如也)" | "暂无房间，创建一个吧" |

**按钮文案**：1–4 字，动词开头：

| ❌ | ✅ |
|---|---|
| "点击此处创建房间" | "创建" |
| "好的，我知道了" | "关闭" |
| "请确认取消操作" | "取消" |

---

## 3. 图标

### 3.1 规则

1. **UI 元素不**用 emoji。✕ ○ 📋 🎮 🏁 等全部改为：
 - **CSS 形状**（伪元素 / 内联 SVG / 几何符号）
 - **文字缩写**（如 X / O 直接用字符 + `--accent` / `--warn` 着色）
 - **图标字体**（未来可引入 Material Symbols 或自建 SVG sprite）
2. **状态指示**用 **dot + 文字** 组合，不靠颜色单独传达（色盲友好）：
 - 在线：绿色 dot + "在线"
 - 警告：黄色 dot + "等待"
 - 错误：红色 dot + "已断开"
3. **加载中**：CSS 旋转动画或进度条，不旋转 emoji。

### 3.2 棋盘标记（井字棋）

| 玩家 | 形状 | 颜色 | 备注 |
|---|---|---|---|
| 玩家 0 | **X**（拉丁字母） | `--accent` | 字体加粗即可 |
| 玩家 1 | **O**（拉丁字母） | `--warn` | |

旧实现 `return idx === 0 ? "✕" : "○";` → 改为 `return idx === 0 ? "X" : "O";`（或 SVG）。

### 3.3 游戏图标（注册表）

每个游戏在 `GAME_META` 中提供 `icon`：

```js
const GAME_META = {
    tictactoe: { icon: "tic-tac-toe", name: "井字棋", description: "..." },
};
```

`icon` 是字符串 key，前端用 CSS / SVG sprite 解析（不用 emoji 字符串）。

---

## 4. 组件库

### 4.1 按钮

```jsx
<button class="primary">  创建房间  </button>  // 主操作
<button class="ghost">    取消      </button>  // 次操作
<button class="danger">   离开房间  </button>  // 危险（需与 ghost 区分）
<button class="link">     查看帮助  </button>  // 文字链样式
```

**尺寸**：

| 变体 | padding | font-size | 高度 |
|---|---|---|---|
| 默认 | `8px 14px` | 14px | 36px |
| 小   | `4px 10px` | 13px | 28px |
| 大   | `12px 18px` | 16px | 44px |

**状态**：

| 状态 | 视觉 |
|---|---|
| 默认 | `--accent` 背景 + `#0a0a10` 文本 |
| hover | `filter: brightness(1.08)` |
| focus-visible | `outline: 2px solid var(--accent); outline-offset: 2px` |
| active | `transform: scale(0.98)` |
| disabled | `opacity: 0.5; cursor: not-allowed` |

**禁用时**：必须给 `title` 属性说明原因（"需要至少 2 名玩家"）。

**loading 状态**：改文案（"加入中…"）+ `disabled`，不用 spinner 覆盖按钮（点击区消失让人误以为没生效）。

### 4.2 输入框

```jsx
<label>
  <span>用户名</span>
  <input name="username" required autocomplete="username" />
</label>
```

**规则**：
- 必须有 `<label>`（不仅是 placeholder）
- 错误时：边框 `--error` + 红字提示在下方（不是 toast）
- 密码字段有"显示/隐藏"切换按钮

### 4.3 卡片（Card）

统一 class `card`：

```css
.card {
    background: var(--card);
    border-radius: var(--radius-lg);
    padding: var(--space-5);
}
```

子组件用 `--card-2` 做嵌套层次（如座位网格）。

### 4.4 状态徽章（Status Tag）

`status-Waiting | status-Starting | status-Running | status-Finished | status-Destroyed`：

```css
.status-tag {
    display: inline-block;
    padding: 2px 10px;
    border-radius: var(--radius-pill);
    font-size: var(--fs-xs);
    font-weight: 600;
}
.status-Waiting   { background: rgba(110,168,254,.15); color: var(--accent); }
.status-Starting  { background: rgba(250,204, 21,.15); color: var(--warn); }
.status-Running   { background: rgba( 74,222,128,.15); color: var(--ok); }
.status-Finished  { background: rgba(139,139,149,.15); color: var(--muted); }
.status-Destroyed { background: rgba(248,113,113,.15); color: var(--error); }
```

文字版（中文）：见 §4.10 状态文案表。

### 4.5 Toast / 通知

```jsx
<div class="toast show ok">已加入房间</div>
```

**规则**：
- 顶部右侧悬浮，3 秒自动消失（成功类）；错误 6 秒
- 同屏最多 1 条，新 toast 替换旧的
- 文案 ≤ 18 字；错误用 `--error` 描边 + 图标前缀
- **禁止**用 emoji 替代图标（`✕` `✓` 等）

### 4.6 模态 / 对话框

V1 暂未使用。如未来加（如"确认删除房间"）：
- 居中卡片，`--card` 背景，`--shadow-2`
- 背景半透明黑色蒙层 `rgba(0,0,0,0.6)`
- ESC 关闭，背景点击关闭
- 焦点锁定在模态内

### 4.7 列表项 / 房间条目

```jsx
<div class="room-item">
    <div class="info">
        <div class="meta-row">
            <strong>#42</strong> · 井字棋
            <span class="status-tag status-Waiting">等待中</span>
        </div>
        <div class="meta">玩家 1/2 · host: alice</div>
    </div>
    <button class="primary">进入</button>
</div>
```

### 4.8 座位网格（房间详情）

```jsx
<div class="seat-grid">
    <div class="seat seat-filled seat-self">
        <span class="seat-num">座位 0</span>
        <span class="seat-avatar">A</span>          <!-- 头像首字母 -->
        <span class="seat-name">alice  你</span>
        <span class="seat-uid">#1</span>
    </div>
    <div class="seat seat-empty">
        <span class="seat-num">座位 1</span>
        <span class="seat-avatar">·</span>
        <span class="seat-name muted">等待玩家</span>
    </div>
</div>
```

`seat-avatar` 用首字母 + 圆形背景，**不**用 emoji 或 SVG 头像占位。

### 4.9 加载 / 空 / 错误状态

每个视图都要显式处理这三种状态：

| 状态 | 视觉 |
|---|---|
| **加载** | 骨架屏（`opacity: 0.5` + 静态占位）或行内 "加载中…" |
| **空** | 居中卡片 + 说明文字 + 主操作按钮（如"创建房间"） |
| **错误** | 红字 + 重试按钮，**不**用 alert/confirm |

### 4.10 状态文案（中英对照）

| 状态 key | 中文 | 英文（预留） |
|---|---|---|
| `Waiting` | 等待中 | Waiting |
| `Starting` | 启动中 | Starting |
| `Running` | 进行中 | In Progress |
| `Finished` | 已结束 | Finished |
| `Destroyed` | 已销毁 | Closed |

actionHint 文案：

| 场景 | 文案 |
|---|---|
| 非成员、Waiting | "空闲中，加入即可参与。" |
| 房主、Waiting、< min_players | "你是房主，至少需要 N 名玩家才能开始。" |
| 房主、Waiting、满员 | "所有玩家已就位，可以开局了。" |
| 成员、Waiting | "等待房主开局…" |
| Starting | "正在启动游戏进程…" |
| Running | "游戏进行中。" |
| Finished | "本局已结束。" |

---

## 5. Emoji 使用规则（硬性约束）

| 场景 | 允许 | 禁用 |
|---|---|---|
| 标题、正文 | — | 一切装饰性 emoji |
| 按钮文案 | — | "🎮 进入游戏" "📋 复制" "✓ 完成" |
| 状态徽章 | — | "🏁 已结束" "⏳ 等待中" |
| Toast | — | "🎉 成功！" |
| 玩家头像 | — | 首字母即可，不放 emoji |
| 棋盘标记 | — | "X" / "O"，**不是** "✕" "○" |
| 房间 banner 游戏图标 | 静态 SVG / CSS 形状 | "🎲" "🃏" "⚡" |

**为什么不用**：
1. 各操作系统渲染不同（Apple/Google/Microsoft 都有自家 emoji 字体）
2. 字号/对比度不可控，深色背景下很多 emoji 几乎看不见
3. 色盲用户友好度差
4. 国际化时大量翻译成本

---

## 6. 交互模式

### 6.1 反馈链

```
用户操作 → 即时反馈 → 后端响应 → 更新 UI → 完成
                ↓             ↓
              按钮文案变    Toast 提示
              "加入中…"    成功 / 失败
                ↓
              disabled=true
              （防止重复点击）
```

任何 await 调用都包在 try/catch，失败时：
1. Toast 错误信息（来自后端的 `error.message`）
2. 按钮文案 + 状态恢复
3. UI 回到操作前状态

### 6.2 破坏性操作确认

V1 没有真正的删除操作。但**"离开房间"、"再来一局"等**需要二次确认的场景：
- 用 inline 确认（按钮变 "确认离开？" → 再点一次确认）
- **不**用 `confirm()` / `alert()`
- 5 秒未确认自动回退

### 6.3 自动轮询

房间页 2s 一次 `GET /api/rooms/:id`：
- diff 旧 cache，玩家/状态变化 → Toast
- 网络失败 → 右上角 dot 变红，**不**刷 UI
- 用户离开房间 → `clearInterval`

**未来**：≥10 人时换 SSE / WebSocket 推送。

### 6.4 路由

URL hash 路由（无服务端 router）：

```
#login        登录
#register     注册
#lobby        房间列表
#room/<id>    房间详情
#game/<id>    游戏进行中
```

正则：`/^#(login|register|lobby|room|game)(?:\/(\d+))?$/`（修复 `m[1]` 捕获组问题）。

---

## 7. 可达性（A11y）

### 7.1 键盘

| 操作 | 键 |
|---|---|
| 切换焦点 | `Tab` / `Shift+Tab` |
| 触发按钮 | `Enter` / `Space` |
| 关闭模态 | `Esc` |
| 复制房间号 | 点击按钮即可（无需快捷键） |

### 7.2 焦点可见

```css
:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
}
button:focus-visible, input:focus-visible, select:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
}
```

**禁止**：`outline: none` 不带 fallback。

### 7.3 ARIA

| 元素 | 属性 |
|---|---|
| 状态徽章 | `aria-label="房间状态：等待中"` |
| 实时轮询指示 | `aria-live="polite"`，text "同步中" / "同步失败" |
| 加载中按钮 | `aria-busy="true"` |
| Toast | `role="status"` + `aria-live="polite"` |

### 7.4 颜色对比

任何彩色文字 / 背景组合都要测对比度。用 https://webaim.org/resources/contrastchecker/ 验证 ≥ 4.5:1。

---

## 8. 响应式

V1 桌面优先，但保证 ≥ 360px 可用：

```css
main { max-width: 720px; margin: 0 auto; padding: 0 16px; }
.seat-grid { grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); }
```

| 断点 | 宽度 | 适配 |
|---|---|---|
| 移动 | < 640px | 单列、按钮全宽、banner 简化 |
| 桌面 | ≥ 640px | 双列 seat-grid、卡片宽度固定 |

---

## 9. 文件结构

```
crates/lobby/static/
├── index.html        # 单页入口，挂载 #app
├── style.css         # 设计令牌 + 组件样式（按章节分组）
└── app.js            # IIFE，路由 + 状态 + 视图

style.css 分区：
  1. 设计令牌（:root）
  2. reset + 全局
  3. 布局（header / main / .row / .spacer）
  4. 表单（label / input / button）
  5. 卡片（.card / 嵌套）
  6. 状态（.status-tag / .poll-dot）
  7. 房间页（.room-banner / .seat-grid / .actions-buttons / .instance-card）
  8. 游戏页（.board / .cell / .event-log）
  9. Toast
  10. 响应式（@media）

app.js 分区：
  1. state
  2. helpers（$, el, escapeHtml, toast, api, apiGet）
  3. PoW（solveCaptcha + pickSha256 + jsSha256）
  4. auth（renderAuth, handleLogin, handleRegister）
  5. lobby（renderLobby, renderRoomList, createRoom）
  6. room（renderRoom, renderRoomDetail, renderRoomActions, pollRoom）
  7. game（renderGame, connectGame, onCellCell, handleGameMessage）
  8. router（render）
```

---

## 10. 重构 Checklist（按此文档落地的 TODO）

> 当前实现与本文档的偏差。按优先级排序。

| 优先级 | 项 | 现状 | 目标 |
|---|---|---|---|
| P0 | 棋盘标记 | `"✕"` `"○"` | `"X"` `"O"`（CSS 加粗 + 着色） |
| P0 | 复制按钮 | `"📋"` | SVG 或文字 "复制" |
| P0 | 实例状态 | `"🎮 游戏中"` `"🏁 已结束"` | `"进行中"` `"已结束"`（仅文字） |
| P0 | 房间 banner 游戏图标 | `"✕"` | CSS/SVG 形状 |
| P1 | toast 错误提示 | 通用 message | `aria-live="polite"` |
| P1 | 按钮 focus 样式 | 缺失 | `outline: var(--accent)` |
| P1 | 加载状态 | "加载中…" | 骨架屏（room-banner 占位） |
| P1 | 空状态 | 部分缺失 | 房间列表空时给主操作按钮 |
| P2 | 输入框错误提示 | toast | 行内红字 |
| P2 | 确认破坏性操作 | 直接离开 | inline 二次确认 |
| P2 | 响应式断点 | 720px max | 360px 移动适配 |
| P3 | 暗黑 / 亮色双主题 | 仅暗色 | 预留 `prefers-color-scheme` |

---

## 11. 引用

- WCAG 2.1 AA：https://www.w3.org/WAI/WCAG21/quickref/
- Apple HIG（macOS）：https://developer.apple.com/design/human-interface-guidelines/
- Material Design 3（按钮/状态）：https://m3.material.io/
- Ant Design 设计原则：https://ant.design/docs/spec/introduce
- Tailwind CSS 命名约定：https://tailwindcss.com/docs/customizing-spacing

---

**版本**：0.1 ｜ 最近更新：2026-08-10
**维护**：前端 owner 审 PR 时按 checklist 比对；任何视觉调整必须先更新本文档再改代码。