# lobby-server 安装包

Linux x86_64 musl 静态安装。详细文档请看仓库根目录的 [`README.md`](../README.md)。

## 文件

| 文件 | 用途 |
|---|---|
| `lobby` | 静态二进制（musl，无运行时依赖） |
| `tictactoe` | 默认游戏二进制 |
| `lobby.service` | systemd unit |
| `lobby.env.example` | 环境变量模板 |
| `games.toml` | 游戏注册表 |
| `nginx.conf.example` | nginx 反代参考配置 |
| `RUNBOOK.md` | 运维手册（systemd、nginx、certbot、备份、升级、添加游戏） |
| `install.sh` | 提权安装器 |
| `uninstall.sh` | 提权卸载器 |

## 安装

```bash
sudo ./install.sh
sudo systemctl edit lobby    # 可选：drop-in 覆盖
sudo systemctl start lobby
sudo systemctl status lobby
```

安装器会：

- 创建 system user `lobby`
- 拷二进制到 `/usr/local/bin/{lobby,tictactoe}`
- 创建 `/var/lib/lobby`（SQLite 数据）+ `/var/log/lobby`（滚动日志）
- 从 `lobby.env.example` 写入 `/etc/lobby/lobby.env`（不会覆盖已有）
- 拷 `games.toml` 到 `/etc/lobby/games.toml`（不会覆盖已有）
- 注册并启用 systemd 服务（**不自动启动**）
- 若 nginx 已装：丢 `nginx.conf.example` 到 `/etc/nginx/sites-available/lobby`（不自动启用）

## 卸载

```bash
sudo ./uninstall.sh
```

保留 `/var/lib/lobby` 和 `/var/log/lobby`，需要时手动清理：

```bash
sudo rm -rf /var/lib/lobby /var/log/lobby
sudo userdel lobby
```

## 防火墙

V1 反代：客户端只连 Lobby 单一公开端口；game 进程绑 `127.0.0.1`，对外不可见——**不要再额外开 game 端口**。