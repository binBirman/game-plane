# 部署指南

适用：在已安装 Rust + musl 工具链的 Linux 服务器上，从零部署（或覆盖旧版）lobby + tictactoe + take_your_position。

预计耗时：5 分钟。

---

## 0. 准备（一次性）

```bash
# Rust toolchain（如未装）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"

# musl target + 工具链（Debian/Ubuntu；Alpine 用 apk add musl-dev gcc）
rustup target add x86_64-unknown-linux-musl
sudo apt update && sudo apt install -y musl-tools
```

---

## 1. 构建（在你的开发机或服务器上）

```bash
cd ~/dev/game-plane-main
VERSION=0.2.0 bash build.sh
```

产物：`dist/lobby-0.2.0.tar.gz`（约 5.7 MB，含 3 个 musl 静态二进制 + 配置）。

**如果 build.sh 报错**：

| 报错 | 修 |
|---|---|
| `linker 'musl-gcc' not found` | `sudo apt install musl-tools` |
| `could not find target x86_64-unknown-linux-musl` | `rustup target add x86_64-unknown-linux-musl` |
| 出来的 binary 报 GLIBC 版本错误 | 你没走 musl target。重新 `bash build.sh`，确认输出含 `target/x86_64-unknown-linux-musl/release/` |
| Cargo 缓存污染导致奇怪错误 | `cargo clean` 后再 `bash build.sh` |

---

## 2. 上传 + 解压 + 安装

```bash
# 上传包（从你的开发机）
scp dist/lobby-0.2.0.tar.gz user@server:

# 上服务器
ssh user@server
cd ~
tar xzf lobby-0.2.0.tar.gz
cd lobby-0.2.0
ls -la     # 应该看到 lobby / tictactoe / take_your_position + games.toml + install.sh 等
```

**确认文件齐全**：

```bash
[[ -x lobby && -x tictactoe && -x take_your_position && -f games.toml ]] \
  && echo "✓ package complete" \
  || echo "✗ package incomplete — re-extract or rebuild"
```

**安装**：

```bash
sudo ./install.sh
```

`install.sh` 会做：

1. 创建 `lobby` 系统用户（已存在则跳过）
2. 建 `/var/lib/lobby`、`/var/log/lobby`、`/etc/lobby`
3. 拷 `lobby` / `tictactoe` / `take_your_position` → `/usr/local/bin/`
4. 拷 `lobby.env.example` → `/etc/lobby/lobby.env`（仅首次）
5. 拷 `games.toml` → `/etc/lobby/games.toml`（仅首次；想强制覆盖加 `--force-games-toml`）
6. 装 systemd unit + `enable`
7. **自动 restart** lobby.service（如果已在跑）
8. 校验文件齐全

---

## 3. 配置（仅首次）

```bash
sudo nano /etc/lobby/lobby.env
```

重点确认 / 修改：

| 变量 | 默认 | 含义 |
|---|---|---|
| `LOBBY_BIND` | `0.0.0.0:8192` | 监听地址。要换端口（如 8080）就改这里 |
| `LOBBY_PUBLIC_HOST` | 跟 `LOBBY_BIND` 同 | 返回给客户端的 `ws_url` 用。如果走 nginx 反代，改成对外域名 |
| `LOBBY_PUBLIC_PORT` | 同 bind port | 同上 |
| `LOBBY_GAMES_TOML` | 注释掉的 `/etc/lobby/games.toml` | **首次部署要打开**这行，否则只用单一 tictactoe |

最简配置（直接覆盖默认）：

```bash
sudo tee /etc/lobby/lobby.env >/dev/null <<'EOF'
LOBBY_BIND=0.0.0.0:8192
LOBBY_PUBLIC_HOST=your.domain.com
LOBBY_PUBLIC_PORT=443
LOBBY_GAMES_TOML=/etc/lobby/games.toml
LOBBY_LOG_FORMAT=json
LOBBY_LOG_FILE_DIR=/var/log/lobby
RUST_LOG=info,lobby::http=debug
EOF
sudo systemctl restart lobby
```

---

## 4. 验证

```bash
# 服务在跑？
systemctl is-active lobby
# → active

# 监听端口？
ss -tlnp | grep -E ':8192|:8080'   # 看你配的端口
# → LISTEN  ... users:(("lobby",pid=...,fd=...))

# 三个 game 都注册了？
curl -s http://127.0.0.1:8192/api/games | python3 -m json.tool
# 应该看到 "take_your_position" 和 "tictactoe" 都在

# 烟测（25 个 REST + WS roundtrip）
cd ~/dev/game-plane-main    # 在开发机上跑
HOST=your.server PORT=8192 bash tools/test.sh
```

---

## 5. 防火墙 / nginx（按需）

### 直接暴露 8192

```bash
sudo ufw allow 8192/tcp
```

### nginx 反代（推荐）

`install.sh` 已经把 `nginx.conf.example` 装到了 `/etc/nginx/sites-available/lobby.conf`（如果有 nginx）。启用：

```bash
sudo nano /etc/nginx/sites-available/lobby.conf
# 改 server_name 为你的域名

sudo ln -s /etc/nginx/sites-available/lobby.conf /etc/nginx/sites-enabled/lobby
sudo nginx -t && sudo systemctl reload nginx
```

确认 `/etc/lobby/lobby.env` 里 `LOBBY_PUBLIC_HOST` / `LOBBY_PUBLIC_PORT` 已设为 nginx 监听的地址。

---

## 6. 升级（覆盖旧版）

把上面的 `dist/lobby-0.2.0.tar.gz` 拷过去重新走第 2 步即可。`install.sh` 会自动覆盖 binary、重启服务、保留 `/var/lib/lobby/lobby.db`（用户数据）。

**注意**：如果旧版 `games.toml` 里只注册了 tictactoe，加 `--force-games-toml`：

```bash
sudo ./install.sh --force-games-toml
```

---

## 故障排查

### "服务起不来"

```bash
systemctl status lobby
journalctl -u lobby -n 50 --no-pager
```

常见原因：

| 日志关键词 | 原因 | 修 |
|---|---|---|
| `Address already in use` | 8192 被别的进程占 | `ss -tlnp \| grep 8192` 找占用者，或换 `LOBBY_BIND=0.0.0.0:8919` |
| `game binary NOT FOUND` | `games.toml` 里的路径不对 | `ls -la /usr/local/bin/take_your_position`；不存在就 `sudo ./install.sh` 重装 |
| `Permission denied` 写 `/var/lib/lobby/` | 文件 owner 不对 | `sudo chown -R lobby:lobby /var/lib/lobby` |
| `database is locked` | 多个进程抢同一个 db | 确认 `lobby.service` 没被启动两次；`pkill lobby && systemctl restart lobby` |

### "服务在跑但 /api/games 只返回 tictactoe"

Lobby 启动时读了 `LOBBY_GAMES_TOML` 但没找到文件 / 解析失败 / 路径写错。

```bash
# 看启动时打了什么
journalctl -u lobby | grep -i "game registry\|games.toml"

# 手动跑 lobby 看错误
sudo -u lobby /usr/local/bin/lobby
```

修：

```bash
# 确认 env 里 LOBBY_GAMES_TOML 有设
grep LOBBY_GAMES_TOML /etc/lobby/lobby.env
# → LOBBY_GAMES_TOML=/etc/lobby/games.toml

# 确认文件合法 TOML 且 binary 路径可执行
sudo -u lobby python3 -c "import tomllib; print(tomllib.load(open('/etc/lobby/games.toml','rb')))"
ls -l /usr/local/bin/take_your_position /usr/local/bin/tictactoe
```

### "升级后行为没变"

systemd 不会因 binary 文件变更而自动重启。需要：

```bash
sudo systemctl restart lobby
# 然后再 curl /api/games
```

### "客户端连不上 / WebSocket 报 404"

最常见：`LOBBY_PUBLIC_HOST` 配错，导致 `ws_url` 返回的是 `127.0.0.1` 但客户端在另一台机器上。

```bash
# 看返回的 ws_url
curl -s -X POST -H "Authorization: Bearer <token>" \
     -H 'Content-Type: application/json' \
     -d '{"game_type":"take_your_position"}' \
     http://127.0.0.1:8192/api/rooms
# 应该有 ws_url 字段，看里面写的 host:port 对不对
```

### "游戏里出牌点了没反应 / game_error 一直弹"

```bash
# 看 game 进程日志（lobby 转发的 stderr）
journalctl -u lobby -n 100 --no-pager | grep -i 'take_your\|game_error'
```

如果看到大量 `INVALID_SESSION`：token 过期 / 重登过。重新登录就行。

### "端口被旧进程占"

```bash
ss -tlnp | grep 8192
# 看哪个 pid 占的
sudo kill <pid>     # 或者：
sudo systemctl restart lobby
```

---

## 一键脚本

如果你懒：

```bash
# 在解压后的目录里
sudo ./install.sh --force-games-toml
sudo systemctl restart lobby
curl -s http://127.0.0.1:8192/api/games
```

完。
