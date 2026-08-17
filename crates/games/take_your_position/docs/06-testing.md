# TYP 测试

> 总体测试策略见 `/docs/TYP_HANDOVER.md#5`。本文件专注 TYP 的具体测试方法。

---

## 1. 单元测试（`cargo test`）

跑：

```bash
cargo test -p take_your_position           # 全部
cargo test -p take_your_position card::    # card::tests
cargo test -p take_your_position state::   # state::tests
```

### 1.1 `card::tests`

| 测试 | 验证 |
|---|---|
| `suit_ranking_is_spade_heart_club_diamond()` | `cmp_table` 在等 rank 时的花色顺序：黑桃 > 红桃 > 梅花 > 方块 |
| `cmp_table_same_rank_uses_suit_order()` | 同 rank 两张牌的 `cmp_table` 对称性 |

### 1.2 `state::tests`

| 测试 | 验证 |
|---|---|
| `start_thinking_only_marks_current_player()` | `PriorPrediction` 只 `current_player` 进 thinking，其余 4 个 `thinking_since = None` |
| `start_thinking_resets_stale_clock_on_next_player()` | 切下一玩家时 `thinking_since` 重置（不继承上一玩家的 elapsed）|
| `play_phase_marks_all_uncommitted_players_thinking()` | `Play` 阶段 5 个玩家同时 thinking，commit 之后只有该玩家 `thinking_since = None` |
| `start_thinking_clears_inactive_players()` | 任何 inactive 玩家 `thinking_since = None` |

### 1.3 写新测试

`state.rs` 末尾的 `mod tests` 加：

```rust
#[test]
fn your_test_name() {
    let mut s = build_test_state();
    // 操作 ...
    assert_eq!(s.field, expected);
}
```

辅助函数 `build_test_state()` 建一个 5 人 `PriorPrediction` 阶段的状态。每个测试是独立的，互不干扰。

---

## 2. 端到端（lobby 烟测）

`tools/test.sh` 跑 25 个烟测（包括 TYP 完整流程）。本地或云端：

```bash
PORT=8192 bash tools/test.sh
```

期望：

```
All 22 tests passed
```

22 个是当前通过的数量（剩 3 个是陈旧密码测试，已删）。

---

## 3. 端到端（直接跑 TYP 进程，跳过 lobby）

适合调试 TYP 后端逻辑（看 `game_log!` JSON Lines）。

### 3.1 启动 TYP 子进程

```bash
TYP=$(pwd)/target/x86_64-unknown-linux-musl/release/take_your_position
PORT=$(python3 -c "import socket;s=socket.socket();s.bind(('',0));print(s.getsockname()[1]);s.close()")

cat > /tmp/typ_init.txt <<JSON
{"room_id":99,"game_type":"take_your_position",
 "listen":"127.0.0.1:$PORT",
 "players":[{"uid":1,"sessions":["tok1"]},{"uid":2,"sessions":["tok2"]},
           {"uid":3,"sessions":["tok3"]},{"uid":4,"sessions":["tok4"]},
           {"uid":5,"sessions":["tok5"]}],
 "config":{"timer_preset":"300+0"}}
JSON

( cat /tmp/typ_init.txt ; echo '{"event":"start"}' ; sleep 10 ) | \
  $TYP 2>/tmp/typ_stderr.log >/tmp/typ_stdout.log &
```

### 3.2 用 Python raw-WS 跑流程

```python
import base64, json, os, socket, struct

def ws_handshake(s, path):
    key = base64.b64encode(os.urandom(16)).decode()
    s.sendall((
        f"GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\n"
        f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    ).encode())
    buf = b""
    while b"\r\n\r\n" not in buf:
        c = s.recv(4096)
        if not c: break
        buf += c
    if b"101" not in buf: raise RuntimeError("handshake failed")

def ws_send_text(s, text):
    p = text.encode()
    h = bytearray([0x81, 0x80 | len(p)]) if len(p) < 126 else \
        bytearray([0x81, 0x80 | 126]) + bytearray(struct.pack(">H", len(p)))
    m = os.urandom(4)
    s.sendall(bytes(h) + m + bytes(b ^ m[i % 4] for i, b in enumerate(p)))

def ws_recv(s, timeout=0.5):
    s.settimeout(timeout)
    try: hdr = s.recv(2)
    except socket.timeout: return None
    if len(hdr) < 2: return None
    op = hdr[0] & 0x0f
    ln = hdr[1] & 0x7f
    if ln == 126: ln = struct.unpack(">H", s.recv(2))[0]
    elif ln == 127: ln = struct.unpack(">Q", s.recv(8))[0]
    pay = b""
    while len(pay) < ln:
        c = s.recv(ln - len(pay))
        if not c: return None
        pay += c
    return json.loads(pay.decode()) if op == 0x1 else None

# 5 玩家连 + login
for uid in range(1, 6):
    ...
```

参考脚本：`/tmp/check.py` 和 `/tmp/check_timing.py`（已经存在，可参考）。

### 3.3 查看 `game_log!` 输出

TYP 把所有 `game_log!` 写 stderr（JSON Lines），启动自己的 `tracing::info!` 写 stderr（普通文本，**fallback**）：

```bash
# 全部 stderr：
cat /tmp/typ_stderr.log | head -40

# 只看 game_log!（JSON Lines，进 lobby 后会被 re-emit 带 target）：
cat /tmp/typ_stderr.log | python3 -c "
import json, sys
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try:
        d = json.loads(line)
        print(f'[{d[\"level\"]:5s}] {d[\"target\"]:35s}  {d[\"message\"]}', end='')
        f = d.get('fields', {})
        if f: print(f'  {f}', end='')
        print()
    except: pass
"
```

期望看到：
- `start_thinking` 行（每次 phase 切换、每次切下一玩家）
- `apply_posterior` / `apply_posterior_draft` 接受/拒绝详情
- `action processed` / `action rejected` 摘要
- `round reveal: scoring` / `round reveal: next round`

---

## 4. 端到端（完整跑 lobby + 5 玩家）

云端 8.148.5.15 早已部署，浏览器 5 个标签（或 5 个 incognito）→ 创建房间 → 房主 start → 5 个 join 房间 → 房主点「开始」→ 5 个浏览器自动跳到 `#game/<id>`。

手动验证：

| 阶段 | 验证 |
|---|---|
| 等待 5 人都登录 | `phase` 为 `prior_prediction` 才能 predict；当前玩家的座位高亮 |
| PriorPrediction | 提交 predict 后锁定；下一玩家座位高亮 |
| 全员 predict 完 | 5 人显示自己手牌（5 张，1 张♠）|
| Play | 5 人同时提交；提交完后再揭示 |
| PosteriorPrediction | 首玩家显示 5×5 网格；后验 3 秒 flash 高亮 |
| 5 轮结束 | `showGameOver` 显示总分排名 |

抓云端日志：

```bash
ssh root@8.148.5.15
journalctl -u lobby -f -n 0 | grep -E "take_your_position|round |RoundResult|PhaseChanged"
```

---

## 5. 调试 checklist

| 症状 | 查什么 |
|---|---|
| `start_thinking` 日志不对 | `phase=... active_players=[N]` 应该每个 phase 只有预期数量的人在 |
| 计时扣错 | `settle_action` 后 `time_a_ms / time_b_ms` 在 snapshot 里的值 |
| 结算页秒弹 | `revealActiveAt` 是否触发（前端 `window.revealActiveAt` 看 console）|
| 后验排名不显示 | `s.posterior.find(([, , committed]) => committed)` 找不找得到 |
| 5 轮结束后不结束 | `tick()` 1s 跑没跑；`s.finish_round` 是否被 `advance_phase` 调到 |
| 牌揭示错 | `Card::cmp_table` 单元测试 + 浏览器里看桌面的明牌顺序 |
