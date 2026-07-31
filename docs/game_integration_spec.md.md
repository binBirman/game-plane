# Lobby Server Architecture & Game Integration Protocol

Version: 0.1

## 1. 设计目标

实现一个轻量级多人游戏大厅服务器。

核心原则：

> Lobby 管理玩家和房间，Game 管理游戏。

---

## 2. 系统架构

```
             Client
                |
      HTTP / WebSocket
                |
                v

        +---------------+
        |    Lobby      |
        |---------------|
        | 用户认证      |
        | 房间管理      |
        | 实例管理      |
        | 游戏启动      |
        +-------+-------+
                |
          Process Spawn
                |
    +-----------+-----------+
    |                       |
    v                       v
+-------------+       +-------------+
| Game Server |       | Game Server |
|   Room A    |       |   Room B    |
+-------------+       +-------------+

```

---

## 3. 职责划分

### Lobby Server

负责：

- 用户注册
- 用户登录
- Session 管理
- 创建房间
- 加入房间
- 启动游戏实例
- 管理游戏生命周期
- 回收游戏实例

不负责：

- 游戏规则
- 游戏状态
- 游戏同步
- 胜负判断

---

### Game Server

负责：

- 游戏逻辑
- 玩家同步
- 游戏状态
- 回合控制
- 断线重连
- 游戏结束

不负责：

- 用户系统
- 房间系统
- 数据库访问

---

## 4. 核心数据模型

### User

```
id

username

password_hash

nickname
```

---

### Room

逻辑房间：

```
room_id

game_type

host_uid

status

players
```

状态：

```
Waiting

Starting

Running

Finished

Destroyed
```

---

### GameInstance

游戏运行实例：

```

instance_id

room_id

pid

port

status

start_time

```

Room 与 GameInstance 分离。

---

## 5. 游戏启动流程

```
用户登录
↓
创建房间
↓
玩家加入
↓
满足开始条件
↓
Lobby 创建 GameInstance
↓
启动 Game 进程
↓
Game 初始化
↓
Game Ready
↓
玩家连接 Game
↓
开始游戏
```

---

## 6. 通信模型

## Client -> Lobby

HTTP。

用于：

- 登录
- 注册
- 创建房间
- 加入房间
- 查询房间

---

## Client -> Game

WebSocket。

用于：

- 游戏操作
- 游戏同步
- 状态更新

---

## Lobby -> Game

进程通信。

方式：

- stdin/stdout
- pipe

用于：

- 初始化
- 生命周期通知
- 心跳

---

## 7. 认证模型

流程：

```
用户登录
↓
Lobby 验证账号
↓
生成 Session Token
↓
启动 Game 时传入
↓
Game 验证 Session
```

Game 不保存账号密码。

Game 不依赖 Lobby 的 JWT。

---

## 8. Game 接入规范

### 8.1 游戏启动

Game 必须作为独立进程运行。

Lobby 启动时传入初始化数据：

```json
{
    "room_id":1001,
    "game_type":"poker",
    "listen":"0.0.0.0:41001",
    "players":[
        {
            "uid":1,
            "session":"xxxx"
        }
    ]
}
```

字段：

| 字段 | 说明 |
| --------- | ---- |
| room_id | 房间ID |
| game_type | 游戏类型 |
| listen | 监听地址 |
| players | 玩家列表 |

---

### 8.2 玩家连接

客户端连接：

WebSocket:

首次发送：

```json
{
    "type":"login",
    "uid":1,
    "session":"xxxx"
}
```

Game：

```
验证Session
↓
匹配初始化玩家列表
↓
允许进入
```

---

## 8.3 生命周期事件

Game 通过 stdout 通知 Lobby。

格式：

```json
{
    "event":"ready"
}
```

事件：

| 事件       | 说明    |
| -------- | ----- |
| ready    | 初始化完成 |
| running  | 游戏开始  |
| finished | 游戏结束  |
| shutdown | 准备退出  |

---

## 8.4 心跳

Game 定时发送：

```json
{
    "event":"heartbeat"
}
```

Lobby：

```
超过15秒无心跳
↓
认为实例异常
```

---

## 8.5 游戏状态

Game 自主管理全部游戏状态。

包括：

- 棋盘
- 手牌
- 回合
- 玩家状态

Lobby 不保存游戏状态。

---

## 9. 断线重连协议

原则：Player != Connection

Player：

```
uid

state
```

Connection：

```
socket
```

---

### 断线

```
Socket关闭
↓
Player保留
```

---

### 重连

客户端：

```json
{
    "type":"reconnect",
    "uid":1,
    "session":"xxxx"
}
```

Game：

```
验证Session
↓
查找Player
↓
绑定新Connection
↓
发送Snapshot
```

---

## 10. Snapshot

重连成功后：

Game 发送完整状态。

示例：

```json
{
    "type":"snapshot",
    "state":{}
}
```

客户端根据 Snapshot 恢复。

---

## 11. 游戏退出流程

Game

```
发送 finished
↓
发送 shutdown
↓
退出进程
```

Lobby

```
删除 GameInstance
↓
释放端口
↓
更新 Room 状态
```

---

## 12. V1 范围

实现：

- 用户注册
- 用户登录
- Session认证
- 房间管理
- 游戏启动
- 动态端口分配
- WebSocket连接
- 生命周期管理
- 心跳检测
- 断线重连

---

## 13. V1 不实现

- 分布式部署
- Gateway
- 插件系统
- Docker调度
- 匹配系统
- 好友系统
- 排行榜
- 游戏录像
- 多服务器同步
