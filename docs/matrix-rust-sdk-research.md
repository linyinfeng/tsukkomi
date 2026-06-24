# matrix-rust-sdk 调研 — tsukkomi Matrix Bot

> 基于 matrix-sdk 0.18，ruma 0.16，Matrix Client-Server API v1.12+

## 1. 概述

[matrix-rust-sdk](https://github.com/matrix-org/matrix-rust-sdk) 是 Matrix 官方维护的 Rust SDK，提供了完整的 Client-Server API 实现。与 teloxide 不同，Matrix SDK 不提供"bot 框架"的概念，而是提供底层客户端，开发者需自行构建事件驱动的循环。

### 核心概念

| 组件 | 作用 |
|------|------|
| `Client` | 核心客户端（内部 Arc，可 Clone） |
| `ClientBuilder` | 构建 Client（配置 store、homeserver 等） |
| `Room` | 房间抽象（Joined / Invited / Left） |
| `SyncRoomMessageEvent` | 同步到的房间消息事件 |
| `ruma` | Matrix 协议类型库（事件类型、标识符等） |
| `StateStore` | 状态持久化（SQLite 内置支持） |
| `SlidingSync` | 增量同步（MSC3575，新一代同步方式） |

### 与 teloxide 的关键区别

| 方面 | teloxide | matrix-rust-sdk |
|------|----------|-----------------|
| 抽象层次 | 高（bot 框架） | 中（客户端 SDK） |
| 回调模式 | `Dispatcher` / `REPL` | `add_event_handler` + `sync` |
| 消息接收 | 长轮询 / Webhook | 持久连接 Sync |
| 认证 | Bot Token | 用户名密码 / Access Token / Appservice |
| 状态管理 | Dialogue（可选） | StateStore（必须） |
| 端到端加密 | 不涉及 | 内置支持（`e2e-encryption` feature） |

## 2. 基础 Bot 搭建

### 2.1 Cargo.toml 依赖

```toml
[dependencies]
matrix-sdk = { version = "*", features = ["sqlite"] }
tokio = { version = "*", features = ["macros", "rt-multi-thread"] }
tracing = "*"
tracing-subscriber = "*"
anyhow = "*"
```

当前项目已配置 `matrix-sdk = "*"` 在 workspace 依赖中。

### 2.2 最小 Bot 示例

```rust
use matrix_sdk::{
    Client, config::SyncSettings,
    ruma::events::room::message::SyncRoomMessageEvent,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // 1. 创建 Client
    let client = Client::builder()
        .server_name("matrix.example.org")  // 或 .homeserver_url(...)
        .sqlite_store("./store", None)       // 持久化存储
        .build()
        .await?;

    // 2. 登录
    client.matrix_auth()
        .login_username("@bot_user:example.org", "password")
        .send()
        .await?;

    tracing::info!("已登录为: {}", client.user_id().unwrap());

    // 3. 注册事件处理器
    client.add_event_handler(|ev: SyncRoomMessageEvent, room: Room, client: Client| async move {
        tracing::info!("房间 {} 收到消息: {:?}", room.room_id(), ev);
    });

    // 4. 开始同步（永不返回，除非出错）
    client.sync(SyncSettings::default()).await?;

    Ok(())
}
```

### 2.3 使用 Session Token 恢复登录（推荐）

```rust
// 首次登录后保存 session
let session = client.session().unwrap();
// 持久化 session（文件/数据库）

// 后续启动时恢复
let client = Client::builder()
    .server_name("matrix.example.org")
    .sqlite_store("./store", None)
    .build()
    .await?;

client.restore_session(session).await?;
```

## 3. 消息监听与处理（核心场景）

### 3.1 事件处理器签名

```rust
// 基本形式：事件类型 + 上下文参数
client.add_event_handler(|ev: SyncRoomMessageEvent| async move { });

// 常用：事件 + 房间 + 客户端
client.add_event_handler(|ev: SyncRoomMessageEvent, room: Room, client: Client| async move { });

// 额外上下文：加密信息
client.add_event_handler(
    |ev: SyncRoomMessageEvent, room: Room, encryption_info: Option<EncryptionInfo>| async move { }
);

// 额外上下文：push actions
client.add_event_handler(
    |ev: SyncRoomMessageEvent, room: Room, push_actions: Vec<Action>| async move { }
);
```

### 3.2 过滤消息

Matrix 没有 Telegram 那样的"群组/私聊"消息分类，所有消息都是 Room 中的事件。需要手动判断房间类型：

```rust
use matrix_sdk::ruma::events::room::message::{
    SyncRoomMessageEvent, RoomMessageEventContent,
};
use matrix_sdk::ruma::events::AnySyncMessageLikeEvent;

client.add_event_handler(
    |ev: SyncRoomMessageEvent, room: Room| async move {
        // 判断房间类型（类似群组）
        if room.is_dm() {
            // 私聊消息
        } else {
            // 群组消息
        }

        // 获取消息文本
        if let AnySyncMessageLikeEvent::Original(original) = ev.original_content() {
            let body = &original.content.body;
            let msgtype = &original.content.msgtype;
            // body: 消息文本
            // msgtype: m.text / m.notice / m.emote 等
        }
    },
);
```

> 注意：matrix-sdk 0.18 的 `SyncRoomMessageEvent` 可以通过 `ev.original_content()` 获取原始内容，返回 `Option`。

### 3.3 仅处理文本消息

```rust
use matrix_sdk::ruma::events::room::message::{
    SyncRoomMessageEvent, MessageType,
};

client.add_event_handler(
    |ev: SyncRoomMessageEvent, room: Room| async move {
        let content = match ev.original_content() {
            Some(AnySyncMessageLikeEvent::Original(c)) => c,
            _ => return,  // 跳过编辑消息
        };

        // 检查消息类型
        let text = match &content.content.msgtype {
            MessageType::Text(text_content) => text_content.body.clone(),
            _ => return,  // 非文本消息跳过
        };

        tracing::info!("文本消息: {text}");
    },
);
```

### 3.4 特定的房间监听

```rust
// 使用 Client::add_room_event_handler
client.add_room_event_handler(
    room_id!("!roomid:example.org"),
    |ev: SyncRoomMessageEvent, room: Room| async move {
        // 仅在该房间中触发
    },
);

// 或使用 Room::add_event_handler
let room = client.get_room(&room_id).unwrap();
room.add_event_handler(|ev: SyncRoomMessageEvent| async move {
    // 仅在该房间中触发
});
```

## 4. 发送消息

### 4.1 基础文本消息

```rust
use matrix_sdk::ruma::events::room::message::{
    RoomMessageEventContent, MessageType, TextMessageEventContent,
};

// 发送到 Room
let content = RoomMessageEventContent::text_plain("Hello from tsukkomi!");
room.send(content).await?;

// 或使用 builder 模式
let content = RoomMessageEventContent::new(MessageType::Text(
    TextMessageEventContent::new("Hello".into()),
));
```

### 4.2 回复消息

```rust
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;

// 创建回复事件
let reply_content = room.make_reply_event(
    RoomMessageEventContent::text_plain("这是回复"),
    reply::Reply::new(&original_event),  // 需要原始事件的指针
).await?;

room.send(reply_content).await?;
```

### 4.3 发送 Markdown（需要 `markdown` feature）

```toml
matrix-sdk = { version = "*", features = ["sqlite", "markdown"] }
```

```rust
// 如果启用 markdown feature
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;

let content = RoomMessageEventContent::text_markdown("**粗体** *斜体*");
room.send(content).await?;
```

### 4.4 获取房间引用

```rust
// 方式1：从事件处理器中获取
client.add_event_handler(|ev: SyncRoomMessageEvent, room: Room| async move {
    room.send(content).await?;
});

// 方式2：通过 room_id 获取
let room = client.get_room(&room_id!("!roomid:example.org")).unwrap();

// 方式3：遍历所有已加入房间
for room in client.joined_rooms() {
    // ...
}
```

## 5. 同步策略

### 5.1 经典 Sync（推荐用于 Bot）

```rust
// 简单同步（永不返回）
client.sync(SyncSettings::default()).await?;

// 自定义同步
let settings = SyncSettings::default()
    .timeout(Duration::from_secs(30))  // 长轮询超时
    .token("previous_batch_token");    // 从指定 token 开始

client.sync_with_callback(settings, |response| async move {
    // 每次 sync 响应后的回调
    tracing::info!("收到 sync 响应");
}).await?;
```

### 5.2 Sliding Sync（实验性，需要服务器支持）

```rust
// Sliding Sync 是一种增量同步协议（MSC3575）
// 需要 homeserver 支持（如 Synapse 1.100+、Conduit）
// 适用于需要按列表筛选房间的场景

use matrix_sdk::sliding_sync::SlidingSyncMode;

let sliding_sync = client.sliding_sync("tsukkomi")?
    .add_list(
        SlidingSyncList::builder("all_rooms")
            .sync_mode(SlidingSyncMode::new_selective()
                .add_range(0, 100)),
    )
    .build()
    .await?;
```

**建议**：Bot 场景使用经典 Sync 更简单可靠，Sliding Sync 适合移动端或需要精细控制同步列表的场景。

## 6. 自动接受邀请

```rust
use matrix_sdk::ruma::events::room::member::StrippedRoomMemberEvent;

client.add_event_handler(|ev: StrippedRoomMemberEvent, room: Room| async move {
    // 处理房间邀请
    if room.state() == RoomState::Invited {
        tracing::info!("被邀请到房间: {}", room.room_id());
        room.join().await?;
        tracing::info!("已加入房间");
    }
});
```

## 7. 状态持久化

### 7.1 SQLite 存储

```rust
let client = Client::builder()
    .server_name("example.org")
    .sqlite_store("./store", None)  // 数据存储在 ./store/
    .build()
    .await?;
```

SQLite 会存储：
- 房间状态（名称、主题、成员等）
- 同步 token（保证断线重连后不丢消息）
- E2EE 密钥（如果启用）

### 7.2 内存存储（测试用）

```rust
let client = Client::builder()
    .server_name("example.org")
    .build()
    .await?;  // 默认 MemoryStore
```

**Bot 必须使用 SQLite 或类似持久化存储**，否则每次重启都会重新同步。

## 8. 端到端加密（E2EE）

`e2e-encryption` 默认启用。对于 Bot 场景：

```rust
let client = Client::builder()
    .server_name("example.org")
    .sqlite_store("./store", Some("passphrase"))  // 加密存储
    .build()
    .await?;

// 检查房间加密状态
let encryption_state = room.encryption_state();
match encryption_state {
    EncryptionState::Encrypted => {
        // 需要确保 Bot 已验证设备
        // 注意：Bot 在加密房间中默认可以解密消息
        // 但在某些服务器配置下需要手动验证
    }
    EncryptionState::NotEncrypted => {
        // 明文消息，可直接处理
    }
    EncryptionState::Unknown => {
        room.request_encryption_state().await?;
    }
}
```

**注意**：如果 Bot 加入的 Matrix 群组启用了 E2EE，必须启用 `e2e-encryption` feature 才能读取消息内容。

## 9. 架构建议（面向 tsukkomi）

### 目录结构

```
crates/tsukkomi-matrix/
├── Cargo.toml
└── src/
    ├── main.rs              # 入口：初始化 Client，登录，注册 handler，开始 sync
    ├── config.rs            # 配置加载（homeserver, credentials, LLM 配置）
    ├── handlers/
    │   ├── mod.rs
    │   ├── message.rs       # 消息事件处理
    │   └── invite.rs        # 邀请处理（自动加入房间）
    ├── llm.rs               # LLM 调用封装
    └── utils.rs             # 工具函数
```

### 核心流程

```
Matrix Sync Loop
  → Client 收到 sync 响应
    → Event Handler Dispatch
      → SyncRoomMessageEvent handler
        1. 跳过自己的消息（比较 sender）
        2. 解析消息文本
        3. 检查触发条件（提及 bot / 关键词 / 概率）
        4. 调用 llm 生成吐槽
        5. room.send() 发送回复
```

### 跳过自己的消息

```rust
client.add_event_handler(
    |ev: SyncRoomMessageEvent, room: Room, client: Client| async move {
        // 跳过 Bot 自己的消息
        if let Some(sender) = ev.sender() {
            if sender == client.user_id().unwrap() {
                return;
            }
        }
        // 处理消息...
    },
);
```

## 10. 配置管理

Matrix Bot 需要的配置项：

| 配置 | 说明 | 来源 |
|------|------|------|
| Homeserver URL | Matrix 服务器地址 | 环境变量 / 配置文件 |
| 用户名 | Bot 账号 MXID | 环境变量 / 配置文件 |
| 密码 / Token | 认证凭据 | 环境变量（不要硬编码） |
| Store 路径 | SQLite 存储路径 | 配置文件 |

推荐使用环境变量（与现有项目风格一致）：

```rust
let homeserver = std::env::var("MATRIX_HOMESERVER")
    .expect("MATRIX_HOMESERVER 未设置");
let username = std::env::var("MATRIX_USERNAME")
    .expect("MATRIX_USERNAME 未设置");
let password = std::env::var("MATRIX_PASSWORD")
    .expect("MATRIX_PASSWORD 未设置");
```

## 11. 注意事项与常见坑

1. **Sync 永不返回**：`client.sync()` 会一直运行直到出错或取消。需要在 tokio task 中运行。
2. **StateStore 的重要性**：没有持久化 store，每次重启会全量同步，可能丢失消息。
3. **速率限制**：Matrix homeserver 一般有速率限制，发送大量消息可能被暂时封禁。
4. **加密房间的兼容性**：如果启用了 E2EE，Bot 需要处理设备验证。某些老旧服务器可能不支持。
5. **消息格式**：Matrix 的消息格式不同于 Telegram。文本是 `body` + `formatted_body` （用于 HTML/Markdown）。
6. **Bot 账号 vs Appservice**：简单 Bot 用普通账号即可。Appservice 适合管理大量虚拟用户的场景。
7. **断线重连**：SDK 内部处理自动重连；Sync token 在 StateStore 中持久化，重启后不会丢消息。
8. **ruma 宏**：使用 `room_id!()`, `user_id!()` 等宏可以编译时验证 Matrix ID 格式。

## 12. 关键文档参考

- [matrix-sdk crate docs](https://docs.rs/matrix-sdk/latest/matrix_sdk/)
- [matrix-rust-sdk GitHub (examples)](https://github.com/matrix-org/matrix-rust-sdk/tree/main/examples)
- [ruma crate docs](https://docs.rs/ruma/latest/ruma/)
- [Matrix Client-Server API 规范](https://spec.matrix.org/latest/client-server-api/)
