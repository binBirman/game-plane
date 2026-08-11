# 账号注册与登录（V1）

> 适用版本：`lobby` v0.1.x（`crates/lobby/src/http/user.rs`、`crates/lobby/src/auth/`）。完整错误码表见 `docs/protocol_spec.md` §3。

## 1. 设计原则（已采纳）

| # | 原则 | 含义 |
|---|---|---|
| 1 | 密码长度 ≥ 8 | 之前的"≥9 + 数字/字母/特殊字符"过严；放宽为单一长度阈值，复杂度由 argon2 兜底。 |
| 2 | 显示/隐藏密码 | 每个密码字段右边有"显示/隐藏"切换按钮，纯文字标签（按 `frontend-design.md` §5，不使用 emoji）。 |
| 3 | 注册时确认密码 | 注册表单两个密码框必须一致，前端校验；后端只接收一份 `password`，把"输错"挡在客户端。 |
| 4 | 完整流程含 PoW | 注册、登录都强制 PoW（人机验证）。这是 V1 防滥用第一道墙。 |

## 2. 总体流程

```
                        ┌─────────────┐
   浏览器                │   lobby     │                SQLite
   ─────                 │   HTTP      │                ──────
                        │  :8192      │
   GET /                │             │
     └────── index.html + app.js + style.css ───────►│
                                                               
   点"注册" tab ─────►►│             │
                        │             │
   POST /api/captcha/challenge ◄──────┤             │
   ├──────── {challenge, difficulty, ttl} ────────► │
                        │             │
   浏览器跑 SHA-256 PoW  │             │
   (challenge:nonce 哈希前导零 ≥ difficulty 位)     │
                        │             │
   POST /api/register {username, password, nickname, captcha}
   ├────────────────────────────────────────────►│
   │ rate-limit (per IP, 60s 滑窗)                │
   │ verify captcha                                │
   │ validate password (length ≥ 8)               │
   │ hash with argon2 (random salt)                │
   │ INSERT INTO users                              │
   ◄──────────── {uid} ───────────────────────────│
                                                               
   切到"登录" tab       │             │
   POST /api/captcha/challenge ◄──────┤             
   POST /api/login {username, password, captcha}     │
   ├────────────────────────────────────────────►│
   │ rate-limit                                    │
   │ verify captcha                                │
   │ SELECT users WHERE username = ?              │
   │ argon2.verify(password, hash)                 │
   │ generate token (32 字节随机 hex)              │
   │ INSERT INTO sessions(token, user_id, expires)│
   ◄───── {uid, token} ────────────────────────────│
                                                               
   token → localStorage("lobby_token")             │
   Authorization: Bearer <token> ←── 后续所有请求带这个头
```

## 3. 密码策略

| 维度 | 规则 | 代码 |
|---|---|---|
| 长度 | ≥ 8 字符（按 `chars().count()` 计；Unicode 字符算 1） | `auth/password.rs:30` |
| 复杂度 | **无要求**（之前要求数字+字母+特殊字符；已移除） | — |
| 字符集 | 任意（密码以 UTF-8 字节传入 argon2） | — |
| 哈希 | `argon2` 默认参数 + 16 字节随机 salt；存 PHC 字符串 | `auth/password.rs:6` |
| 校验 | `argon2.verify_password`；失败返回 `INVALID_CREDENTIALS`（不区分"用户不存在"和"密码错"） | `auth/password.rs:16`, `http/user.rs:142` |

**测试**（`auth/password.rs:38`）：`accepts_eight_or_more`（"abcdefgh" / "12345678" / "Test1234" 都过）；`rejects_below_eight`（空串 / 1 字符 / 7 字符不过）。

> **不存明文**：DB 里只有 `password_hash`（PHC 字符串：`$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>`）。`hash_password` 永远不返回明文；`verify_password` 也只返 bool。
>
> **不区分"用户名不存在"和"密码错"**：登录路径两种情况都返 `401 INVALID_CREDENTIALS`，错误文案统一为"invalid credentials"。这是有意为之，避免 username enumeration。

## 4. 注册

`POST /api/register`（`http/user.rs:33`）

请求体：
```json
{
  "username": "3-20 chars",
  "password": ">= 8 chars",
  "nickname": "non-empty",
  "captcha": { "challenge": "<hex>", "nonce": "<string>" }
}
```

后端校验链：
1. **IP 速率限制**（`state.rl_register`，默认 10/60s 滑窗，可通过 `LOBBY_RL_REGISTER_PER_MIN` 调）→ 命中 → `429 RATE_LIMITED`
2. `captcha` 字段缺失 → `400 CAPTCHA_REQUIRED`
3. PoW 校验失败 → `400 CAPTCHA_INVALID`
4. `username` / `password` / `nickname` 任一为空 → `400 INVALID_PARAMS`
5. `validate_strength`（长度 ≥ 8）失败 → `400 WEAK_PASSWORD`
6. argon2 hash
7. `INSERT INTO users` —— 唯一冲突（username UNIQUE） → `409 USERNAME_TAKEN`
8. 成功 → `200 {"uid": <int>}`

### 4.1 前端注册表单（`static/app.js`）

```
[用户名] [__________________]   hint: 3-20 字符
[密码]   [______________][显示]
                                    hint: 至少 8 位
[确认密码] [______________][显示]
                                    hint: 再次输入密码，必须一致
[昵称]   [__________________]
[创建账号]
```

- **密码/确认密码字段**：用 `passwordInput(name, autocomplete)` 工厂（`app.js:259`）生成。外面包一层 `<div class="pwd-wrap">`，里面 `<input type="password">` + `<button class="pwd-toggle">`。点击切换 `type` 在 `"password"` ↔ `"text"` 之间，按钮文案 "显示" / "隐藏" 同步切换，`aria-pressed` 同步。
- **客户端校验**（`app.js:309` 的 `clientValidate`）：
  - `username.length ∈ [3, 20]`
  - `password.length ≥ 8`（不再校验复杂度）
  - **`password === password_confirm`** —— 不一致就在 `password_confirm` 字段下显示"两次输入的密码不一致"
  - `nickname` 非空
- **autocomplete**：`username` → `username`，密码字段 → `new-password`（注册场景）
- 失败时不发请求；服务端校验失败（USERNAME_TAKEN 等）通过 `mapBackendErrorToField` 把错误归到对应字段下（`app.js:330`）。

### 4.2 注册成功后

`app.js:387`：toast "注册成功"，`form.reset()`，切回 "登录" tab 让用户立即登录。

## 5. 登录

`POST /api/login`（`http/user.rs:107`）

请求体：
```json
{
  "username": "...",
  "password": "...",
  "captcha": { "challenge": "<hex>", "nonce": "<string>" }
}
```

后端校验链：
1. **IP 速率限制**（`state.rl_login`，默认 20/60s，可通过 `LOBBY_RL_LOGIN_PER_MIN` 调）→ 命中 → `429 RATE_LIMITED`
2. `captcha` 缺失 → `400 CAPTCHA_REQUIRED`；PoW 失败 → `400 CAPTCHA_INVALID`
3. `username` / `password` 任一为空 → `400 INVALID_PARAMS`
4. `SELECT users WHERE username = ?` —— 找不到 → `401 INVALID_CREDENTIALS`（不告诉客户端"用户不存在"）
5. `argon2.verify_password` —— 失败 → `401 INVALID_CREDENTIALS`
6. `generate_token()` —— 32 字节随机 hex
7. `INSERT INTO sessions(token, user_id, expires_at)`
8. 成功 → `200 {"uid": <int>, "token": "<hex>"}`

> **登录不要求 captcha 之外的二次校验**：V1 设计上 PoW + argon2 + 速率限制 三件套足够；后续要加 TOTP / 邮件验证 / 风控再说。

### 5.1 前端登录表单

```
[用户名] [__________________]
[密码]   [______________][显示]
[登录]
```

- 没有"确认密码"（登录不是创建账号）
- 客户端校验只查"非空"
- 成功后 `state.token = r.token; state.uid = r.uid; state.nickname = username`；`localStorage.setItem("lobby_token", ...)` 等
- 跳到 `#lobby`

## 6. Session 管理

### 6.1 生成

`auth/session.rs::generate_token`：32 字节 OS 随机数 → hex 字符串（64 字符）。**强度**：128 bit，等同 UUIDv4 随机性。

```
expires_at = datetime('now', '+' || session_ttl_days || ' days')
```

`LOBBY_SESSION_TTL_DAYS` 默认 7。DB schema：

```sql
CREATE TABLE sessions (
    token      TEXT PRIMARY KEY,     -- 64 hex chars
    user_id    INTEGER NOT NULL REFERENCES users(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at TEXT NOT NULL          -- "YYYY-MM-DDTHH:MM:SSZ"
);
```

### 6.2 携带方式

每个需要鉴权的 HTTP 请求带：

```
Authorization: Bearer <token>
```

提取：`auth/extractor.rs` 的 `CurrentUser::from_request_parts`：
1. 缺 header 或不是 `Bearer ...` → `401 INVALID_CREDENTIALS`
2. `SELECT u.id, u.username, u.nickname, s.expires_at FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token = ?`
3. 行不存在 → `401 INVALID_CREDENTIALS`
4. `expires_at < now()` → `401 INVALID_CREDENTIALS`（懒校验，避免每请求回 GC）
5. 返 `CurrentUser { uid, username, nickname }`

> **Bearer token 不是 JWT**：纯随机串，无签名无 claim。所有权限判断都回到 DB 查 `sessions` 行 — 改密 / 踢人 = `DELETE FROM sessions WHERE token = ?` 立即生效。V1 不用 JWT 的好处。

### 6.3 过期与 GC

- **懒校验**：每个请求拉 `expires_at` 比对 now。
- **主动 GC**：`main.rs:114` 每小时一次：
  ```sql
  DELETE FROM sessions WHERE expires_at < datetime('now')
  ```
  清掉过期行，避免表膨胀。

### 6.4 多端登录

**故意允许**：同一 `user_id` 可有任意多 `sessions` 行（每次 `POST /api/login` 都 `INSERT`）。每个浏览器标签 / 设备独立持有 token。

副作用：spawn 游戏时 lobby 用 `JOIN sessions ON user_id = rp.uid` 取出该用户**所有未过期 session**，传给 game 的 `PlayerInit.sessions: Vec<String>`，让 game 接受任一 token（详见 `room_lifecycle.md`）。

> **V1 无"互踢"**：要实现"新登录踢掉旧 session"，加 `DELETE FROM sessions WHERE user_id = ? AND token != ?` 在登录成功后。但当前设计故意允许多端共存，便于多端游戏。

### 6.5 登出

**V1 无 server-side logout endpoint**。前端 `logout()`（`app.js:71`）只 `localStorage.removeItem(...)`，把 token 丢弃。下次再访问 `/api/*` 没 token 就返 401 → 客户端跳 `#login`。DB 里的 session 行等 GC 自动清理。

> 加 server-side logout 是下周期工作（V1 假定 token 等同密码，泄露了只能改密码或等过期）。

## 7. 人机验证（PoW Captcha）

### 7.1 为什么不用 reCAPTCHA

V1 选择自建 PoW（Hashcash 变体）而非 Google reCAPTCHA / hCaptcha：
- 无第三方依赖（用户可在局域网 / 离线跑）
- 隐私：不上传用户行为
- 攻击面：服务端只需 32 字节 challenge + 难度位
- 算力证明在浏览器几 ms 完成（WebCrypto.subtle），不会卡用户

### 7.2 协议

`POST /api/captcha/challenge`（`http/captcha.rs:16`）

```
→ 200 {"challenge": "<32 hex>", "difficulty": 16, "ttl_seconds": 300}
→ 429 RATE_LIMITED          （命中 rl_captcha，默认 60/60s）
```

`auth/pow.rs::issue`：16 字节 OS 随机 → 32 字符 hex；difficulty 取自 `state.pow_difficulty`（默认 16 bit 前导零，`LOBBY_POW_DIFFICULTY` 可改）。

`auth/pow.rs::verify`：客户端提交 `nonce`（任意字符串），服务端 `SHA-256(challenge + ":" + nonce)`，前导零位数 ≥ difficulty 即通过。**stateless**：服务端不存 challenge。

### 7.3 客户端求解

`app.js:115 solveCaptcha`：

```js
fetch POST /api/captcha/challenge  →  { challenge, difficulty }
for nonce in 0..N:
    SHA-256(challenge + ":" + nonce)  // WebCrypto.subtle.digest
    if leadingZeros(hash) >= difficulty: return { challenge, nonce: String(nonce) }
```

WebCrypto.subtle 只在 **Secure Context**（HTTPS / `localhost` / `127.0.0.1`）可用。远程 IP 上跑 `crypto.subtle.digest` 在大多数浏览器会失败 —— 所以 `app.js:143` 有降级：纯 JS SHA-256（`jsSha256` 函数，整个算法展开）。慢但可用。

### 7.4 失效与 TTL

`ttl_seconds=300` 在响应里告诉前端"5 分钟内有效"。服务端**不强制** —— 任何 challenge 都接受，只要 PoW 算对。这是有意的：纯 stateless，horizontal scale 不需要共享 challenge store。

### 7.5 难度与算力

| difficulty | 期望 PoW 迭代数 | 浏览器耗时（WebCrypto） |
|---|---|---|
| 16 | ~65k | 几 ms |
| 20 | ~1M | 几十 ms |
| 24 | ~16M | 几百 ms |

默认 16 已能挡住机器人（每 IP 60 次/分钟，xargs 暴力也会被速率限制挡住）。要更高防御就调 `LOBBY_POW_DIFFICULTY=20`。

## 8. 速率限制

`crates/lobby/src/ratelimit.rs`：**内存滑窗**，按客户端 IP 分桶，**非集群范围**。

| 限流器 | 默认阈值（per IP, 60s 滑窗） | env | 触发位置 |
|---|---|---|---|
| `rl_register` | 10 | `LOBBY_RL_REGISTER_PER_MIN` | `POST /api/register` 第一关 |
| `rl_login` | 20 | `LOBBY_RL_LOGIN_PER_MIN` | `POST /api/login` 第一关 |
| `rl_captcha` | 60 | `LOBBY_RL_CAPTCHA_PER_MIN` | `POST /api/captcha/challenge` |

超出 → `429 RATE_LIMITED`。限流器是 `HashMap<IpAddr, VecDeque<Instant>>`，过期的请求时间戳会被裁掉。

> **集群范围限流要做的话**：上 Redis 或类似。V1 单进程足够；横向扩展后再说。

## 9. 失败模式

| 场景 | 检测 | 行为 |
|---|---|---|
| 密码长度 < 8 | `validate_strength` | `400 WEAK_PASSWORD`，前端 `mapBackendErrorToField` 把错误归到 password 字段下 |
| 密码 ≠ 确认密码 | 前端 `clientValidate` | 前端阻断，不发请求 |
| 用户名已存在 | DB UNIQUE 冲突 | `409 USERNAME_TAKEN` |
| 用户名不存在 / 密码错 | 统一处理 | `401 INVALID_CREDENTIALS`（不区分） |
| PoW 缺失 | 服务端检查 | `400 CAPTCHA_REQUIRED` |
| PoW 算错 | `pow::verify` | `400 CAPTCHA_INVALID` |
| captcha challenge IP 速率超出 | `rl_captcha` | `429 RATE_LIMITED` |
| 注册 IP 速率超出 | `rl_register` | `429 RATE_LIMITED` |
| 登录 IP 速率超出 | `rl_login` | `429 RATE_LIMITED` |
| session 过期 | `extractor` lazy check | `401 INVALID_CREDENTIALS` → 客户端跳 `#login` |
| argon2 hash 算不动 | 内部错误 | `500 INTERNAL_ERROR`（极少见） |

## 10. API 摘要

| Method | Path | Auth | 用途 |
|---|---|---|---|
| POST | `/api/captcha/challenge` | 无 | 拿 PoW challenge |
| POST | `/api/register` | 无 | 创建账号，返 `{uid}` |
| POST | `/api/login` | 无 | 验证账号，返 `{uid, token}` |
| * | `/api/*` (其他) | Bearer | 标准鉴权 |

## 11. 关键不变量（code review checklist）

- 密码永不明文出现在 DB、日志、API 响应里
- argon2 是唯一允许的密码哈希算法；不要新增 SHA / bcrypt 等
- 登录响应不区分"用户不存在" vs "密码错"（enumeration 防护）
- 注册路径的速率限制先于任何 DB 读写
- `users.username` UNIQUE 由 DB 约束保证（不用应用层 SELECT-then-INSERT）
- `sessions.token` 是 64 hex 字符（OS 随机），不用用户输入作 token
- PoW 是 stateless，服务端只验证不算
- 登出是纯前端行为；token 泄露后只能等 `LOBBY_SESSION_TTL_DAYS` 过期

## 12. 已知弱点与下一周期

| # | 弱点 | 计划 |
|---|---|---|
| W1 | 无 server-side logout | 加 `POST /api/logout`：DELETE 自己的 session 行；前端调它再清 localStorage |
| W2 | 无"新登录踢旧设备" | 登录成功后 `DELETE FROM sessions WHERE user_id = ? AND token != ?` |
| W3 | 速率限制仅 in-memory | 集群部署时改 Redis；单进程足够 V1 |
| W4 | PoW 仅 16 bit，xargs + 算力池可破 | 高负载时可调 `LOBBY_POW_DIFFICULTY=20` 或加 IP 子网维度的二次限流 |
| W5 | 无密码找回 | 加 email + token reset 流程；需要 SMTP 配置 |
| W6 | 无"最近登录设备 / 异地提醒" | login 成功时记录 IP / UA；UI 展示 |
| W7 | nickname 可重复 | 加 UNIQUE 或 length cap |
| W8 | username 字符集未限（任意 UTF-8） | 限 `[a-zA-Z0-9_-]`，长度 3-20，避免奇异 unicode homoglyph |