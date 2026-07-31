# Lobby Server Architecture

Version: 0.1

## 1. 设计目标

实现一个轻量级多人游戏大厅服务器。

核心原则：

> Lobby 管理玩家和房间，Game 管理游戏。

Lobby 不参与任何游戏逻辑。

---

## 2. 系统结构

          Client
             |
             |
      HTTP / WebSocket
             |
             v

    +----------------+
    | Lobby Server   |
    |----------------|
    | 用户认证       |
    | 房间管理       |
    | 游戏启动       |
    | 实例管理       |
    +--------+-------+
             |
             |
        启动 Game
             |
    +--------+--------+
    |                 |
    v                 v
    GameInstanceA GameInstanceB

---

## 3. 模块职责

### Lobby Server

负责：

- 用户注册
- 用户登录
- Session管理
- 创建房间
- 加入房间
- 启动游戏实例
- 管理游戏生命周期
- 回收游戏实例

不负责：

- 游戏规则
- 游戏同步
- 游戏状态
- 胜负判断

---

### Game Server

负责：

- 游戏逻辑
- 玩家同步
- 游戏状态
- 回合控制
- 断线重连
- 游戏结束处理

不负责：

- 用户系统
- 房间管理
- 数据库访问

---

## 4. 核心对象

### User

用户信息。

id

username

password_hash

nickname

---

### Room

逻辑房间。

room_id

game_type

host_uid

status

players

状态：

Waiting

Starting

Running

Finished

Destroyed

---

### GameInstance

游戏运行实例。

instance_id

room_id

pid

port

status

start_time

Room 与 GameInstance 分离。

原因：

一个房间可能经历：

创建房间

↓

启动游戏

↓

游戏结束

↓

重新开始

---

## 5. 游戏启动流程

用户登录

↓

创建房间

↓

玩家加入

↓

满足开始条件

↓

Lobby创建GameInstance

↓

启动Game进程

↓

Game初始化

↓

Game Ready

↓

玩家连接Game

↓

开始游戏

---

## 6. 通信方式

### Client -> Lobby

HTTP

用于：

- 登录
- 创建房间
- 加入房间
- 查询房间

---

### Client -> Game

WebSocket

用于：

- 游戏消息
- 状态同步
- 操作请求

---

### Lobby -> Game

进程通信。

使用：

- stdin/stdout
- pipe

用于：

- 初始化
- 生命周期通知

---

## 7. 认证模型

流程：

用户登录

↓

Lobby验证账号

↓

生成Session Token

↓

启动Game时传入

↓

Game验证Session

Game不保存用户密码。

Game不依赖JWT。

---

## 8. 断线重连

原则：

Player != Connection

玩家：

uid

state

连接：

socket

断线：

socket关闭

Player保留

重连：

客户端携带Session

↓

Game查找Player

↓

绑定新连接

↓

发送状态快照

---

## 9. V1 不实现

以下功能不属于第一版本：

- 分布式部署
- Gateway
- 插件系统
- Docker调度
- 匹配系统
- 好友系统
- 排行榜
- 游戏录像
- 多服务器同步
